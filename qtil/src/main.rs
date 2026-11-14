#![allow(unsafe_op_in_unsafe_fn, dead_code, static_mut_refs)]
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::os::windows::ffi::OsStringExt;
use std::result::Result::Ok;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, RECT, POINT, WPARAM, LPARAM, LRESULT, HINSTANCE};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Graphics::GdiPlus::*;
use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::core::{w, PCWSTR, BOOL};

mod config_menu;
mod holst;
mod world3d;
use config_menu::SettingsState;

// ---------------------------------------------------------------- тайминги
const FRAME_MS: u64 = 20;
const PANEL_T: f32 = 1.0;
const CARD_START: f32 = 0.52;
const LAND_T: f32 = 1.05;
const CLOSE_T: f32 = 0.70;
const HEADER: f32 = 28.0;
const TAU: f32 = std::f32::consts::PI * 2.0;
const PI: f32 = std::f32::consts::PI;

// ---------------------------------------------------------------- хелперы
fn argb(a: u8, r: u8, g: u8, b: u8) -> u32 {
    ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}
fn lerp(a: f32, b: f32, t: f32) -> f32 { a + (b - a) * t }
fn smooth(t: f32) -> f32 { t * t * (3.0 - 2.0 * t) }
fn smoother(t: f32) -> f32 { t * t * t * (t * (t * 6.0 - 15.0) + 10.0) }
fn ease_out_back(t: f32) -> f32 {
    let c1 = 1.70158f32;
    let c3 = c1 + 1.0;
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}
fn pulse_env(age: f32) -> f32 {
    if age < 0.0 { 0.0 } else { (-age * 5.5).exp() * (age * 22.0).sin() }
}
fn snake_color(u: f32, a: (u8, u8, u8), b: (u8, u8, u8)) -> (u8, u8, u8) {
    let u = u.clamp(0.0, 1.0);
    (lerp(a.0 as f32, b.0 as f32, u) as u8,
     lerp(a.1 as f32, b.1 as f32, u) as u8,
     lerp(a.2 as f32, b.2 as f32, u) as u8)
}
fn palette_color(alpha: u8, c: (u8, u8, u8)) -> u32 { argb(alpha, c.0, c.1, c.2) }
fn pingpong(v: f32) -> f32 {
    let v = v.rem_euclid(2.0);
    if v > 1.0 { 2.0 - v } else { v }
}
fn wave(x: f32, t: f32, seed: f32) -> f32 {
    (x * 0.0105 + t * 2.1 + seed).sin() * 16.0
    + (x * 0.0261 - t * 3.0 + seed * 1.7).sin() * 9.0
    + (x * 0.0047 + t * 1.2 + seed * 0.6).sin() * 20.0
}
unsafe fn rounded_path(l: f32, t: f32, w: f32, h: f32, r: f32) -> *mut GpPath {
    let mut p: *mut GpPath = std::ptr::null_mut();
    let _ = GdipCreatePath(FillModeAlternate, &mut p);
    let d = r * 2.0;
    let _ = GdipAddPathArc(p, l, t, d, d, 180.0, 90.0);
    let _ = GdipAddPathArc(p, l + w - d, t, d, d, 270.0, 90.0);
    let _ = GdipAddPathArc(p, l + w - d, t + h - d, d, d, 0.0, 90.0);
    let _ = GdipAddPathArc(p, l, t + h - d, d, d, 90.0, 90.0);
    let _ = GdipClosePathFigure(p);
    p
}
unsafe fn capture_window(hwnd: HWND, r: &RECT) -> *mut GpBitmap {
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    if w <= 0 || h <= 0 { return std::ptr::null_mut(); }
    let sdc = GetDC(None);
    let dc = CreateCompatibleDC(Some(sdc));
    let bmp = CreateCompatibleBitmap(sdc, w, h);
    let old = SelectObject(dc, bmp.into());
    let _ = PrintWindow(hwnd, dc, PRINT_WINDOW_FLAGS(2));
    SelectObject(dc, old);
    let mut big: *mut GpBitmap = std::ptr::null_mut();
    let _ = GdipCreateBitmapFromHBITMAP(bmp, HPALETTE(std::ptr::null_mut()), &mut big);
    let tw = w.min(640).max(1);
    let th = ((h as f32 * tw as f32 / w as f32) as i32).max(1);
    let mut thumb: *mut GpBitmap = std::ptr::null_mut();
    let _ = GdipCreateBitmapFromScan0(tw, th, 0, 925707, None, &mut thumb);
    if !thumb.is_null() && !big.is_null() {
        let mut tg: *mut GpGraphics = std::ptr::null_mut();
        let _ = GdipGetImageGraphicsContext(thumb as *mut GpImage, &mut tg);
        let _ = GdipSetInterpolationMode(tg, InterpolationModeHighQualityBicubic);
        let _ = GdipDrawImageRectI(tg, big as *mut GpImage, 0, 0, tw, th);
        let _ = GdipDeleteGraphics(tg);
    }
    if !big.is_null() { let _ = GdipDisposeImage(big as *mut GpImage); }
    let _ = DeleteDC(dc);
    let _ = DeleteObject(bmp.into());
    ReleaseDC(None, sdc);
    thumb
}
#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "OpenProcess"]
    fn open_process(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
    #[link_name = "QueryFullProcessImageNameW"]
    fn query_image(process: *mut std::ffi::c_void, flags: u32, path: *mut u16, size: *mut u32) -> i32;
    #[link_name = "CloseHandle"]
    fn close_handle(handle: *mut std::ffi::c_void) -> i32;
}

#[link(name = "user32")]
unsafe extern "system" {
    #[link_name = "IsWindow"]
    fn window_exists(hwnd: *mut std::ffi::c_void) -> i32;
    #[link_name = "PostMessageW"]
    fn post_window_message(hwnd: *mut std::ffi::c_void, message: u32, wparam: usize, lparam: isize) -> i32;
}

fn process_executable(pid: u32) -> std::io::Result<PathBuf> {
    unsafe {
        let handle = open_process(0x1000, 0, pid);
        if handle.is_null() { return Err(std::io::Error::last_os_error()); }
        let mut buf = vec![0u16; 32768];
        let mut len = buf.len() as u32;
        let ok = query_image(handle, 0, buf.as_mut_ptr(), &mut len);
        let error = if ok == 0 { Some(std::io::Error::last_os_error()) } else { None };
        let _ = close_handle(handle);
        if let Some(e) = error { return Err(e); }
        buf.truncate(len as usize);
        Ok(PathBuf::from(std::ffi::OsString::from_wide(&buf)))
    }
}

fn notice(message: &str) {
    let text: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe { let _ = MessageBoxW(None, PCWSTR(text.as_ptr()), w!("Q-Tiler"), MB_OK | MB_ICONWARNING); }
}

struct TileMove { hwnd: HWND, start: RECT, end: RECT }
struct TileTransition { moves: Vec<TileMove>, started: Instant, duration: Duration }
struct PendingDuplicate { initial_windows: HashSet<isize>, old_pid: u32, child_pid: u32, started: Instant }
struct PendingClose { hwnd: HWND, pid: u32, same_mode: bool, started: Instant }

struct Card { x: f32, y: f32, w: f32, h: f32, depth: f32, p: f32 }

// ---------------------------------------------------------------- состояние
#[derive(Clone)]
struct PreviewWindow {
    title: String,
    bmp: *mut GpBitmap,
    aspect: f32,
}

struct State {
    original_positions: HashMap<isize, RECT>,
    transition: Option<TileTransition>,
    pending_duplicate: Option<PendingDuplicate>,
    pending_close: Option<PendingClose>,
    same_windows: Vec<HWND>,
    same_originals: HashMap<isize, RECT>,
    same_mode: bool,
    active_windows: Vec<HWND>,
    is_tiled: bool,
    mirrored: bool,
    is_selecting: bool,
    selected_index: usize,
    w_released: bool,
    spin_flag: bool,
    spin_angle: f32,
    preview_data: Vec<PreviewWindow>,
    overlay_hwnd: HWND,
    settings_hwnd: HWND,
    settings_state: *mut SettingsState,
    screen_w: i32,
    screen_h: i32,
    gdiplus_token: usize,
    anim_start: Instant,
    last_tick: Instant,
    closing: Option<Instant>,
    t: f32,
    ct: f32,
    pick: Option<usize>,
    hover: Option<usize>,
    spin_speed: f32,
    overlay_duration: f32,
    overlay_colors: ((u8, u8, u8), (u8, u8, u8)),
    cache_brush_bg: *mut GpSolidFill,
    cache_brush_dim: *mut GpSolidFill,
    cache_brush_ribbon: *mut GpLineGradient,
    cache_pen_glow: *mut GpPen,
    cache_pen_mid: *mut GpPen,
    cache_pen_thin: *mut GpPen,
    cache_brush_center: *mut GpSolidFill,
    cache_pen_center: *mut GpPen,
    cache_brush_card: *mut GpSolidFill,
    cache_pen_border: *mut GpPen,
    cache_brush_text: *mut GpSolidFill,
    cache_font_title: *mut GpFont,
    cache_format_center: *mut GpStringFormat,
    cache_brush_red: *mut GpSolidFill,
    cache_brush_yellow: *mut GpSolidFill,
    cache_brush_green: *mut GpSolidFill,
    mem_dc: HDC,
    mem_bmp: HBITMAP,
    shot_dc: HDC,
    shot_bmp: HBITMAP,
    bg_dc: HDC,
    bg_bmp: HBITMAP,
}

