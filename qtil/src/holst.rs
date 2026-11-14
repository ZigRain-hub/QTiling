// holst.rs — бесконечный холст, встроенный в Q-Tiler.
//
// Alt+C  — открыть / закрыть холст (регистрируется в main.rs, id 12)
// Alt+V  — снять выбор с окна (бывший Ctrl+Alt+W), регистрируется только пока холст открыт
// Alt+= / Alt+-        — масштаб
// Alt + колесо         — масштаб под курсором
// Alt + ПКМ + движение — перемещение всего пространства
// ЛКМ + движение       — перемещение окна по холсту
// короткий клик по окну — «живой режим»: настоящее окно приложения выезжает ровно на место
//                         превью (с текущим масштабом) и работает полностью нативно:
//                         клики, drag, выделение текста, меню, ввод с клавиатуры, колесо.
//
// Холст использует windows-sys (как и раньше), остальной проект — крейт windows.
// Типы не смешиваются: наружу торчат только usize-хэндлы.

use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    mem::{size_of, zeroed},
    process::{Child, ChildStdin, Command, Stdio},
    ptr::{null, null_mut},
    sync::mpsc,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM},
    Graphics::{Dwm::*, Gdi::*},
    Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY},
    System::{
        LibraryLoader::GetModuleHandleW,
        Threading::{
            GetCurrentProcessId, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW,
            PROCESS_QUERY_LIMITED_INFORMATION,
        },
    },
    UI::{HiDpi::SetProcessDpiAwarenessContext, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

// Глобальные хоткеи холста (id >= 200, чтобы не пересекаться с хоткеями тайлинга 1..=12)
const ZOOM_IN: i32 = 201;
const ZOOM_OUT: i32 = 202;
const UNFOCUS: i32 = 203;

const ANIM_TIMER: usize = 2;
const ACTIVATE_TIMER: usize = 5;
const LIVE_TIMER: usize = 6;

const MIN_ZOOM: f64 = 0.25;
const MAX_ZOOM: f64 = 2.0;

const DRAG_MOVE: u32 = WM_APP + 1;
const DRAG_UP: u32 = WM_APP + 2;
const ZOOM_WHEEL: u32 = WM_APP + 3;
const PAN_DOWN: u32 = WM_APP + 5;

const TOPMOST: HWND = -1isize as HWND;
const NOTOPMOST: HWND = -2isize as HWND;

#[link(name = "winmm")]
unsafe extern "system" {
    fn timeBeginPeriod(period: u32) -> u32;
    fn timeEndPeriod(period: u32) -> u32;
}

static mut HOOK_HOST: HWND = null_mut();
static mut DRAG_HOOK: HHOOK = null_mut();
static mut PAN_HOOK: bool = false;

// ------------------------------------------------------------------ мышиный хук
// Перехватывает только Alt+ПКМ (панорама) и Alt+колесо (масштаб).
// Всё остальное уходит приложениям как есть — поэтому живое окно работает нативно.
unsafe extern "system" fn drag_hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code >= 0 && !HOOK_HOST.is_null() && IsWindowVisible(HOOK_HOST) != 0 {
        let event = &*(lp as *const MSLLHOOKSTRUCT);
        if event.flags & LLMHF_INJECTED == 0 {
            let mut pt = event.pt;
            ScreenToClient(HOOK_HOST, &mut pt);
            let what = wp as u32;
            let alt = (GetAsyncKeyState(VK_MENU as i32) as u16 & 0x8000) != 0;
            if PAN_HOOK {
                if what == WM_MOUSEMOVE {
                    PostMessageW(HOOK_HOST, DRAG_MOVE, 0, packed(pt.x, pt.y));
                }
                if what == WM_RBUTTONUP {
                    PAN_HOOK = false;
                    PostMessageW(HOOK_HOST, DRAG_UP, 0, packed(pt.x, pt.y));
                    return 1;
                }
            } else if what == WM_RBUTTONDOWN && alt {
                if PostMessageW(HOOK_HOST, PAN_DOWN, 0, packed(pt.x, pt.y)) != 0 {
                    PAN_HOOK = true;
                    return 1;
                }
            } else if what == WM_MOUSEWHEEL && alt {
                if PostMessageW(HOOK_HOST, ZOOM_WHEEL, event.mouseData as usize, packed(pt.x, pt.y)) != 0 {
                    return 1;
                }
            }
        }
    }
    CallNextHookEx(null_mut(), code, wp, lp)
}

// ------------------------------------------------------------------ хелперы
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
unsafe fn alert(s: &str) {
    MessageBoxW(null_mut(), wide(s).as_ptr(), wide("Q-Tiler — холст").as_ptr(), MB_OK | MB_ICONINFORMATION);
}
fn xy(lp: LPARAM) -> (i32, i32) {
    ((lp as u16 as i16) as i32, ((lp >> 16) as u16 as i16) as i32)
}
fn packed(x: i32, y: i32) -> isize {
    ((x as u16 as u32) | ((y as u16 as u32) << 16)) as isize
}
fn pos(v: f64) -> i32 {
    v.round().clamp(-30000., 30000.) as i32
}
fn input_log(message: &str) {
    use std::fs::OpenOptions;
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(std::env::temp_dir().join("holst-input.log")) {
        let _ = writeln!(f, "{}", message);
    }
}

