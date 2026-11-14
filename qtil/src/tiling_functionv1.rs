use std::collections::HashMap;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const MAX_WINDOWS: usize = 10;

pub struct TabState {
    pub windows: Vec<HWND>,
    pub original_positions: HashMap<isize, RECT>,
    pub active_tab: usize,
    pub is_tabbed: bool,
}

impl TabState {
    pub fn new() -> Self {
        Self {
            windows: Vec::new(),
            original_positions: HashMap::new(),
            active_tab: 0,
            is_tabbed: false,
        }
    }
}

pub fn register_hotkeys(hwnd: HWND) {
    unsafe {
        let _ = RegisterHotKey(Some(hwnd), 100, MOD_ALT | MOD_NOREPEAT, 'R' as u32);
        let _ = RegisterHotKey(Some(hwnd), 101, MOD_ALT | MOD_NOREPEAT, VK_TAB.0 as u32);
        let _ = RegisterHotKey(Some(hwnd), 102, MOD_ALT | MOD_SHIFT | MOD_NOREPEAT, VK_TAB.0 as u32);
    }
}

pub fn unregister_hotkeys(hwnd: HWND) {
    unsafe {
        let _ = UnregisterHotKey(Some(hwnd), 100);
        let _ = UnregisterHotKey(Some(hwnd), 101);
        let _ = UnregisterHotKey(Some(hwnd), 102);
    }
}

fn get_all_windows() -> Vec<HWND> {
    let mut result = Vec::new();
    unsafe {
        let mut hwnd = match GetTopWindow(None) {
            Ok(h) => h,
            Err(_) => return result,
        };
        loop {
            if IsWindowVisible(hwnd).as_bool() && GetWindowTextLengthW(hwnd) > 0 {
                let mut pid = 0;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                if pid != std::process::id() {
                    let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
                    if ex & WS_EX_TOOLWINDOW.0 == 0 {
                        let mut r = RECT::default();
                        let _ = GetWindowRect(hwnd, &mut r);
                        if (r.right - r.left) > 0 && (r.bottom - r.top) > 0 {
                            result.push(hwnd);
                        }
                    }
                }
            }
            match GetWindow(hwnd, GW_HWNDNEXT) {
                Ok(next) => hwnd = next,
                Err(_) => break,
            }
        }
    }
    result
}

pub fn toggle_tabs(state: &mut TabState) {
    unsafe {
        if state.is_tabbed {
            for &hwnd in &state.windows {
                if let Some(&rect) = state.original_positions.get(&(hwnd.0 as isize)) {
                    let _ = SetWindowPos(hwnd, None, rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top, SWP_NOZORDER | SWP_NOACTIVATE);
                }
            }
            state.windows.clear();
            state.original_positions.clear();
            state.is_tabbed = false;
            state.active_tab = 0;
            println!("[Вкладки] Разобрано.");
        } else {
            let wins = get_all_windows();
            if wins.is_empty() { return; }
            
            state.windows.clear();
            state.original_positions.clear();
            
            for &hwnd in &wins {
                let mut wr = RECT::default();
                let _ = GetWindowRect(hwnd, &mut wr);
                state.original_positions.insert(hwnd.0 as isize, wr);
                state.windows.push(hwnd);
                if state.windows.len() >= MAX_WINDOWS { break; }
            }
            
            if state.windows.is_empty() { return; }
            state.is_tabbed = true;
            state.active_tab = 0;
            
            apply_tabs_layout(state);
            println!("[Вкладки] Собрано {} окон.", state.windows.len());
        }
    }
}

pub fn switch_tab(state: &mut TabState, forward: bool) {
    if !state.is_tabbed || state.windows.len() <= 1 { return; }
    
    if forward {
        state.active_tab = (state.active_tab + 1) % state.windows.len();
    } else {
        if state.active_tab == 0 {
            state.active_tab = state.windows.len() - 1;
        } else {
            state.active_tab -= 1;
        }
    }
    apply_tabs_layout(state);
}

fn apply_tabs_layout(state: &TabState) {
    unsafe {
        let mut wa = RECT::default();
        let _ = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut wa as *mut _ as *mut _), SPIF_UPDATEINIFILE);
        let sw = wa.right - wa.left;
        let sh = wa.bottom - wa.top;
        let tab_width = 200;
        let main_width = sw - tab_width;

        for (i, &hwnd) in state.windows.iter().enumerate() {
            if i == state.active_tab {
                let _ = SetWindowPos(hwnd, None, wa.left, wa.top, main_width, sh, SWP_NOZORDER | SWP_NOACTIVATE);
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = SetForegroundWindow(hwnd);
            } else {
                let offset = i as i32 * 40;
                let _ = SetWindowPos(hwnd, None, wa.left + main_width, wa.top + offset, tab_width, sh - offset, SWP_NOZORDER | SWP_NOACTIVATE);
                let _ = ShowWindow(hwnd, SW_SHOW);
            }
        }
    }
}