impl State {
    fn new() -> Self {
        Self {
            original_positions: HashMap::new(),
            transition: None,
            pending_duplicate: None,
            pending_close: None,
            same_windows: Vec::new(),
            same_originals: HashMap::new(),
            same_mode: false,
            active_windows: Vec::new(),
            is_tiled: false,
            mirrored: false,
            is_selecting: false,
            selected_index: 0,
            w_released: false,
            spin_flag: false,
            spin_angle: 0.0,
            preview_data: Vec::new(),
            overlay_hwnd: HWND(std::ptr::null_mut()),
            settings_hwnd: HWND(std::ptr::null_mut()),
            settings_state: std::ptr::null_mut(),
            screen_w: 1920,
            screen_h: 1080,
            gdiplus_token: 0,
            anim_start: Instant::now(),
            last_tick: Instant::now(),
            closing: None,
            t: 0.0,
            ct: 0.0,
            pick: None,
            hover: None,
            spin_speed: 0.25,
            overlay_duration: 1.0,
            overlay_colors: (config_menu::COLORS[0], config_menu::COLORS[1]),
            cache_brush_bg: std::ptr::null_mut(),
            cache_brush_dim: std::ptr::null_mut(),
            cache_brush_ribbon: std::ptr::null_mut(),
            cache_pen_glow: std::ptr::null_mut(),
            cache_pen_mid: std::ptr::null_mut(),
            cache_pen_thin: std::ptr::null_mut(),
            cache_brush_center: std::ptr::null_mut(),
            cache_pen_center: std::ptr::null_mut(),
            cache_brush_card: std::ptr::null_mut(),
            cache_pen_border: std::ptr::null_mut(),
            cache_brush_text: std::ptr::null_mut(),
            cache_font_title: std::ptr::null_mut(),
            cache_format_center: std::ptr::null_mut(),
            cache_brush_red: std::ptr::null_mut(),
            cache_brush_yellow: std::ptr::null_mut(),
            cache_brush_green: std::ptr::null_mut(),
            mem_dc: HDC(std::ptr::null_mut()),
            mem_bmp: HBITMAP(std::ptr::null_mut()),
            shot_dc: HDC(std::ptr::null_mut()),
            shot_bmp: HBITMAP(std::ptr::null_mut()),
            bg_dc: HDC(std::ptr::null_mut()),
            bg_bmp: HBITMAP(std::ptr::null_mut()),
        }
    }

    unsafe fn is_taskable(&self, hwnd: HWND) -> bool {
        if hwnd == self.overlay_hwnd || hwnd == self.settings_hwnd
            || !IsWindowVisible(hwnd).as_bool()
            || GetWindowTextLengthW(hwnd) == 0
        {
            return false;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == std::process::id() { return false; }
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        let is_app = ex & WS_EX_APPWINDOW.0 != 0;
        if ex & WS_EX_TOOLWINDOW.0 != 0 && !is_app { return false; }
        if let Ok(owner) = GetWindow(hwnd, GW_OWNER) {
            if !owner.0.is_null() && !is_app { return false; }
        }
        let mut cloaked: u32 = 0;
        let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, std::mem::size_of::<u32>() as u32);
        if cloaked != 0 { return false; }
        let mut cls = [0u16; 64];
        let n = GetClassNameW(hwnd, &mut cls).max(0) as usize;
        let name = String::from_utf16_lossy(&cls[..n.min(64)]);
        if name == "Progman" || name == "WorkerW" || name == "Shell_TrayWnd" { return false; }
        let mut r = RECT::default();
        let _ = GetWindowRect(hwnd, &mut r);
        (r.right - r.left) > 0 && (r.bottom - r.top) > 0
    }

    fn get_windows(&self) -> Vec<HWND> {
        let mut result = Vec::new();
        unsafe {
            let mut hwnd = match GetTopWindow(None) {
                Ok(h) => h,
                Err(_) => return result,
            };
            loop {
                if self.is_taskable(hwnd) { result.push(hwnd); }
                match GetWindow(hwnd, GW_HWNDNEXT) {
                    Ok(next) => hwnd = next,
                    Err(_) => break,
                }
            }
        }
        result
    }

    fn tile_duration(&self) -> Duration {
        let ms = if self.settings_state.is_null() { 192 } else {
            unsafe { (*self.settings_state).tile_duration_ms() }
        };
        Duration::from_millis(ms as u64)
    }

    fn finish_tiling_transition(&mut self) {
        if let Some(transition) = self.transition.take() {
            for item in transition.moves {
                unsafe {
                    let r = item.end;
                    let _ = SetWindowPos(item.hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top,
                        SWP_NOZORDER | SWP_NOACTIVATE);
                }
            }
        }
    }