unsafe fn eligible_process(pid: u32) -> bool {
    if pid == 0 || pid == GetCurrentProcessId() {
        return false;
    }
    let proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
    if proc.is_null() {
        return false;
    }
    let mut path = [0u16; 1024];
    let mut len = path.len() as u32;
    let ok = QueryFullProcessImageNameW(proc, 0, path.as_mut_ptr(), &mut len);
    let mut token = null_mut();
    let ok_token = OpenProcessToken(proc, TOKEN_QUERY, &mut token);
    CloseHandle(proc);
    if ok == 0 || ok_token == 0 {
        if ok_token != 0 {
            CloseHandle(token);
        }
        return false;
    }
    let name = String::from_utf16_lossy(&path[..len as usize]).to_lowercase();
    let exe = name.rsplit(['\\', '/']).next().unwrap_or("");
    let mut elev: TOKEN_ELEVATION = zeroed();
    let mut n = 0;
    let ok = GetTokenInformation(token, TokenElevation, &mut elev as *mut _ as _, size_of::<TOKEN_ELEVATION>() as u32, &mut n);
    CloseHandle(token);
    // окна от администратора мы не можем двигать — пропускаем
    if ok == 0 || elev.TokenIsElevated != 0 {
        return false;
    }
    ![
        "taskmgr.exe", "dwm.exe", "searchhost.exe", "startmenuexperiencehost.exe", "shellexperiencehost.exe",
        "textinputhost.exe", "lockapp.exe", "systemsettings.exe", "securityhealthsystray.exe",
    ]
    .contains(&exe)
}

unsafe extern "system" fn enumerate(h: HWND, l: LPARAM) -> i32 {
    if IsWindowVisible(h) == 0
        || IsIconic(h) != 0
        || IsZoomed(h) != 0
        || GetWindow(h, GW_OWNER) != null_mut()
        || GetWindowLongW(h, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW != 0
        || GetWindowTextLengthW(h) <= 0
    {
        return 1;
    }
    let mut name = [0u16; 256];
    let n = GetClassNameW(h, name.as_mut_ptr(), 256);
    let class = String::from_utf16_lossy(&name[..n.max(0) as usize]);
    if ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd", "TaskManagerWindow"]
        .iter()
        .any(|v| class.eq_ignore_ascii_case(v))
    {
        return 1;
    }
    let mut cloaked = 0u32;
    if DwmGetWindowAttribute(h, DWMWA_CLOAKED as u32, &mut cloaked as *mut _ as _, size_of::<u32>() as u32) >= 0 && cloaked != 0 {
        return 1;
    }
    let mut pid = 0;
    GetWindowThreadProcessId(h, &mut pid);
    if !eligible_process(pid) {
        return 1;
    }
    let mut r: RECT = zeroed();
    if GetWindowRect(h, &mut r) == 0 || r.right <= r.left || r.bottom <= r.top {
        return 1;
    }
    (&mut *(l as *mut Vec<(usize, u32)>)).push((h as usize, pid));
    1
}

// ------------------------------------------------------------------ состояние
struct View {
    source: HWND,
    pid: u32,
    original_x: i32,
    original_y: i32,
    park_x: i32,
    park_y: i32,
    thumb: isize,
    src: SIZE,             // размер окна в «мировых» координатах (при zoom=1 это пиксели)
    x: f64,
    y: f64,
    target_x: f64,
    target_y: f64,
    last_dst: Option<(i32, i32, i32, i32, bool, u8)>,
    live: bool,            // настоящее окно сейчас на экране поверх превью
    live_rect: (i32, i32, i32, i32),
    live_stamp: Instant,
}

enum Drag {
    Window { index: usize, start: (i32, i32), origin: (f64, f64), moved: bool },
    Pan { start: (i32, i32), origin: (f64, f64) },
}

struct State {
    views: Vec<View>,
    zoom: f64,
    pan_x: f64,
    pan_y: f64,
    w: i32,
    h: i32,
    drag: Option<Drag>,
    frame: HBITMAP,
    region_cache: Option<Vec<(usize, i32, i32, i32, i32)>>,
    target_zoom: f64,
    target_pan_x: f64,
    target_pan_y: f64,
    animating: bool,
    last_tick: Instant,
    period_active: bool,
    guard: Option<(Child, ChildStdin)>,
    fade_start: Option<Instant>,
    fade_out: bool,
    opacity: u8,
    selected: Option<(usize, u32)>,
    pending_activation: Option<(usize, u32, Instant)>,
}

impl State {
    fn new(w: i32, h: i32) -> Self {
        Self {
            views: Vec::new(),
            zoom: 1.,
            pan_x: 0.,
            pan_y: 0.,
            w,
            h,
            drag: None,
            frame: null_mut(),
            region_cache: None,
            target_zoom: 1.,
            target_pan_x: 0.,
            target_pan_y: 0.,
            animating: false,
            last_tick: Instant::now(),
            period_active: false,
            guard: None,
            fade_start: None,
            fade_out: false,
            opacity: 255,
            selected: None,
            pending_activation: None,
        }
    }

    fn rect(&self, v: &View) -> RECT {
        RECT {
            left: pos(self.pan_x + v.x * self.zoom),
            top: pos(self.pan_y + v.y * self.zoom),
            right: pos(self.pan_x + (v.x + v.src.cx as f64) * self.zoom),
            bottom: pos(self.pan_y + (v.y + v.src.cy as f64) * self.zoom),
        }
    }

    fn hit(&self, x: i32, y: i32) -> Option<usize> {
        self.views.iter().enumerate().rev().find_map(|(i, v)| {
            if v.live {
                return None; // живое окно получает мышь само
            }
            let r = self.rect(v);
            (x >= r.left && x < r.right && y >= r.top && y < r.bottom).then_some(i)
        })
    }

    unsafe fn clear(&mut self) {
        for v in self.views.drain(..) {
            if v.thumb != 0 {
                DwmUnregisterThumbnail(v.thumb);
            }
        }
        self.drag = None;
        self.region_cache = None;
    }

    unsafe fn init_buffers(&mut self, host: HWND) {
        let dc = GetDC(host);
        if !dc.is_null() {
            self.frame = CreateCompatibleBitmap(dc, self.w, self.h);
            ReleaseDC(host, dc);
        }
    }

    unsafe fn paint_scene(&self, dc: HDC) {
        let r = RECT { left: 0, top: 0, right: self.w, bottom: self.h };
        let brush = CreateSolidBrush(0x24201c);
        FillRect(dc, &r, brush);
        DeleteObject(brush as _);
        for v in &self.views {
            if v.thumb == 0 && !v.live {
                let r = self.rect(v);
                let brush = CreateSolidBrush(0x343434);
                FillRect(dc, &r, brush);
                DeleteObject(brush as _);
            }
        }
    }

    // Окно холста «вырезается» по прямоугольникам превью (кроме живого окна — оно настоящее).
    unsafe fn apply_region(&self, host: HWND, key: &[(usize, i32, i32, i32, i32)]) -> bool {
        let union = CreateRectRgn(0, 0, 0, 0);
        if union.is_null() {
            return false;
        }
        for &(_, l, t, r, b) in key {
            let part = CreateRectRgn(l, t, r, b);
            if !part.is_null() {
                CombineRgn(union, union, part, RGN_OR);
                DeleteObject(part as _);
            }
        }
        if SetWindowRgn(host, union, 1) == 0 {
            DeleteObject(union as _);
            false
        } else {
            true
        }
    }

    unsafe fn update(&mut self, host: HWND) {
        let _ = SetLayeredWindowAttributes(host, 0, self.opacity, LWA_ALPHA);
        let rects: Vec<RECT> = self.views.iter().map(|v| self.rect(v)).collect();

        // область окна холста
        let key: Vec<(usize, i32, i32, i32, i32)> = self
            .views
            .iter()
            .zip(rects.iter())
            .filter_map(|(v, r)| {
                if v.live || IsWindow(v.source) == 0 {
                    return None;
                }
                let l = r.left.clamp(0, self.w);
                let t = r.top.clamp(0, self.h);
                let rr = r.right.clamp(0, self.w);
                let b = r.bottom.clamp(0, self.h);
                (rr > l && b > t).then_some((v.source as usize, l, t, rr, b))
            })
            .collect();
        if self.region_cache.as_ref() != Some(&key) {
            if self.apply_region(host, &key) {
                self.region_cache = Some(key);
            }
        }

        // DWM-превью (у живого окна превью скрыто)
        let opacity = self.opacity;
        let mut missing = false;
        for (i, r) in rects.iter().enumerate() {
            let v = &mut self.views[i];
            if v.thumb == 0 {
                missing = true;
                continue;
            }
            let show = IsWindow(v.source) != 0 && !v.live;
            let k = if show { (r.left, r.top, r.right, r.bottom, true, opacity) } else { (0, 0, 0, 0, false, opacity) };
            if v.last_dst == Some(k) {
                continue;
            }
            let prop = DWM_THUMBNAIL_PROPERTIES {
                dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE | DWM_TNP_OPACITY,
                rcDestination: *r,
                rcSource: zeroed(),
                opacity,
                fVisible: show as i32,
                fSourceClientAreaOnly: 0,
            };
            if DwmUpdateThumbnailProperties(v.thumb, &prop) >= 0 {
                v.last_dst = Some(k);
            }
        }
        if missing {
            InvalidateRect(host, null(), 0);
        }

        // живое окно следует за превью (пан / зум / перетаскивание)
        for (i, r) in rects.iter().enumerate() {
            let v = &mut self.views[i];
            if !v.live || IsWindow(v.source) == 0 {
                continue;
            }
            let target = (r.left, r.top, r.right, r.bottom);
            if target != v.live_rect {
                SetWindowPos(
                    v.source, TOPMOST, r.left, r.top, (r.right - r.left).max(1), (r.bottom - r.top).max(1),
                    SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
                );
                v.live_rect = target;
                v.live_stamp = Instant::now();
            }
        }
    }

    unsafe fn collect(&mut self, host: HWND) -> usize {
        let saved: HashMap<(usize, u32), (f64, f64)> =
            self.views.iter().map(|v| ((v.source as usize, v.pid), (v.x, v.y))).collect();
        self.clear();
        let mut found: Vec<(usize, u32)> = Vec::new();
        EnumWindows(Some(enumerate), &mut found as *mut _ as LPARAM);
        found.reverse();
        let park_x = GetSystemMetrics(SM_XVIRTUALSCREEN)
            .saturating_add(GetSystemMetrics(SM_CXVIRTUALSCREEN))
            .saturating_add(256);
        let park_y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        if park_x > 28000 || park_x < -28000 || park_y > 28000 || park_y < -28000 {
            return 0;
        }
        for (key, pid) in found.into_iter().take(64) {
            let source = key as HWND;
            let mut rect: RECT = zeroed();
            if GetWindowRect(source, &mut rect) == 0 {
                continue;
            }
            let mut thumb = 0isize;
            if DwmRegisterThumbnail(host, source, &mut thumb) < 0 {
                continue;
            }
            let mut src = SIZE { cx: 0, cy: 0 };
            if DwmQueryThumbnailSourceSize(thumb, &mut src) < 0 || src.cx <= 0 || src.cy <= 0 || src.cx > 8192 || src.cy > 8192 {
                DwmUnregisterThumbnail(thumb);
                continue;
            }
            let (x, y) = saved
                .get(&(key, pid))
                .copied()
                .unwrap_or(((rect.left as f64 - self.pan_x) / self.zoom, (rect.top as f64 - self.pan_y) / self.zoom));
            self.views.push(View {
                source, pid, original_x: rect.left, original_y: rect.top, park_x, park_y, thumb, src,
                x, y, target_x: x, target_y: y, last_dst: None,
                live: false, live_rect: (0, 0, 0, 0), live_stamp: Instant::now(),
            });
        }
        self.update(host);
        self.views.len()
    }

    // Сначала запускаем отдельный процесс-«страховку», и только потом трогаем окна.
    unsafe fn park_windows(&mut self) -> bool {
        let exe = match std::env::current_exe() {
            Ok(v) => v,
            Err(_) => return false,
        };
        let mut child = match Command::new(exe)
            .arg("--holst-watchdog")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => return false,
        };
        let Some(mut input) = child.stdin.take() else {
            let _ = child.kill();
            return false;
        };
        let Some(output) = child.stdout.take() else {
            let _ = child.kill();
            return false;
        };
        for v in &self.views {
            if writeln!(input, "{} {} {} {} {} {}", v.source as usize, v.pid, v.original_x, v.original_y, v.park_x, v.park_y).is_err() {
                let _ = child.kill();
                return false;
            }
        }
        if writeln!(input, "END").is_err() || input.flush().is_err() {
            let _ = child.kill();
            return false;
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let _ = BufReader::new(output).read_line(&mut line);
            let _ = tx.send(line);
        });
        if !matches!(rx.recv_timeout(Duration::from_secs(3)), Ok(ref line) if line.trim() == "READY") {
            let _ = child.kill();
            let _ = child.wait();
            return false;
        }
        self.guard = Some((child, input));
        for v in &self.views {
            let _ = SetWindowPos(v.source, null_mut(), v.park_x, v.park_y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS);
        }
        true
    }

    unsafe fn restore_windows(&mut self) {
        for v in &mut self.views {
            if v.live {
                // живое окно на экране — возвращаем на исходное место (с текущим размером)
                if IsWindow(v.source) != 0 {
                    SetWindowPos(
                        v.source, NOTOPMOST, v.original_x, v.original_y, v.src.cx.max(1), v.src.cy.max(1),
                        SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
                    );
                }
                v.live = false;
                v.last_dst = None;
            } else {
                restore_one(v.source as usize, v.pid, v.original_x, v.original_y, v.park_x, v.park_y);
            }
        }
        // EOF у помощника = ещё одно восстановление (в том числе при аварийном выходе)
        if let Some((mut child, input)) = self.guard.take() {
            drop(input);
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }

    // ---------------------------------------------------------------- живой режим
    // Клик по превью: настоящее окно выезжает на место превью и получает фокус.
    unsafe fn select(&mut self, host: HWND, index: usize) {
        if self.selected.is_some() || self.pending_activation.is_some() {
            return;
        }
        let Some(v) = self.views.get(index) else { return };
        let (source, expected_pid) = (v.source, v.pid);
        if IsWindow(source) == 0 {
            input_log("activation failed: window closed");
            return;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(source, &mut pid);
        if pid != expected_pid {
            input_log("activation failed: window identity changed");
            return;
        }
        // окно могли развернуть на весь экран — вернём обычное состояние
        if IsZoomed(source) != 0 {
            ShowWindow(source, SW_RESTORE);
        }
        let r = self.rect(&self.views[index]);
        {
            let v = &mut self.views[index];
            v.live = true;
            v.live_rect = (r.left, r.top, r.right, r.bottom);
            v.live_stamp = Instant::now();
            v.last_dst = None;
        }
        SetWindowPos(
            source, TOPMOST, r.left, r.top, (r.right - r.left).max(1), (r.bottom - r.top).max(1),
            SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
        );
        self.pending_activation = Some((source as usize, pid, Instant::now()));
        if SetTimer(host, ACTIVATE_TIMER, 40, None) == 0 {
            self.pending_activation = None;
            self.park_back(source as usize);
            input_log("activation failed: timer unavailable");
            return;
        }
        SetForegroundWindow(source);
        self.update(host);
    }

    unsafe fn poll_activation(&mut self, host: HWND) {
        let Some((raw, pid, started)) = self.pending_activation else {
            KillTimer(host, ACTIVATE_TIMER);
            return;
        };
        let source = raw as HWND;
        let mut actual_pid = 0;
        GetWindowThreadProcessId(source, &mut actual_pid);
        if IsWindowVisible(host) == 0 || IsWindow(source) == 0 || pid != actual_pid {
            self.unselect(host);
            input_log("activation canceled: window no longer available");
            return;
        }
        if GetForegroundWindow() == source {
            self.pending_activation = None;
            KillTimer(host, ACTIVATE_TIMER);
            self.selected = Some((raw, pid));
            SetTimer(host, LIVE_TIMER, 60, None);
            input_log("activation: foreground confirmed");
            return;
        }
        if started.elapsed() >= Duration::from_millis(1200) {
            self.unselect(host);
            input_log("activation failed: foreground not confirmed within 1200 ms");
            alert("Окно не стало активным. Проверьте, что приложение запущено без прав администратора.");
        }
    }

    // Следим за живым окном: закрыто ли оно, не подвинул/не растянул ли его пользователь сам.
    unsafe fn poll_live(&mut self, host: HWND) {
        let Some((raw, pid)) = self.selected else {
            KillTimer(host, LIVE_TIMER);
            return;
        };
        let source = raw as HWND;
        if IsWindow(source) == 0 {
            self.unselect(host);
            return;
        }
        let Some(i) = self.views.iter().position(|v| v.source == source && v.pid == pid) else { return };
        if !self.views[i].live || self.views[i].live_stamp.elapsed() < Duration::from_millis(300) {
            return;
        }
        if IsIconic(source) != 0 {
            return;
        }
        if IsZoomed(source) != 0 {
            // развернули на весь экран — возвращаем в рамку превью
            ShowWindow(source, SW_RESTORE);
            self.views[i].live_rect = (0, 0, 0, 0);
            self.update(host);
            return;
        }
        let mut r: RECT = zeroed();
        if GetWindowRect(source, &mut r) == 0 {
            return;
        }
        let cur = (r.left, r.top, r.right, r.bottom);
        if cur == self.views[i].live_rect {
            return;
        }
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        if w < 50 || h < 50 {
            return;
        }
        // пользователь сам переместил/растянул окно — холст подстраивается под него
        let z = self.zoom;
        let (px, py) = (self.pan_x, self.pan_y);
        let v = &mut self.views[i];
        v.x = (r.left as f64 - px) / z;
        v.y = (r.top as f64 - py) / z;
        v.target_x = v.x;
        v.target_y = v.y;
        v.src = SIZE { cx: ((w as f64 / z).round() as i32).clamp(1, 8192), cy: ((h as f64 / z).round() as i32).clamp(1, 8192) };
        v.live_rect = cur;
        v.live_stamp = Instant::now();
        self.update(host);
    }

    // Убираем окно с экрана обратно «за кулисы» (на парковку).
    unsafe fn park_back(&mut self, raw: usize) {
        for v in self.views.iter_mut() {
            if v.source as usize == raw && v.live {
                if IsWindow(v.source) != 0 {
                    SetWindowPos(
                        v.source, NOTOPMOST, v.park_x, v.park_y, v.src.cx.max(1), v.src.cy.max(1),
                        SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
                    );
                }
                v.live = false;
                v.last_dst = None;
            }
        }
    }

    unsafe fn unselect(&mut self, host: HWND) {
        let had_pending = self.pending_activation.take().is_some();
        KillTimer(host, ACTIVATE_TIMER);
        KillTimer(host, LIVE_TIMER);
        let was_selected = self.selected.take().is_some();
        let raws: Vec<usize> = self.views.iter().filter(|v| v.live).map(|v| v.source as usize).collect();
        for raw in raws {
            self.park_back(raw);
        }
        if GetCapture() == host {
            ReleaseCapture();
        }
        if (was_selected || had_pending) && IsWindowVisible(host) != 0 {
            self.update(host);
            let _ = SetForegroundWindow(host);
        }
    }

    // Raise: пере-регистрируем превью, чтобы оно оказалось сверху остальных.
    unsafe fn raise(&mut self, host: HWND, index: usize) -> usize {
        if index >= self.views.len() || index + 1 == self.views.len() {
            return index;
        }
        let mut v = self.views.remove(index);
        let mut new_thumb = 0isize;
        if DwmRegisterThumbnail(host, v.source, &mut new_thumb) >= 0 {
            if v.thumb != 0 {
                DwmUnregisterThumbnail(v.thumb);
            }
            v.thumb = new_thumb;
        }
        v.last_dst = None;
        self.views.push(v);
        self.update(host);
        self.views.len() - 1
    }

    // ---------------------------------------------------------------- анимация
    unsafe fn stop_animation(&mut self, host: HWND) {
        KillTimer(host, ANIM_TIMER);
        self.animating = false;
        if self.period_active {
            let _ = timeEndPeriod(1);
            self.period_active = false;
        }
        self.target_zoom = self.zoom;
        self.target_pan_x = self.pan_x;
        self.target_pan_y = self.pan_y;
        for v in &mut self.views {
            v.target_x = v.x;
            v.target_y = v.y;
        }
    }

    unsafe fn release_keys(&self, host: HWND) {
        UnregisterHotKey(host, ZOOM_IN);
        UnregisterHotKey(host, ZOOM_OUT);
        UnregisterHotKey(host, UNFOCUS);
    }

    unsafe fn remove_hook() {
        if !DRAG_HOOK.is_null() {
            UnhookWindowsHookEx(DRAG_HOOK);
            DRAG_HOOK = null_mut();
        }
        HOOK_HOST = null_mut();
        PAN_HOOK = false;
    }

    // Полное закрытие: прячем окно холста, возвращаем все приложения на место.
    unsafe fn suspend(&mut self, host: HWND) {
        self.restore_windows();
        self.unselect(host);
        ShowWindow(host, SW_HIDE);
        self.release_keys(host);
        self.fade_start = None;
        self.fade_out = false;
        Self::remove_hook();
        self.stop_animation(host);
        self.drag = None;
        if GetCapture() == host {
            ReleaseCapture();
        }
        for v in &mut self.views {
            if v.thumb != 0 {
                DwmUnregisterThumbnail(v.thumb);
                v.thumb = 0;
                v.last_dst = None;
            }
        }
    }

    unsafe fn begin_animation(&mut self, host: HWND) {
        if self.animating {
            return;
        }
        self.animating = true;
        self.last_tick = Instant::now();
        self.period_active = timeBeginPeriod(1) == 0;
        if SetTimer(host, ANIM_TIMER, 10, None) == 0 {
            self.animating = false;
            if self.period_active {
                let _ = timeEndPeriod(1);
                self.period_active = false;
            }
            self.zoom = self.target_zoom;
            self.pan_x = self.target_pan_x;
            self.pan_y = self.target_pan_y;
            for v in &mut self.views {
                v.x = v.target_x;
                v.y = v.target_y;
            }
            if self.fade_out {
                self.suspend(host);
            } else {
                self.fade_start = None;
                self.opacity = 255;
                self.update(host);
            }
        }
    }

    unsafe fn zoom_at(&mut self, host: HWND, p: POINT, factor: f64) {
        let old = self.target_zoom;
        let next = (old * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let x = (p.x as f64 - self.target_pan_x) / old;
        let y = (p.y as f64 - self.target_pan_y) / old;
        self.target_zoom = next;
        self.target_pan_x = p.x as f64 - x * next;
        self.target_pan_y = p.y as f64 - y * next;
        self.begin_animation(host);
    }

    unsafe fn zoom_at_cursor(&mut self, factor: f64, host: HWND) {
        let mut p: POINT = zeroed();
        if GetCursorPos(&mut p) == 0 {
            p = POINT { x: self.w / 2, y: self.h / 2 };
        }
        ScreenToClient(host, &mut p);
        self.zoom_at(host, p, factor);
    }

    unsafe fn tick(&mut self, host: HWND) {
        if let Some(start) = self.fade_start {
            let t = (start.elapsed().as_secs_f64() / 0.25).clamp(0., 1.);
            self.opacity = if self.fade_out { ((1. - t) * 255.) as u8 } else { (t * 255.) as u8 };
            if t >= 1.0 {
                self.fade_start = None;
                if self.fade_out {
                    self.suspend(host);
                    return;
                }
            }
        }
        if IsWindowVisible(host) == 0 {
            self.stop_animation(host);
            return;
        }
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f64().clamp(0.001, 0.1);
        self.last_tick = now;
        let a = 1.0 - (-dt / 0.09).exp();
        self.zoom += (self.target_zoom - self.zoom) * a;
        self.pan_x += (self.target_pan_x - self.pan_x) * a;
        self.pan_y += (self.target_pan_y - self.pan_y) * a;
        let mut moving = (self.zoom - self.target_zoom).abs() > 0.0001
            || (self.pan_x - self.target_pan_x).abs() > 0.1
            || (self.pan_y - self.target_pan_y).abs() > 0.1;
        for v in &mut self.views {
            v.x += (v.target_x - v.x) * a;
            v.y += (v.target_y - v.y) * a;
            moving |= (v.x - v.target_x).abs() > 0.1 || (v.y - v.target_y).abs() > 0.1;
        }
        moving |= self.fade_start.is_some();
        if !moving {
            self.zoom = self.target_zoom;
            self.pan_x = self.target_pan_x;
            self.pan_y = self.target_pan_y;
            for v in &mut self.views {
                v.x = v.target_x;
                v.y = v.target_y;
            }
            self.animating = false;
            KillTimer(host, ANIM_TIMER);
            if self.period_active {
                let _ = timeEndPeriod(1);
                self.period_active = false;
            }
        }
        self.update(host);
    }

    // ---------------------------------------------------------------- открыть / закрыть
    unsafe fn open(&mut self, host: HWND) {
        if RegisterHotKey(host, ZOOM_IN, MOD_ALT | MOD_NOREPEAT, VK_OEM_PLUS as u32) == 0 {
            alert("Alt+= уже занят другой программой. Холст не открыт.");
            return;
        }
        if RegisterHotKey(host, ZOOM_OUT, MOD_ALT | MOD_NOREPEAT, VK_OEM_MINUS as u32) == 0 {
            self.release_keys(host);
            alert("Alt+- уже занят другой программой. Холст не открыт.");
            return;
        }
        if RegisterHotKey(host, UNFOCUS, MOD_ALT | MOD_NOREPEAT, b'V' as u32) == 0 {
            self.release_keys(host);
            alert("Alt+V уже занят другой программой. Холст не открыт.");
            return;
        }
        let n = self.collect(host);
        if n == 0 {
            self.release_keys(host);
            alert("Подходящих открытых окон не найдено. Проверьте, что приложения не свёрнуты и не развёрнуты на весь экран, и запущены без прав администратора.");
            return;
        }
        if !self.park_windows() {
            self.clear();
            self.release_keys(host);
            alert("Не удалось запустить восстановление окон. Холст не открыт.");
            return;
        }
        HOOK_HOST = host;
        DRAG_HOOK = SetWindowsHookExW(WH_MOUSE_LL, Some(drag_hook), GetModuleHandleW(null()), 0);
        if DRAG_HOOK.is_null() {
            HOOK_HOST = null_mut();
            self.suspend(host);
            alert("Не удалось включить управление холстом.");
            return;
        }
        self.opacity = 0;
        self.fade_out = false;
        self.fade_start = Some(Instant::now());
        ShowWindow(host, SW_SHOW);
        let _ = SetForegroundWindow(host);
        self.update(host);
        self.begin_animation(host);
    }

    unsafe fn begin_close(&mut self, host: HWND) {
        Self::remove_hook();
        self.drag = None;
        if GetCapture() == host {
            ReleaseCapture();
        }
        self.restore_windows();
        self.unselect(host);
        self.fade_out = true;
        self.fade_start = Some(Instant::now());
        self.begin_animation(host);
    }
}

// ------------------------------------------------------------------ оконная процедура
unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
    if p.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let st = &mut *p;
    if st.fade_out && matches!(msg, WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONDOWN | WM_RBUTTONUP | WM_MOUSEMOVE | DRAG_MOVE | DRAG_UP | PAN_DOWN | ZOOM_WHEEL) {
        return 0;
    }
    match msg {
        WM_HOTKEY => {
            if IsWindowVisible(hwnd) != 0 {
                match wp as i32 {
                    ZOOM_IN => st.zoom_at_cursor(1.15, hwnd),
                    ZOOM_OUT => st.zoom_at_cursor(1. / 1.15, hwnd),
                    UNFOCUS => st.unselect(hwnd),
                    _ => {}
                }
            }
            0
        }
        WM_TIMER => {
            match wp {
                ACTIVATE_TIMER => st.poll_activation(hwnd),
                LIVE_TIMER => st.poll_live(hwnd),
                ANIM_TIMER => {
                    if st.animating {
                        st.tick(hwnd);
                    }
                }
                _ => {}
            }
            0
        }
        ZOOM_WHEEL => {
            if IsWindowVisible(hwnd) == 0 {
                return 0;
            }
            let (x, y) = xy(lp);
            let delta = ((wp >> 16) as u16 as i16) as f64;
            st.zoom_at(hwnd, POINT { x, y }, 1.12f64.powf(delta / 120.));
            0
        }
        WM_MOUSEACTIVATE => MA_NOACTIVATE as isize,
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            if !dc.is_null() {
                let mem = CreateCompatibleDC(dc);
                if !mem.is_null() && !st.frame.is_null() {
                    let old = SelectObject(mem, st.frame as _);
                    st.paint_scene(mem);
                    BitBlt(dc, 0, 0, st.w, st.h, mem, 0, 0, SRCCOPY);
                    SelectObject(mem, old);
                } else {
                    st.paint_scene(dc);
                }
                if !mem.is_null() {
                    DeleteDC(mem);
                }
            }
            EndPaint(hwnd, &ps);
            0
        }
        PAN_DOWN => {
            if st.drag.is_some() {
                return 0;
            }
            let (x, y) = xy(lp);
            st.drag = Some(Drag::Pan { start: (x, y), origin: (st.target_pan_x, st.target_pan_y) });
            0
        }
        WM_RBUTTONDOWN => {
            if st.drag.is_some() {
                return 0;
            }
            let (x, y) = xy(lp);
            if (GetAsyncKeyState(VK_MENU as i32) as u16 & 0x8000) != 0 {
                st.drag = Some(Drag::Pan { start: (x, y), origin: (st.target_pan_x, st.target_pan_y) });
                SetCapture(hwnd);
            }
            0
        }
        WM_LBUTTONDOWN => {
            if st.drag.is_some() || st.pending_activation.is_some() {
                return 0;
            }
            let (x, y) = xy(lp);
            // клик по любому другому превью: отпускаем текущее живое окно и берём новое
            if st.selected.is_some() {
                st.unselect(hwnd);
            }
            if let Some(i) = st.hit(x, y) {
                let i = st.raise(hwnd, i);
                let v = &st.views[i];
                let origin = (v.target_x, v.target_y);
                st.drag = Some(Drag::Window { index: i, start: (x, y), origin, moved: false });
                SetCapture(hwnd);
            }
            0
        }
        WM_MOUSEMOVE | DRAG_MOVE => {
            let (x, y) = xy(lp);
            let zoom = st.zoom;
            if let Some(d) = st.drag.as_mut() {
                match d {
                    Drag::Pan { start, origin } => {
                        st.target_pan_x = (origin.0 + (x - start.0) as f64).clamp(-100000., 100000.);
                        st.target_pan_y = (origin.1 + (y - start.1) as f64).clamp(-100000., 100000.);
                    }
                    Drag::Window { index, start, origin, moved } => {
                        let dx = x - start.0;
                        let dy = y - start.1;
                        if dx.abs() + dy.abs() > 5 {
                            *moved = true;
                        }
                        if *moved {
                            if let Some(v) = st.views.get_mut(*index) {
                                v.target_x = (origin.0 + dx as f64 / zoom).clamp(-100000., 100000.);
                                v.target_y = (origin.1 + dy as f64 / zoom).clamp(-100000., 100000.);
                            }
                        }
                    }
                }
                st.begin_animation(hwnd);
            }
            0
        }
        WM_RBUTTONUP | WM_LBUTTONUP | DRAG_UP => {
            let ends = match st.drag.as_ref() {
                Some(Drag::Window { .. }) => msg == WM_LBUTTONUP,
                Some(Drag::Pan { .. }) => msg == DRAG_UP || msg == WM_RBUTTONUP,
                None => false,
            };
            if ends {
                if let Some(d) = st.drag.take() {
                    if GetCapture() == hwnd {
                        ReleaseCapture();
                    }
                    // короткий клик без перетаскивания — включаем живой режим
                    if let Drag::Window { index, moved: false, .. } = d {
                        st.select(hwnd, index);
                    }
                }
            }
            0
        }
        WM_LBUTTONDBLCLK => 0,
        WM_DESTROY => {
            KillTimer(hwnd, ACTIVATE_TIMER);
            KillTimer(hwnd, LIVE_TIMER);
            State::remove_hook();
            st.stop_animation(hwnd);
            st.release_keys(hwnd);
            st.clear();
            if !st.frame.is_null() {
                DeleteObject(st.frame as _);
            }
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Box::from_raw(p));
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

// ------------------------------------------------------------------ публичный API для main.rs
/// Нужно вызвать самым первым в main(), до создания любых окон: холсту нужны реальные пиксели.
pub fn init_dpi() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(-4isize as _);
    }
}