    fn animate_positions(&mut self, items: &[(HWND, i32, i32, i32, i32, bool)]) {
        if let Some(previous) = self.transition.take() {
            for item in previous.moves {
                if !items.iter().any(|(hwnd, ..)| *hwnd == item.hwnd) {
                    let r = item.end;
                    unsafe { let _ = SetWindowPos(item.hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE); }
                }
            }
        }
        let mut moves = Vec::new();
        for &(hwnd, tx, ty, tw, th, adjust) in items {
            unsafe {
                if IsIconic(hwnd).as_bool() { let _ = ShowWindow(hwnd, SW_RESTORE); }
                let mut start = RECT::default();
                if GetWindowRect(hwnd, &mut start).is_err() { continue; }
                let (mut x, mut y, mut w, mut h) = (tx, ty, tw, th);
                if adjust {
                    let mut eb = RECT::default();
                    let ok = DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS,
                        &mut eb as *mut _ as *mut _, std::mem::size_of::<RECT>() as u32).is_ok();
                    if ok {
                        x -= eb.left - start.left;
                        y -= eb.top - start.top;
                        w += (start.right - start.left) - (eb.right - eb.left);
                        h += (start.bottom - start.top) - (eb.bottom - eb.top);
                    }
                }
                moves.push(TileMove { hwnd, start, end: RECT { left: x, top: y, right: x + w, bottom: y + h } });
            }
        }
        if !moves.is_empty() {
            self.transition = Some(TileTransition { moves, started: Instant::now(), duration: self.tile_duration() });
        }
    }

    fn step_tiling_transition(&mut self) {
        let Some(t) = self.transition.as_ref() else { return; };
        let elapsed = t.started.elapsed().as_secs_f32();
        let progress = (elapsed / t.duration.as_secs_f32()).clamp(0.0, 1.0);
        let e = smooth(progress);
        for item in &t.moves {
            let a = item.start;
            let b = item.end;
            let interp = |x: i32, y: i32| (x as f32 + (y - x) as f32 * e).round() as i32;
            let (l, top, r, bot) = (interp(a.left, b.left), interp(a.top, b.top), interp(a.right, b.right), interp(a.bottom, b.bottom));
            unsafe { let _ = SetWindowPos(item.hwnd, None, l, top, (r-l).max(1), (bot-top).max(1), SWP_NOZORDER | SWP_NOACTIVATE); }
        }
        if progress >= 1.0 { self.transition = None; }
    }

    fn compute_rects(count: usize, wa: &RECT, gap: i32, mirrored: bool) -> Vec<(i32, i32, i32, i32)> {
        let w = wa.right - wa.left;
        let h = wa.bottom - wa.top;
        let (l, t) = (wa.left, wa.top);
        let col = (w - gap * 3) / 2;
        let row = (h - gap * 3) / 2;
        let right_x = l + col + gap * 2;
        let bottom_y = t + row + gap * 2;
        let mut rects: Vec<(i32, i32, i32, i32)> = match count {
            0 | 1 => vec![(l, t, w, h)],
            2 => vec![
                (l + gap, t + gap, col, h - gap * 2),
                (right_x, t + gap, col, h - gap * 2),
            ],
            3 => vec![
                (l + gap, t + gap, col, h - gap * 2),
                (right_x, t + gap, col, row),
                (right_x, bottom_y, col, row),
            ],
            4 => vec![
                (l + gap, t + gap, col, row),
                (right_x, t + gap, col, row),
                (l + gap, bottom_y, col, row),
                (right_x, bottom_y, col, row),
            ],
            _ => {
                let half = (col - gap) / 2;
                vec![
                    (l + gap, t + gap, col, row),
                    (right_x, t + gap, col, row),
                    (l + gap, bottom_y, col, row),
                    (right_x, bottom_y, half, row),
                    (right_x + half + gap, bottom_y, half, row),
                ]
            }
        };
        if mirrored && count > 1 {
            for r in rects.iter_mut() {
                r.0 = wa.left + wa.right - r.0 - r.2;
            }
        }
        rects
    }

    fn layout_targets(&self) -> Vec<(HWND, i32, i32, i32, i32, bool)> {
        if self.active_windows.is_empty() { return Vec::new(); }
        let mut wa = RECT::default();
        unsafe { let _ = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut wa as *mut _ as *mut _), SPIF_UPDATEINIFILE); }
        let rects = Self::compute_rects(self.active_windows.len(), &wa, 10, self.mirrored);
        self.active_windows.iter().zip(rects.iter())
            .map(|(&hwnd, &(x, y, w, h))| (hwnd, x, y, w, h, true)).collect()
    }

    fn apply_layout(&mut self) {
        let targets = self.layout_targets();
        self.animate_positions(&targets);
    }

    fn toggle_mirror(&mut self) {
        if !self.is_tiled || self.active_windows.is_empty() { return; }
        self.mirrored = !self.mirrored;
        self.apply_layout();
        println!("[Тайлинг] Зеркало: {}", if self.mirrored { "вкл" } else { "выкл" });
    }

    fn toggle(&mut self) {
        if self.pending_close.is_some() { notice("Дождитесь закрытия окна."); return; }
        if self.same_mode { self.leave_same_mode(); }
        else if self.is_tiled { self.untile(); } else { self.tile(); }
    }

    fn tile(&mut self) {
        let windows = self.get_windows();
        if windows.is_empty() { return; }
        if !self.active_windows.is_empty() { self.untile(); }
        self.finish_tiling_transition();
        self.original_positions.clear();
        self.active_windows.clear();
        let count = std::cmp::min(3, windows.len());
        let mut added = 0;
        for &hwnd in &windows {
            if !self.active_windows.contains(&hwnd) {
                unsafe {
                    // Если окно свернуто, разворачиваем его без передачи фокуса, 
                    // чтобы GetWindowRect вернул корректные координаты
                    if IsIconic(hwnd).as_bool() {
                        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                        std::thread::sleep(Duration::from_millis(50)); // Небольшая пауза для применения изменений ОС
                    }
                    let mut wr = RECT::default();
                    let _ = GetWindowRect(hwnd, &mut wr);
                    self.original_positions.insert(hwnd.0 as isize, wr);
                }
                self.active_windows.push(hwnd);
                added += 1;
                if added == count { break; }
            }
        }
        self.apply_layout();
        self.is_tiled = true;
        println!("[Тайлинг] Собрано {} окон.", self.active_windows.len());
    }

    fn untile(&mut self) {
        if self.active_windows.is_empty() { return; }
        self.pending_duplicate = None;
        let targets: Vec<_> = self.active_windows.iter().filter_map(|&hwnd| {
            self.original_positions.get(&(hwnd.0 as isize)).map(|r|
                (hwnd, r.left, r.top, r.right-r.left, r.bottom-r.top, false))
        }).collect();
        self.animate_positions(&targets);
        self.active_windows.clear();
        self.original_positions.clear();
        self.is_tiled = false;
        self.mirrored = false;
        println!("[Тайлинг] Окна расформированы.");
    }

    fn add_window(&mut self) {
        if !self.is_tiled { self.tile(); return; }
        if self.active_windows.len() >= 5 { return; }
        let windows = self.get_windows();
        for hwnd in windows {
            if !self.active_windows.contains(&hwnd) {
                unsafe {
                    // Аналогичная проверка и разворачивание для добавляемого окна
                    if IsIconic(hwnd).as_bool() {
                        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    let mut wr = RECT::default();
                    let _ = GetWindowRect(hwnd, &mut wr);
                    self.original_positions.insert(hwnd.0 as isize, wr);
                }
                self.active_windows.push(hwnd);
                self.apply_layout();
                return;
            }
        }
    }

    fn remove_window(&mut self) {
        if !self.is_tiled || self.active_windows.len() <= 1 { return; }
        if let Some(hwnd) = self.active_windows.pop() {
            let previous = self.original_positions.remove(&(hwnd.0 as isize));
            let mut targets = self.layout_targets();
            if let Some(r) = previous {
                targets.push((hwnd, r.left, r.top, r.right-r.left, r.bottom-r.top, false));
            }
            self.animate_positions(&targets);
        }
    }

    fn same_targets(&self) -> Vec<(HWND, i32, i32, i32, i32, bool)> {
        if self.same_windows.is_empty() { return Vec::new(); }
        let mut wa = RECT::default();
        unsafe { let _ = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut wa as *mut _ as *mut _), SPIF_UPDATEINIFILE); }
        let rects = Self::compute_rects(self.same_windows.len(), &wa, 10, false);
        self.same_windows.iter().zip(rects.iter())
            .map(|(&h, &(x, y, w, height))| (h, x, y, w, height, true)).collect()
    }

    fn apply_same_layout(&mut self) {
        let targets = self.same_targets();
        self.animate_positions(&targets);
    }

    fn leave_same_mode(&mut self) {
        if !self.same_mode { return; }
        self.pending_duplicate = None;
        let mut targets = if self.is_tiled { self.layout_targets() } else { Vec::new() };
        for &hwnd in &self.same_windows {
            if !self.active_windows.contains(&hwnd) {
                if let Some(r) = self.same_originals.get(&(hwnd.0 as isize)) {
                    targets.push((hwnd, r.left, r.top, r.right-r.left, r.bottom-r.top, false));
                }
            }
        }
        self.same_windows.clear();
        self.same_originals.clear();
        self.same_mode = false;
        self.animate_positions(&targets);
    }

    fn duplicate_main(&mut self) {
        if self.pending_close.is_some() { notice("Дождитесь ответа окна на запрос закрытия."); return; }
        if !self.same_mode {
            self.finish_tiling_transition();
            let hwnd = if self.is_tiled && !self.active_windows.is_empty() { self.active_windows[0] }
                else { unsafe { GetForegroundWindow() } };
            if !unsafe { self.is_taskable(hwnd) } {
                notice("Выберите обычное окно или сначала соберите тайлинг через Alt+Q."); return;
            }
            let mut r = RECT::default();
            if unsafe { GetWindowRect(hwnd, &mut r) }.is_err() { notice("Не удалось определить положение окна."); return; }
            self.same_windows.clear();
            self.same_originals.clear();
            self.same_originals.insert(hwnd.0 as isize, r);
            self.same_windows.push(hwnd);
            self.same_mode = true;
            self.apply_same_layout();
            unsafe { let _ = SetForegroundWindow(hwnd); }
            return;
        }
        if self.same_windows.len() >= 5 {
            notice("В отдельном тайлинге уже 5 окон. Сначала закройте последнее через Alt+F."); return;
        }
        if self.pending_duplicate.is_some() { notice("Предыдущее окно ещё запускается."); return; }
        let Some(&root) = self.same_windows.first() else { return; };
        let mut old_pid = 0u32;
        unsafe { GetWindowThreadProcessId(root, Some(&mut old_pid)); }
        let path = match process_executable(old_pid) {
            Ok(path) => path,
            Err(error) => { notice(&format!("Не удалось определить приложение: {error}")); return; }
        };
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.eq_ignore_ascii_case("conhost.exe") || name.eq_ignore_ascii_case("ApplicationFrameHost.exe") {
            notice("Приложение скрыто за системным процессом. Попробуйте Windows Terminal или обычное приложение."); return;
        }
        let initial_windows = self.get_windows().iter().map(|w| w.0 as isize).collect();
        let terminal = name.eq_ignore_ascii_case("WindowsTerminal.exe");
        let program = if terminal { std::ffi::OsStr::new("wt.exe") } else { path.as_os_str() };
        let mut command = std::process::Command::new(program);
        if terminal { command.args(["-w", "new"]); }
        else if ["chrome.exe", "msedge.exe", "brave.exe", "firefox.exe"].iter().any(|v| name.eq_ignore_ascii_case(v)) {
            command.arg("--new-window");
        }
        match command.spawn() {
            Ok(child) => {
                let child_pid = child.id();
                drop(child);
                self.pending_duplicate = Some(PendingDuplicate { initial_windows, old_pid, child_pid, started: Instant::now() });
            }
            Err(error) => notice(&format!("Не удалось запустить {}: {error}", path.display())),
        }
    }

    fn poll_duplicate(&mut self) {
        if self.spin_flag || self.pending_duplicate.is_none() { return; }
        if !self.same_mode || self.same_windows.len() >= 5 { self.pending_duplicate = None; return; }
        let pending = self.pending_duplicate.as_ref().unwrap();
        if pending.started.elapsed() >= Duration::from_secs(12) {
            self.pending_duplicate = None;
            notice("Новое окно не найдено за 12 секунд. Приложение может не поддерживать повторный запуск или использовать другой процесс.");
            return;
        }
        let mut direct = Vec::new();
        let mut existing_process = Vec::new();
        for hwnd in self.get_windows() {
            if pending.initial_windows.contains(&(hwnd.0 as isize)) || self.same_windows.contains(&hwnd) { continue; }
            let mut pid = 0;
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)); }
            if pid == pending.child_pid { direct.push(hwnd); }
            else if pid == pending.old_pid { existing_process.push(hwnd); }
        }
        let target = if direct.len() == 1 { direct.first().copied() }
            else if direct.is_empty() && existing_process.len() == 1 { existing_process.first().copied() }
            else { None };
        if let Some(hwnd) = target {
            let mut r = RECT::default();
            if unsafe { GetWindowRect(hwnd, &mut r) }.is_ok() {
                self.same_originals.insert(hwnd.0 as isize, r);
                self.same_windows.push(hwnd);
                self.pending_duplicate = None;
                self.apply_same_layout();
            }
        }
    }

    fn request_close_last(&mut self) {
        if self.pending_close.is_some() { notice("Окно ещё отвечает на предыдущий запрос закрытия."); return; }
        if self.pending_duplicate.is_some() { notice("Сначала дождитесь запуска нового окна."); return; }
        let last = if self.same_mode { self.same_windows.last().copied() }
            else if self.is_tiled { self.active_windows.last().copied() } else { None };
        let Some(hwnd) = last else { notice("Нет окон в активном тайлинге."); return; };
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)); }
        if unsafe { post_window_message(hwnd.0, WM_CLOSE, 0, 0) } == 0 {
            notice(&format!("Не удалось отправить запрос закрытия: {}", std::io::Error::last_os_error()));
            return;
        }
        self.pending_close = Some(PendingClose { hwnd, pid, same_mode: self.same_mode, started: Instant::now() });
    }

    fn poll_closed(&mut self) {
        let Some(pending) = self.pending_close.as_ref() else { return; };
        let hwnd = pending.hwnd;
        let same_mode = pending.same_mode;
        let mut pid = 0;
        let exists = unsafe { window_exists(hwnd.0) != 0 };
        if exists { unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)); } }
        if exists && pid == pending.pid {
            if pending.started.elapsed() >= Duration::from_secs(30) {
                self.pending_close = None;
                notice("Окно пока не закрылось. Проверьте запрос сохранения или попробуйте ещё раз.");
            }
            return;
        }
        self.pending_close = None;
        self.active_windows.retain(|w| *w != hwnd);
        self.original_positions.remove(&(hwnd.0 as isize));
        if self.active_windows.is_empty() { self.is_tiled = false; }
        if same_mode && self.same_mode {
            self.same_windows.retain(|w| *w != hwnd);
            self.same_originals.remove(&(hwnd.0 as isize));
            if self.same_windows.is_empty() { self.leave_same_mode(); }
            else { self.apply_same_layout(); }
        } else if self.is_tiled {
            self.apply_layout();
        }
    }

    fn start_selection(&mut self) {
        if !self.is_tiled || self.active_windows.is_empty() { return; }
        self.is_selecting = true;
        self.w_released = false;
        self.selected_index = (self.selected_index + 1) % self.active_windows.len();
        if let Some(&hwnd) = self.active_windows.get(self.selected_index) {
            unsafe {
                let _ = BringWindowToTop(hwnd);
                let _ = SetForegroundWindow(hwnd);
            }
        }
    }

    fn end_selection(&mut self) {
        if !self.is_selecting { return; }
        self.is_selecting = false;
        if self.selected_index > 0 && self.selected_index < self.active_windows.len() {
            self.active_windows.swap(0, self.selected_index);
            self.apply_layout();
        }
        self.selected_index = 0;
    }

    fn panel_p(&self) -> f32 {
        if self.closing.is_some() {
            let u = ((self.ct - 0.20) / 0.50).clamp(0.0, 1.0);
            1.0 - smoother(u)
        } else {
            smoother((self.t / PANEL_T).clamp(0.0, 1.0))
        }
    }

    fn card_p(&self, i: usize) -> f32 {
        if self.closing.is_some() {
            let u = ((self.ct - 0.04 * i as f32) / 0.32).clamp(0.0, 1.0);
            1.0 - u * u
        } else {
            let u = ((self.t - CARD_START - 0.07 * i as f32) / 0.65).clamp(0.0, 1.0);
            ease_out_back(u)
        }
    }

    fn center_p(&self) -> f32 {
        if self.closing.is_some() {
            1.0 - smooth(((self.ct - 0.05) / 0.30).clamp(0.0, 1.0))
        } else {
            ease_out_back(((self.t - 0.50) / 0.45).clamp(0.0, 1.0))
        }
    }

    fn pulse_now(&self) -> f32 {
        if self.closing.is_some() { pulse_env(self.ct) } else { pulse_env(self.t - LAND_T) }
    }

    fn layout(&self, sw: f32, sh: f32) -> Vec<Card> {
        let n = self.preview_data.len();
        if n == 0 { return Vec::new(); }
        let (cx, cy) = (sw / 2.0, sh / 2.0);
        let (rx, ry) = (sw * 0.34, sh * 0.25);
        let base_h = sh * 0.26;
        let land = 1.0 + 0.05 * self.pulse_now();
        self.preview_data.iter().enumerate().map(|(i, win)| {
            let e = self.card_p(i);
            let a = self.spin_angle + i as f32 * TAU / n as f32 + PI / 2.0;
            let depth = (a.sin() + 1.0) / 2.0;
            let scale = 0.72 + 0.28 * depth;
            let th = base_h * scale;
            let tw = (th * win.aspect).clamp(sw * 0.14, sw * 0.30 * scale);
            let tx = cx + rx * a.cos();
            let ty = cy + ry * a.sin();
            let mut k = land;
            if self.pick == Some(i) { k *= 1.0 + 0.10 * pulse_env(self.ct); }
            Card {
                x: lerp(cx, tx, e),
                y: lerp(cy, ty, e),
                w: (tw * e * k).max(1.0),
                h: (th * e * k).max(1.0),
                depth,
                p: e,
            }
        }).collect()
    }

    fn hit_test(&self, x: f32, y: f32) -> Option<usize> {
        let cards = self.layout(self.screen_w as f32, self.screen_h as f32);
        let mut order: Vec<usize> = (0..cards.len()).collect();
        order.sort_by(|&a, &b| cards[b].depth.partial_cmp(&cards[a].depth).unwrap());
        order.into_iter().find(|&i| {
            let c = &cards[i];
            c.p > 0.5 && (x - c.x).abs() <= c.w / 2.0 && (y - c.y).abs() <= c.h / 2.0
        })
    }

    fn begin_close(&mut self, pick: Option<usize>) {
        if self.spin_flag && self.closing.is_none() {
            self.closing = Some(Instant::now());
            self.ct = 0.0;
            self.pick = pick;
            self.hover = None;
        }
    }

    fn finish_close(&mut self) {
        let pick = self.pick.take();
        self.stop_spin();
        if let Some(i) = pick {
            if i < self.active_windows.len() {
                if i > 0 && self.is_tiled {
                    self.active_windows.swap(0, i);
                    self.apply_layout();
                    unsafe { let _ = SetForegroundWindow(self.active_windows[0]); }
                } else {
                    unsafe { let _ = SetForegroundWindow(self.active_windows[i]); }
                }
            }
        }
    }

    fn tick(&mut self) {
        if !self.spin_flag { return; }
        if !self.settings_state.is_null() {
            let (duration, colors) = unsafe {
                ((*self.settings_state).overlay_duration(), (*self.settings_state).overlay_colors())
            };
            self.overlay_duration = duration;
            if colors != self.overlay_colors {
                self.overlay_colors = colors;
                self.refresh_overlay_palette();
            }
        }
        let now = Instant::now();
        let dt = (now - self.last_tick).as_secs_f32().min(0.05);
        self.last_tick = now;
        let elapsed = (now - self.anim_start).as_secs_f32();
        let duration = self.overlay_duration.max(0.01);
        // Final card finishes by the configured time; ambient motion keeps real-time speed.
        let intro_end = CARD_START + 0.07 * self.preview_data.len().saturating_sub(1) as f32 + 0.65;
        self.t = if elapsed < duration { elapsed / duration * intro_end }
            else { intro_end + elapsed - duration };
        self.ct = self.closing.map(|c| (now - c).as_secs_f32() / duration * CLOSE_T).unwrap_or(0.0);
        let ramp = smooth(((self.t - 0.5) / 1.0).clamp(0.0, 1.0));
        let target = if self.hover.is_some() || self.closing.is_some() { 0.0 } else { 0.25 * ramp };
        self.spin_speed += (target - self.spin_speed) * (1.0 - (-dt * 7.0).exp());
        self.spin_angle += self.spin_speed * dt;
        if self.closing.is_some() && self.ct >= CLOSE_T {
            self.finish_close();
        }
    }

    fn refresh_overlay_palette(&mut self) {
        unsafe {
            if !self.cache_brush_ribbon.is_null() {
                let _ = GdipDeleteBrush(self.cache_brush_ribbon as *mut GpBrush);
                self.cache_brush_ribbon = std::ptr::null_mut();
            }
            let p1 = Point { X: 0, Y: 0 };
            let p2 = Point { X: self.screen_w, Y: 0 };
            let _ = GdipCreateLineBrushI(&p1, &p2,
                palette_color(240, self.overlay_colors.0), palette_color(240, self.overlay_colors.1),
                WrapModeTile, &mut self.cache_brush_ribbon);

            if self.bg_dc.is_invalid() || self.cache_brush_dim.is_null() { return; }
            let mut graphics: *mut GpGraphics = std::ptr::null_mut();
            let _ = GdipCreateFromHDC(self.bg_dc, &mut graphics);
            if graphics.is_null() { return; }
            let _ = GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias);
            let p2 = Point { X: self.screen_w, Y: self.screen_h };
            let darken = |c: (u8, u8, u8)| argb(255, c.0 / 8, c.1 / 8, c.2 / 8);
            let mut bg: *mut GpLineGradient = std::ptr::null_mut();
            let _ = GdipCreateLineBrushI(&p1, &p2,
                darken(self.overlay_colors.1), darken(self.overlay_colors.0), WrapModeTile, &mut bg);
            if !bg.is_null() {
                let _ = GdipFillRectangleI(graphics, bg as *mut GpBrush, 0, 0, self.screen_w, self.screen_h);
                let _ = GdipDeleteBrush(bg as *mut GpBrush);
            }
            let colors = [
                (0.16, 0.22, self.overlay_colors.1),
                (0.86, 0.84, self.overlay_colors.0),
                (0.50, 0.50, snake_color(0.5, self.overlay_colors.0, self.overlay_colors.1)),
            ];
            for (gx, gy, (r, g, b)) in colors {
                let (x, y) = (self.screen_w as f32 * gx, self.screen_h as f32 * gy);
                for k in 1..=12 {
                    let rad = self.screen_h as f32 * 0.055 * k as f32;
                    let _ = GdipSetSolidFillColor(self.cache_brush_dim, argb(9, r, g, b));
                    let _ = GdipFillEllipse(graphics, self.cache_brush_dim as *mut GpBrush,
                        x - rad, y - rad, rad * 2.0, rad * 2.0);
                }
            }
            let _ = GdipDeleteGraphics(graphics);
        }
    }

    fn start_spin(&mut self) {
        if self.active_windows.is_empty() { return; }
        if !self.settings_state.is_null() {
            unsafe {
                self.overlay_duration = (*self.settings_state).overlay_duration();
                self.overlay_colors = (*self.settings_state).overlay_colors();
            }
        }
        self.preview_data.clear();
        for &hwnd in &self.active_windows {
            let len = unsafe { GetWindowTextLengthW(hwnd) };
            let mut buf = vec![0u16; len as usize + 1];
            unsafe { GetWindowTextW(hwnd, &mut buf); }
            let title = String::from_utf16_lossy(&buf).trim_matches('\0').trim().to_string();
            let title = if title.chars().count() > 22 {
                format!("{}…", title.chars().take(22).collect::<String>())
            } else { title };
            let mut wr = RECT::default();
            unsafe { let _ = GetWindowRect(hwnd, &mut wr); }
            let bmp = unsafe { capture_window(hwnd, &wr) };
            let aspect = ((wr.right - wr.left) as f32 / (wr.bottom - wr.top).max(1) as f32).clamp(0.8, 2.2);
            self.preview_data.push(PreviewWindow { title, bmp, aspect });
        }
        self.spin_flag = true;
        self.spin_angle = 0.0;
        self.spin_speed = 0.0;
        self.hover = None;
        self.pick = None;
        self.closing = None;
        self.t = 0.0;
        self.ct = 0.0;
        self.anim_start = Instant::now();
        self.last_tick = Instant::now();
        unsafe {
            if self.cache_brush_bg.is_null() {
                let _ = GdipCreateSolidFill(0xFF07060Du32, &mut self.cache_brush_bg);
                let _ = GdipCreateSolidFill(0xFF000000u32, &mut self.cache_brush_dim);
                let p1 = Point { X: 0, Y: 0 };
                let p2 = Point { X: self.screen_w, Y: 0 };
                let _ = GdipCreateLineBrushI(&p1, &p2, palette_color(240, self.overlay_colors.0), palette_color(240, self.overlay_colors.1), WrapModeTile, &mut self.cache_brush_ribbon);
                let _ = GdipCreatePen1(0x4400FFFFu32, 12.0f32, UnitPixel, &mut self.cache_pen_glow);
                let _ = GdipCreatePen1(0x8800FFFFu32, 6.0f32, UnitPixel, &mut self.cache_pen_mid);
                let _ = GdipCreatePen1(0xFF00FFFFu32, 2.0f32, UnitPixel, &mut self.cache_pen_thin);
                let _ = GdipSetPenStartCap(self.cache_pen_glow, LineCapRound);
                let _ = GdipSetPenEndCap(self.cache_pen_glow, LineCapRound);
                let _ = GdipSetPenStartCap(self.cache_pen_mid, LineCapRound);
                let _ = GdipSetPenEndCap(self.cache_pen_mid, LineCapRound);
                let _ = GdipSetPenStartCap(self.cache_pen_thin, LineCapRound);
                let _ = GdipSetPenEndCap(self.cache_pen_thin, LineCapRound);
                let _ = GdipCreateSolidFill(0xFF00FFFFu32, &mut self.cache_brush_center);
                let _ = GdipCreatePen1(0xFF00FFFFu32, 3.0f32, UnitPixel, &mut self.cache_pen_center);
                let _ = GdipCreateSolidFill(0xFF1A1A1Au32, &mut self.cache_brush_card);
                let _ = GdipCreatePen1(0xFF333333u32, 1.0f32, UnitPixel, &mut self.cache_pen_border);
                let _ = GdipCreateSolidFill(0xFFFFFFFFu32, &mut self.cache_brush_text);
                let _ = GdipCreateSolidFill(0xFFFF5F56u32, &mut self.cache_brush_red);
                let _ = GdipCreateSolidFill(0xFFFFBD2Eu32, &mut self.cache_brush_yellow);
                let _ = GdipCreateSolidFill(0xFF27C93Fu32, &mut self.cache_brush_green);
                let mut font_family: *mut GpFontFamily = std::ptr::null_mut();
                let family_name: Vec<u16> = "Segoe UI\0".encode_utf16().collect();
                let _ = GdipCreateFontFamilyFromName(PCWSTR(family_name.as_ptr()), std::ptr::null_mut(), &mut font_family);
                let _ = GdipCreateFont(font_family, 12.0f32, 0, UnitPixel, &mut self.cache_font_title);
                let _ = GdipDeleteFontFamily(font_family);
                let _ = GdipCreateStringFormat(0, 0, &mut self.cache_format_center);
                let _ = GdipSetStringFormatAlign(self.cache_format_center, StringAlignmentCenter);
                let _ = GdipSetStringFormatLineAlign(self.cache_format_center, StringAlignmentCenter);
            }
            let (sw, sh) = (self.screen_w, self.screen_h);
            let sdc = GetDC(None);
            self.shot_dc = CreateCompatibleDC(Some(sdc));
            self.shot_bmp = CreateCompatibleBitmap(sdc, sw, sh);
            SelectObject(self.shot_dc, self.shot_bmp.into());
            let _ = BitBlt(self.shot_dc, 0, 0, sw, sh, Some(sdc), 0, 0, SRCCOPY);
            self.bg_dc = CreateCompatibleDC(Some(sdc));
            self.bg_bmp = CreateCompatibleBitmap(sdc, sw, sh);
            SelectObject(self.bg_dc, self.bg_bmp.into());
            self.mem_dc = CreateCompatibleDC(Some(sdc));
            self.mem_bmp = CreateCompatibleBitmap(sdc, sw, sh);
            SelectObject(self.mem_dc, self.mem_bmp.into());
            ReleaseDC(None, sdc);
            let mut gb: *mut GpGraphics = std::ptr::null_mut();
            let _ = GdipCreateFromHDC(self.bg_dc, &mut gb);
            let _ = GdipSetSmoothingMode(gb, SmoothingModeAntiAlias);
            let p1 = Point { X: 0, Y: 0 };
            let p2 = Point { X: sw, Y: sh };
            let mut lb: *mut GpLineGradient = std::ptr::null_mut();
            let darken = |c: (u8, u8, u8)| argb(255, c.0 / 8, c.1 / 8, c.2 / 8);
            let _ = GdipCreateLineBrushI(&p1, &p2, darken(self.overlay_colors.1), darken(self.overlay_colors.0), WrapModeTile, &mut lb);
            let _ = GdipFillRectangleI(gb, lb as *mut GpBrush, 0, 0, sw, sh);
            let _ = GdipDeleteBrush(lb as *mut GpBrush);
            let hf = sh as f32;
            let glows: [(f32, f32, (u8, u8, u8)); 3] = [
                (0.16, 0.22, self.overlay_colors.1),
                (0.86, 0.84, self.overlay_colors.0),
                (0.50, 0.50, snake_color(0.5, self.overlay_colors.0, self.overlay_colors.1)),
            ];
            for (gx, gy, (r, g, b)) in glows {
                let (x, y) = (sw as f32 * gx, hf * gy);
                for k in 1..=12 {
                    let rad = hf * 0.055 * k as f32;
                    let _ = GdipSetSolidFillColor(self.cache_brush_dim, argb(9, r, g, b));
                    let _ = GdipFillEllipse(gb, self.cache_brush_dim as *mut GpBrush, x - rad, y - rad, rad * 2.0, rad * 2.0);
                }
            }
            let _ = GdipDeleteGraphics(gb);
            let _ = BitBlt(self.mem_dc, 0, 0, sw, sh, Some(self.shot_dc), 0, 0, SRCCOPY);
            self.anim_start = Instant::now();
            self.last_tick = self.anim_start;
            let _ = SetWindowPos(self.overlay_hwnd, Some(HWND_TOPMOST), 0, 0, sw, sh, SWP_SHOWWINDOW);
            let _ = SetForegroundWindow(self.overlay_hwnd);
            let _ = InvalidateRect(Some(self.overlay_hwnd), None, false);
            let _ = UpdateWindow(self.overlay_hwnd);
        }
    }

    fn stop_spin(&mut self) {
        unsafe {
            for p in &self.preview_data {
                if !p.bmp.is_null() { let _ = GdipDisposeImage(p.bmp as *mut GpImage); }
            }
        }
        self.preview_data.clear();
        self.closing = None;
        self.spin_flag = false;
        unsafe {
            let _ = ShowWindow(self.overlay_hwnd, SW_HIDE);
            if !self.mem_dc.is_invalid() {
                let _ = DeleteDC(self.mem_dc);
                let _ = DeleteObject(self.mem_bmp.into());
                self.mem_dc = HDC(std::ptr::null_mut());
                self.mem_bmp = HBITMAP(std::ptr::null_mut());
            }
            if !self.shot_dc.is_invalid() {
                let _ = DeleteDC(self.shot_dc);
                let _ = DeleteObject(self.shot_bmp.into());
                self.shot_dc = HDC(std::ptr::null_mut());
                self.shot_bmp = HBITMAP(std::ptr::null_mut());
            }
            if !self.bg_dc.is_invalid() {
                let _ = DeleteDC(self.bg_dc);
                let _ = DeleteObject(self.bg_bmp.into());
                self.bg_dc = HDC(std::ptr::null_mut());
                self.bg_bmp = HBITMAP(std::ptr::null_mut());
            }
            if !self.cache_brush_bg.is_null() { let _ = GdipDeleteBrush(self.cache_brush_bg as *mut GpBrush); self.cache_brush_bg = std::ptr::null_mut(); }
            if !self.cache_brush_dim.is_null() { let _ = GdipDeleteBrush(self.cache_brush_dim as *mut GpBrush); self.cache_brush_dim = std::ptr::null_mut(); }
            if !self.cache_brush_ribbon.is_null() { let _ = GdipDeleteBrush(self.cache_brush_ribbon as *mut GpBrush); self.cache_brush_ribbon = std::ptr::null_mut(); }
            if !self.cache_pen_glow.is_null() { let _ = GdipDeletePen(self.cache_pen_glow); self.cache_pen_glow = std::ptr::null_mut(); }
            if !self.cache_pen_mid.is_null() { let _ = GdipDeletePen(self.cache_pen_mid); self.cache_pen_mid = std::ptr::null_mut(); }
            if !self.cache_pen_thin.is_null() { let _ = GdipDeletePen(self.cache_pen_thin); self.cache_pen_thin = std::ptr::null_mut(); }
            if !self.cache_brush_center.is_null() { let _ = GdipDeleteBrush(self.cache_brush_center as *mut GpBrush); self.cache_brush_center = std::ptr::null_mut(); }
            if !self.cache_pen_center.is_null() { let _ = GdipDeletePen(self.cache_pen_center); self.cache_pen_center = std::ptr::null_mut(); }
            if !self.cache_brush_card.is_null() { let _ = GdipDeleteBrush(self.cache_brush_card as *mut GpBrush); self.cache_brush_card = std::ptr::null_mut(); }
            if !self.cache_pen_border.is_null() { let _ = GdipDeletePen(self.cache_pen_border); self.cache_pen_border = std::ptr::null_mut(); }
            if !self.cache_brush_text.is_null() { let _ = GdipDeleteBrush(self.cache_brush_text as *mut GpBrush); self.cache_brush_text = std::ptr::null_mut(); }
            if !self.cache_font_title.is_null() { let _ = GdipDeleteFont(self.cache_font_title); self.cache_font_title = std::ptr::null_mut(); }
            if !self.cache_format_center.is_null() { let _ = GdipDeleteStringFormat(self.cache_format_center); self.cache_format_center = std::ptr::null_mut(); }
            if !self.cache_brush_red.is_null() { let _ = GdipDeleteBrush(self.cache_brush_red as *mut GpBrush); self.cache_brush_red = std::ptr::null_mut(); }
            if !self.cache_brush_yellow.is_null() { let _ = GdipDeleteBrush(self.cache_brush_yellow as *mut GpBrush); self.cache_brush_yellow = std::ptr::null_mut(); }
            if !self.cache_brush_green.is_null() { let _ = GdipDeleteBrush(self.cache_brush_green as *mut GpBrush); self.cache_brush_green = std::ptr::null_mut(); }
        }
    }
}