/// Создаёт скрытое окно холста. Возвращает HWND как usize (0 — ошибка).
pub unsafe fn create() -> usize {
    let instance = GetModuleHandleW(null());
    let class = wide("QtilHolst");
    let mut wc: WNDCLASSW = zeroed();
    wc.lpfnWndProc = Some(proc);
    wc.hInstance = instance;
    wc.lpszClassName = class.as_ptr();
    wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
    if RegisterClassW(&wc) == 0 {
        return 0;
    }
    let w = GetSystemMetrics(SM_CXSCREEN);
    let h = GetSystemMetrics(SM_CYSCREEN);
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
        class.as_ptr(),
        wide("Q-Tiler Holst").as_ptr(),
        WS_POPUP,
        0, 0, w, h,
        null_mut(), null_mut(), instance, null(),
    );
    if hwnd.is_null() {
        return 0;
    }
    let mut state = Box::new(State::new(w, h));
    state.init_buffers(hwnd);
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
    hwnd as usize
}

pub unsafe fn is_open(h: usize) -> bool {
    h != 0 && IsWindowVisible(h as HWND) != 0
}

/// Alt+C: открыть холст, если закрыт; закрыть, если открыт.
pub unsafe fn toggle(h: usize) {
    if h == 0 {
        return;
    }
    let hwnd = h as HWND;
    let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
    if p.is_null() {
        return;
    }
    let st = &mut *p;
    if IsWindowVisible(hwnd) != 0 {
        if !st.fade_out {
            st.begin_close(hwnd);
        }
    } else {
        st.open(hwnd);
    }
}