// ---------------------------------------------------------------- оконные процедуры
unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

unsafe extern "system" fn overlay_wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        let sp = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        match msg {
            WM_SETCURSOR => {
                if let Ok(c) = LoadCursorW(None, IDC_ARROW) { let _ = SetCursor(Some(c)); }
                return LRESULT(1);
            }
            WM_ERASEBKGND => return LRESULT(1),
            WM_MOUSEMOVE | WM_LBUTTONDOWN => {
                if !sp.is_null() {
                    let s = &mut *sp;
                    if s.spin_flag && s.closing.is_none() && s.t > 0.9 {
                        let x = (lparam.0 & 0xFFFF) as i16 as f32;
                        let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
                        let hit = s.hit_test(x, y);
                        if msg == WM_MOUSEMOVE {
                            s.hover = hit;
                        } else if hit.is_some() {
                            s.begin_close(hit);
                        }
                    }
                }
                return LRESULT(0);
            }
            WM_KEYDOWN => {
                if wparam.0 as u16 == VK_ESCAPE.0 && !sp.is_null() { (*sp).begin_close(None); }
                return LRESULT(0);
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                if !sp.is_null() {
                    let state = &*sp;
                    if state.spin_flag && !state.mem_dc.is_invalid() { paint(state, hdc); }
                }
                let _ = EndPaint(hwnd, &ps);
                return LRESULT(0);
            }
            _ => {}
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

unsafe fn edge_line(g: *mut GpGraphics, pen: *mut GpPen, pts: &[PointF], t: f32, w: f32, width: f32, colors: ((u8, u8, u8), (u8, u8, u8))) {
    for k in 1..pts.len() {
        let v = pingpong(pts[k].X / w * 0.9 + t * 0.12);
        let (r, gc, b) = snake_color(v, colors.0, colors.1);
        let _ = GdipSetPenColor(pen, argb(255, r, gc, b));
        let _ = GdipSetPenWidth(pen, width);
        let _ = GdipDrawLine(g, pen, pts[k - 1].X, pts[k - 1].Y, pts[k].X, pts[k].Y);
    }
}

unsafe fn paint(state: &State, hdc: HDC) {
    let (sw, sh) = (state.screen_w, state.screen_h);
    let (w, h) = (sw as f32, sh as f32);
    let (cx, cy) = (w / 2.0, h / 2.0);
    let t = state.t;
    let (color_a, color_b) = state.overlay_colors;
    let pp = state.panel_p();
    let motion = 4.0 * pp * (1.0 - pp);
    let amp = 1.0 + 0.9 * motion;
    let margin = h * 0.06;
    let top_y = lerp(h + 120.0, margin, pp);
    let bot_y = lerp(h + 240.0, h - margin, pp);
    let step = 8.0f32;
    let mut top: Vec<PointF> = Vec::new();
    let mut bot: Vec<PointF> = Vec::new();
    let mut top_out: Vec<PointF> = Vec::new();
    let mut bot_out: Vec<PointF> = Vec::new();
    let mut x = 0.0f32;
    while x <= w + step {
        let ty = top_y + wave(x, t, 0.0) * amp;
        let by = bot_y + wave(x, t, 2.3) * amp;
        top.push(PointF { X: x, Y: ty });
        bot.push(PointF { X: x, Y: by });
        let th1 = 26.0 + 9.0 * (x * 0.013 - t * 1.7).sin();
        let th2 = 26.0 + 9.0 * (x * 0.017 + t * 1.4).sin();
        top_out.push(PointF { X: x, Y: ty - th1 });
        bot_out.push(PointF { X: x, Y: by + th2 });
        x += step;
    }
    let strip_top_end = ((top_y + 55.0 * amp) as i32).clamp(0, sh);
    let strip_bot_start = ((bot_y - 55.0 * amp) as i32).clamp(0, sh);
    if strip_top_end > 0 {
        let _ = BitBlt(state.mem_dc, 0, 0, sw, strip_top_end, Some(state.shot_dc), 0, 0, SRCCOPY);
    }
    if strip_bot_start < sh {
        let _ = BitBlt(state.mem_dc, 0, strip_bot_start, sw, sh - strip_bot_start, Some(state.shot_dc), 0, strip_bot_start, SRCCOPY);
    }
    let dim_a = (150.0 * pp) as u8;
    if dim_a > 0 {
        let mut g1: *mut GpGraphics = std::ptr::null_mut();
        let _ = GdipCreateFromHDC(state.mem_dc, &mut g1);
        let _ = GdipSetSolidFillColor(state.cache_brush_dim, argb(dim_a, 6, 4, 14));
        if strip_top_end > 0 {
            let _ = GdipFillRectangle(g1, state.cache_brush_dim as *mut GpBrush, 0.0, 0.0, w, strip_top_end as f32);
        }
        if strip_bot_start < sh {
            let _ = GdipFillRectangle(g1, state.cache_brush_dim as *mut GpBrush, 0.0, strip_bot_start as f32, w, (sh - strip_bot_start) as f32);
        }
        let _ = GdipDeleteGraphics(g1);
    }
    let mut rp: Vec<POINT> = Vec::with_capacity(top.len() * 2);
    for p in &top { rp.push(POINT { x: p.X as i32, y: p.Y as i32 }); }
    for p in bot.iter().rev() { rp.push(POINT { x: p.X as i32, y: p.Y as i32 }); }
    let rgn = CreatePolygonRgn(&rp, ALTERNATE);
    let _ = SelectClipRgn(state.mem_dc, Some(rgn));
    let _ = BitBlt(state.mem_dc, 0, 0, sw, sh, Some(state.bg_dc), 0, 0, SRCCOPY);
    let _ = SelectClipRgn(state.mem_dc, None);
    let _ = DeleteObject(rgn.into());
    let mut g: *mut GpGraphics = std::ptr::null_mut();
    let _ = GdipCreateFromHDC(state.mem_dc, &mut g);
    let _ = GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
    let _ = GdipSetTextRenderingHint(g, TextRenderingHintAntiAlias);
    let _ = GdipSetInterpolationMode(g, InterpolationModeBilinear);
    let mut poly: Vec<PointF> = top_out.clone();
    poly.extend(top.iter().rev().cloned());
    let _ = GdipFillPolygon(g, state.cache_brush_ribbon as *mut GpBrush, poly.as_ptr(), poly.len() as i32, FillModeAlternate);
    let mut poly2: Vec<PointF> = bot.clone();
    poly2.extend(bot_out.iter().rev().cloned());
    let _ = GdipFillPolygon(g, state.cache_brush_ribbon as *mut GpBrush, poly2.as_ptr(), poly2.len() as i32, FillModeAlternate);
    let pen = state.cache_pen_thin;
    for edge in [&top, &bot] {
        let _ = GdipSetPenColor(pen, palette_color(0x1A, color_b));
        let _ = GdipSetPenWidth(pen, 24.0);
        let _ = GdipDrawLines(g, pen, edge.as_ptr(), edge.len() as i32);
        let _ = GdipSetPenColor(pen, palette_color(0x34, color_a));
        let _ = GdipSetPenWidth(pen, 11.0);
        let _ = GdipDrawLines(g, pen, edge.as_ptr(), edge.len() as i32);
        edge_line(g, pen, edge, t, w, 3.5, state.overlay_colors);
    }
    for edge in [&top_out, &bot_out] {
        let _ = GdipSetPenColor(pen, argb(0x70, 255, 255, 255));
        let _ = GdipSetPenWidth(pen, 1.5);
        let _ = GdipDrawLines(g, pen, edge.as_ptr(), edge.len() as i32);
    }
    let cards = state.layout(w, h);
    let n = 56usize;
    for (i, c) in cards.iter().enumerate() {
        if c.p < 0.02 { continue; }
        let fade = c.p.clamp(0.0, 1.0);
        let (vx, vy) = (c.x - cx, c.y - cy);
        let len = (vx * vx + vy * vy).sqrt().max(1.0);
        let (nx, ny) = (-vy / len, vx / len);
        let hovered = state.hover == Some(i);
        let amp_s = (if hovered { 70.0 } else { 48.0 }) * fade;
        let breathe = 0.6 + 0.4 * (t * 1.3 + i as f32).sin();
        let mut pts: Vec<PointF> = Vec::with_capacity(n + 1);
        for k in 0..=n {
            let u = k as f32 / n as f32;
            let env = (u * PI).sin();
            let off = ((u * 9.0 - t * 3.4 + i as f32 * 1.7).sin() * 0.7
                + (u * 17.0 + t * 5.1 + i as f32).sin() * 0.3 * breathe) * amp_s * env;
            pts.push(PointF { X: cx + vx * u + nx * off, Y: cy + vy * u + ny * off });
        }
        let _ = GdipSetPenColor(pen, palette_color((0x1C as f32 * fade) as u8, color_a));
        let _ = GdipSetPenWidth(pen, 36.0);
        let _ = GdipDrawLines(g, pen, pts.as_ptr(), pts.len() as i32);
        let _ = GdipSetPenColor(pen, palette_color((0x34 as f32 * fade) as u8, color_b));
        let _ = GdipSetPenWidth(pen, 20.0);
        let _ = GdipDrawLines(g, pen, pts.as_ptr(), pts.len() as i32);
        let phase = 0.5 + 0.5 * (t * 1.1 + i as f32).sin();
        for k in 1..=n {
            let u = k as f32 / n as f32;
            let (r, gr, b) = snake_color(u * 0.85 + 0.15 * phase, color_a, color_b);
            let _ = GdipSetPenColor(pen, argb(255, (r as f32 * fade) as u8, (gr as f32 * fade) as u8, (b as f32 * fade) as u8));
            let bulge = 1.0 + 0.25 * (u * 12.0 - t * 5.0 + i as f32).sin();
            let wd = lerp(15.0, 3.5, u) * bulge * if hovered { 1.25 } else { 1.0 };
            let _ = GdipSetPenWidth(pen, wd);
            let _ = GdipDrawLine(g, pen, pts[k - 1].X, pts[k - 1].Y, pts[k].X, pts[k].Y);
        }
        let su = (t * 0.55 + i as f32 * 0.37) % 1.0;
        let sp = pts[((su * n as f32) as usize).min(n)];
        for (rad, a) in [(22.0f32, 0x20u8), (13.0, 0x55), (6.0, 0xFF)] {
            let _ = GdipSetSolidFillColor(state.cache_brush_dim, argb((a as f32 * fade) as u8, 220, 255, 255));
            let _ = GdipFillEllipse(g, state.cache_brush_dim as *mut GpBrush, sp.X - rad, sp.Y - rad, rad * 2.0, rad * 2.0);
        }
    }
    let ages: [f32; 2] = if state.closing.is_some() { [state.ct, state.ct - 0.12] } else { [t - LAND_T, t - LAND_T - 0.14] };
    for (ri, age) in ages.iter().enumerate() {
        if *age > 0.0 && *age < 0.9 {
            let k = *age / 0.9;
            let rr = 1.0 - (1.0 - k) * (1.0 - k);
            let radius = 30.0 + rr * h * 0.6;
            let alpha = ((1.0 - k) * (1.0 - k) * 170.0) as u8;
            let (r, gc, b) = if ri == 0 { color_a } else { color_b };
            let _ = GdipSetPenColor(state.cache_pen_center, argb(alpha, r, gc, b));
            let _ = GdipSetPenWidth(state.cache_pen_center, lerp(6.0, 1.5, k));
            let _ = GdipDrawEllipse(g, state.cache_pen_center, cx - radius, cy - radius, radius * 2.0, radius * 2.0);
        }
    }
    let cs = state.center_p();
    if cs > 0.01 {
        let cr = (30.0 + 3.0 * (t * 3.0).sin() + 14.0 * state.pulse_now()) * cs;
        for (m, a) in [(2.4f32, 0x14u8), (1.8, 0x22), (1.35, 0x38)] {
            let _ = GdipSetSolidFillColor(state.cache_brush_dim, palette_color(a, color_a));
            let rr = cr * m;
            let _ = GdipFillEllipse(g, state.cache_brush_dim as *mut GpBrush, cx - rr, cy - rr, rr * 2.0, rr * 2.0);
        }
        let _ = GdipSetSolidFillColor(state.cache_brush_center, palette_color(255, color_a));
        let _ = GdipFillEllipse(g, state.cache_brush_center as *mut GpBrush, cx - cr, cy - cr, cr * 2.0, cr * 2.0);
        let _ = GdipSetSolidFillColor(state.cache_brush_dim, argb(255, 7, 6, 13));
        let ir = cr * 0.62;
        let _ = GdipFillEllipse(g, state.cache_brush_dim as *mut GpBrush, cx - ir, cy - ir, ir * 2.0, ir * 2.0);
        let _ = GdipSetSolidFillColor(state.cache_brush_dim, palette_color(255, color_b));
        let kr = cr * 0.28;
        let _ = GdipFillEllipse(g, state.cache_brush_dim as *mut GpBrush, cx - kr, cy - kr, kr * 2.0, kr * 2.0);
    }
    let mut order: Vec<usize> = (0..cards.len()).collect();
    order.sort_by(|&a, &b| cards[a].depth.partial_cmp(&cards[b].depth).unwrap());
    for idx in order {
        let c = &cards[idx];
        if c.p < 0.03 { continue; }
        let win = &state.preview_data[idx];
        let hov = state.hover == Some(idx) || state.pick == Some(idx);
        let (l, tp) = (c.x - c.w / 2.0, c.y - c.h / 2.0);
        let sh_path = rounded_path(l + 6.0, tp + 10.0, c.w, c.h, 12.0);
        let _ = GdipSetSolidFillColor(state.cache_brush_dim, argb(90, 0, 0, 0));
        let _ = GdipFillPath(g, state.cache_brush_dim as *mut GpBrush, sh_path);
        let _ = GdipDeletePath(sh_path);
        if hov {
            for (wd, al) in [(18.0f32, 0x14u8), (10.0, 0x2A), (4.0, 0x90)] {
                let gp = rounded_path(l, tp, c.w, c.h, 12.0);
                let _ = GdipSetPenColor(state.cache_pen_glow, palette_color(al, color_b));
                let _ = GdipSetPenWidth(state.cache_pen_glow, wd);
                let _ = GdipDrawPath(g, state.cache_pen_glow, gp);
                let _ = GdipDeletePath(gp);
            }
        }
        let path = rounded_path(l, tp, c.w, c.h, 12.0);
        let _ = GdipFillPath(g, state.cache_brush_card as *mut GpBrush, path);
        let _ = GdipSetClipPath(g, path, CombineModeReplace);
        if !win.bmp.is_null() {
            let _ = GdipDrawImageRectI(g, win.bmp as *mut GpImage,
                l as i32, (tp + HEADER) as i32, c.w as i32, (c.h - HEADER).max(1.0) as i32);
        }
        let dim = ((1.0 - c.depth) * 150.0) as u8;
        let _ = GdipSetSolidFillColor(state.cache_brush_dim, argb(dim, 5, 5, 12));
        let _ = GdipFillPath(g, state.cache_brush_dim as *mut GpBrush, path);
        let _ = GdipResetClip(g);
        let _ = GdipSetPenColor(state.cache_pen_border,
            if hov { palette_color(255, color_b) } else { palette_color(0x80, color_a) });
        let _ = GdipSetPenWidth(state.cache_pen_border, if hov { 2.5 } else { 1.5 });
        let _ = GdipDrawPath(g, state.cache_pen_border, path);
        let _ = GdipDeletePath(path);
        let by = tp + HEADER / 2.0 - 5.0;
        let _ = GdipFillEllipse(g, state.cache_brush_red as *mut GpBrush, l + 12.0, by, 10.0, 10.0);
        let _ = GdipFillEllipse(g, state.cache_brush_yellow as *mut GpBrush, l + 28.0, by, 10.0, 10.0);
        let _ = GdipFillEllipse(g, state.cache_brush_green as *mut GpBrush, l + 44.0, by, 10.0, 10.0);
        let wide: Vec<u16> = win.title.encode_utf16().chain(std::iter::once(0)).collect();
        let rect = RectF { X: l + 56.0, Y: tp, Width: (c.w - 68.0).max(1.0), Height: HEADER };
        let _ = GdipDrawString(g, PCWSTR(wide.as_ptr()), -1, state.cache_font_title, &rect,
            state.cache_format_center, state.cache_brush_text as *mut GpBrush);
    }
    let _ = GdipDeleteGraphics(g);
    let _ = BitBlt(hdc, 0, 0, sw, sh, Some(state.mem_dc), 0, 0, SRCCOPY);
}

// ---------------------------------------------------------------- main
fn main() {
    // Служебные режимы холста: тот же exe запускается как процесс-страховка.
    if std::env::args().nth(1).as_deref() == Some("--holst-watchdog") {
        unsafe { holst::watchdog(); }
        return;
    }
    // Холсту нужны реальные пиксели — DPI-режим выставляем до создания любых окон.
    holst::init_dpi();

    println!("=== Q-Tiler (Hyperland Neon Exposé) === ");
    println!("Alt+Q       - Сбор/Разбор тайлинга ");
    println!("Alt+A       - Добавить окно ");
    println!("Alt+S       - Убрать окно ");
    println!("Alt+W       - Выбрать главное окно ");
    println!("Alt+D       - Exposé (панель снизу + змеи) ");
    println!("Alt+E       - Зеркало раскладки ");
    println!("Alt+Z       - Настройки / Бинды ");
    println!("Alt+X       - 3D-обзор окон ");
    println!("Alt+C       - Бесконечный холст ");
    println!("Alt+V       - Снять выбор окна на холсте ");
    println!("Ctrl+Alt+Q  - Выход ");

    let mut state = State::new();
    unsafe {
        state.screen_w = GetSystemMetrics(SM_CXSCREEN);
        state.screen_h = GetSystemMetrics(SM_CYSCREEN);
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            DebugEventCallback: 0,
            SuppressBackgroundThread: BOOL(0),
            SuppressExternalCodecs: BOOL(0),
        };
        let _ = GdiplusStartup(&mut state.gdiplus_token as *mut _, &input, std::ptr::null_mut());
        let hinstance = GetModuleHandleW(None).unwrap();
        let hinstance_ptr: HINSTANCE = hinstance.into();
        let class_name = w!("QtilClass");
        let wc = WNDCLASSW { lpfnWndProc: Some(wnd_proc), hInstance: hinstance_ptr, lpszClassName: class_name, ..Default::default() };
        RegisterClassW(&wc);
        let hotkey_hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), class_name, w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND(std::ptr::null_mut())), Some(HMENU(std::ptr::null_mut())), Some(hinstance_ptr), None).unwrap();
        let overlay_class = w!("QtilOverlay");
        let wc_overlay = WNDCLASSW { lpfnWndProc: Some(overlay_wnd_proc), hInstance: hinstance_ptr, lpszClassName: overlay_class, ..Default::default() };
        RegisterClassW(&wc_overlay);
        state.overlay_hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            overlay_class, w!(""),
            WS_POPUP,
            0, 0, state.screen_w, state.screen_h,
            Some(HWND(std::ptr::null_mut())), Some(HMENU(std::ptr::null_mut())), Some(hinstance_ptr), None
        ).unwrap();

        // Создание окна настроек через config_menu
        let (settings_hwnd, settings_state_ptr) = config_menu::create_settings_window(hinstance_ptr, state.screen_w, state.screen_h);
        state.settings_hwnd = settings_hwnd;
        state.settings_state = settings_state_ptr;
        let mut room = match world3d::Room::new(hinstance_ptr, state.screen_w, state.screen_h) {
            Ok(room) => Some(room),
            Err(error) => { println!("3D-обзор недоступен: {error}"); None }
        };

        let holst_hwnd = holst::create();
        if holst_hwnd == 0 { println!("Холст недоступен: не удалось создать окно."); }

        SetWindowLongPtrW(state.overlay_hwnd, GWLP_USERDATA, &mut state as *mut _ as isize);
        let _ = RegisterHotKey(Some(hotkey_hwnd), 1, MOD_ALT | MOD_NOREPEAT, 'Q' as u32);
        let _ = RegisterHotKey(Some(hotkey_hwnd), 2, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, 'Q' as u32);
        let _ = RegisterHotKey(Some(hotkey_hwnd), 3, MOD_ALT | MOD_NOREPEAT, 'A' as u32);
        let _ = RegisterHotKey(Some(hotkey_hwnd), 4, MOD_ALT | MOD_NOREPEAT, 'S' as u32);
        let _ = RegisterHotKey(Some(hotkey_hwnd), 5, MOD_ALT | MOD_NOREPEAT, 'W' as u32);
        let _ = RegisterHotKey(Some(hotkey_hwnd), 6, MOD_ALT | MOD_NOREPEAT, 'D' as u32);
        let _ = RegisterHotKey(Some(hotkey_hwnd), 7, MOD_ALT | MOD_NOREPEAT, 'E' as u32);
        let _ = RegisterHotKey(Some(hotkey_hwnd), 8, MOD_ALT | MOD_NOREPEAT, 'Z' as u32); // Настройки
        if RegisterHotKey(Some(hotkey_hwnd), 9, MOD_ALT | MOD_NOREPEAT, 'R' as u32).is_err() {
            notice("Alt+R уже занят другой программой. Закройте её или измените сочетание.");
        }

        if RegisterHotKey(Some(hotkey_hwnd), 10, MOD_ALT | MOD_NOREPEAT, 'F' as u32).is_err() {
            notice("Alt+F уже занят другой программой.");
        }

        if RegisterHotKey(Some(hotkey_hwnd), 11, MOD_ALT | MOD_NOREPEAT, 'X' as u32).is_err() {
            notice("Alt+X уже занят другой программой. 3D-обзор по этой клавише недоступен.");
        }

        if RegisterHotKey(Some(hotkey_hwnd), 12, MOD_ALT | MOD_NOREPEAT, 'C' as u32).is_err() {
            notice("Alt+C уже занят другой программой. Холст по этой клавише недоступен.");
        }

        let frame = Duration::from_millis(FRAME_MS);
        let mut next_frame = Instant::now();
        let mut msg = MSG::default();
        loop {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT { break; }
                if msg.message == WM_HOTKEY {
                    let id = msg.wParam.0 as i32;
                    // id >= 200 — внутренние хоткеи холста (Alt+=, Alt+-, Alt+V), их обрабатывает holst.rs
                    if id < 200 {
                        // пока открыт холст, остальные бинды тайлинга отключены (кроме выхода и самого Alt+C)
                        if holst::is_open(holst_hwnd) && id != 2 && id != 12 { continue; }
                        if id != 2 && id != 11 && room.as_ref().is_some_and(|r| r.is_visible()) { continue; }
                    }
                    match id {
                        1 => { if !state.spin_flag { state.toggle() } },
                        2 => {
                            if state.same_mode { state.leave_same_mode(); }
                            state.finish_tiling_transition();
                            for i in 1..=12 { let _ = UnregisterHotKey(Some(hotkey_hwnd), i); }
                            holst::shutdown(holst_hwnd);
                            drop(room.take());
                            let _ = GdiplusShutdown(state.gdiplus_token);
                            if !state.settings_state.is_null() {
                                config_menu::free_settings_state(state.settings_state);
                            }
                            std::process::exit(0);
                        }
                        11 => {
                            if let Some(view) = room.as_mut() {
                                if view.is_active() {
                                    view.close();
                                } else if !state.spin_flag && !state.is_selecting && state.transition.is_none()
                                    && state.pending_duplicate.is_none() && state.pending_close.is_none() {
                                    if IsWindowVisible(state.settings_hwnd).as_bool() {
                                        let _ = ShowWindow(state.settings_hwnd, SW_HIDE);
                                    }
                                    let windows = state.get_windows();
                                    view.open(&windows);
                                } else {
                                    notice("Дождитесь завершения текущего действия перед открытием 3D-обзора.");
                                }
                            } else { notice("Не удалось создать окно 3D-обзора."); }
                        },
                        12 => {
                            // Alt+C — бесконечный холст
                            if holst::is_open(holst_hwnd) {
                                holst::toggle(holst_hwnd);
                            } else if !state.spin_flag && !state.is_selecting && state.pending_duplicate.is_none() && state.pending_close.is_none() {
                                state.finish_tiling_transition();
                                holst::toggle(holst_hwnd);
                            }
                        },
                        9 => { if !state.spin_flag { state.duplicate_main() } },
                        10 => { if !state.spin_flag { state.request_close_last() } },
                        3 => { if !state.spin_flag && !state.same_mode && state.pending_close.is_none() { state.add_window() } },
                        4 => { if !state.spin_flag && !state.same_mode && state.pending_close.is_none() { state.remove_window() } },
                        5 => { if !state.spin_flag && !state.same_mode && state.pending_close.is_none() { state.start_selection() } },
                        7 => { if !state.spin_flag && !state.same_mode && state.pending_close.is_none() { state.toggle_mirror() } },
                        8 => {
                            // Открыть/Закрыть настройки (Alt+Z)
                            if IsWindowVisible(state.settings_hwnd).as_bool() {
                                let _ = ShowWindow(state.settings_hwnd, SW_HIDE);
                            } else {
                                // Сброс анимации появления
                                if !state.settings_state.is_null() {
                                    (*state.settings_state).anim_t = 0.0;
                                }
                                let _ = ShowWindow(state.settings_hwnd, SW_SHOW);
                                let _ = SetForegroundWindow(state.settings_hwnd);
                                let _ = InvalidateRect(Some(state.settings_hwnd), None, false);
                            }
                        },
                        6 => {
                            if state.spin_flag { state.begin_close(None); } else if !state.same_mode { state.start_spin(); }
                        },
                        _ => {}
                    }
                }
                let _ = TranslateMessage(&msg);
                let _ = DispatchMessageW(&msg);
            }
            if msg.message == WM_QUIT { break; }
            state.step_tiling_transition();
            state.poll_duplicate();
            state.poll_closed();
            if state.is_selecting {
                let alt_pressed = (GetAsyncKeyState(VK_MENU.0 as i32) as u16 & 0x8000) != 0;
                let w_pressed = (GetAsyncKeyState('W' as i32) as u16 & 0x8000) != 0;
                if !w_pressed { state.w_released = true; }
                if state.w_released && !alt_pressed {
                    state.end_selection();
                }
            }
            if state.spin_flag {
                let now = Instant::now();
                if now >= next_frame {
                    state.tick();
                    if state.spin_flag {
                        let _ = InvalidateRect(Some(state.overlay_hwnd), None, false);
                        let _ = UpdateWindow(state.overlay_hwnd);
                    }
                    next_frame += frame;
                    let after = Instant::now();
                    if next_frame < after { next_frame = after; }
                } else {
                    std::thread::sleep(Duration::from_millis(1));
                }
            } else {
                if state.is_selecting || state.transition.is_some() || state.pending_duplicate.is_some() || state.pending_close.is_some() {
                    std::thread::sleep(Duration::from_millis(10));
                } else {
                    let _ = WaitMessage();
                }
                next_frame = Instant::now();
            }
        }
    }
}