/// Вызывать при выходе из Q-Tiler: возвращает все окна на места.
pub unsafe fn shutdown(h: usize) {
    if h == 0 {
        return;
    }
    let hwnd = h as HWND;
    let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
    if !p.is_null() {
        (*p).suspend(hwnd);
    }
    DestroyWindow(hwnd);
}

// ------------------------------------------------------------------ процесс-страховка
unsafe fn restore_one(raw: usize, pid: u32, x: i32, y: i32, px: i32, py: i32) {
    let h = raw as HWND;
    if IsWindow(h) == 0 {
        return;
    }
    let mut actual_pid = 0;
    GetWindowThreadProcessId(h, &mut actual_pid);
    if actual_pid != pid {
        return;
    }
    let mut r: RECT = zeroed();
    if GetWindowRect(h, &mut r) == 0 {
        return;
    }
    // не трогаем окна, которые пользователь сам унёс в другое место
    if (r.left == px && r.top == py) || (r.left == x && r.top == y) {
        let _ = SetWindowPos(h, null_mut(), x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS);
    }
}

/// Запускается как `qtil.exe --holst-watchdog`. Ждёт закрытия pipe и возвращает окна на место,
/// даже если основной процесс упал.
pub unsafe fn watchdog() {
    let _ = SetProcessDpiAwarenessContext(-4isize as _);
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let mut lines = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        if line.trim() == "END" {
            break;
        }
        let values: Vec<i64> = line.split_whitespace().filter_map(|p| p.parse().ok()).collect();
        if values.len() != 6 {
            return;
        }
        lines.push(values);
        if lines.len() > 64 {
            return;
        }
    }
    println!("READY");
    let _ = std::io::stdout().flush();
    let mut remaining = String::new();
    let _ = reader.read_to_string(&mut remaining);
    for v in lines {
        restore_one(v[0] as usize, v[1] as u32, v[2] as i32, v[3] as i32, v[4] as i32, v[5] as i32);
    }
}
