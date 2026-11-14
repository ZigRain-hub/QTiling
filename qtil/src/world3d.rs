#![allow(unsafe_op_in_unsafe_fn)]

use std::f32::consts::PI;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::Instant;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Graphics::GdiPlus::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};
use std::result::Result::Ok;
const LIMIT: usize = 9;
const NEAR: f32 = 0.3;
const TIMER: usize = 70;
const RETURN_TO_ROOM: u32 = WM_APP + 41;
static ROOM_FOR_HOOK: AtomicIsize = AtomicIsize::new(0);

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && wparam.0 as u32 == WM_MBUTTONDOWN {
        let raw = ROOM_FOR_HOOK.load(Ordering::SeqCst);
        if raw != 0 && PostMessageW(Some(HWND(raw as *mut _)), RETURN_TO_ROOM, WPARAM(0), LPARAM(0)).is_ok() {
            return LRESULT(1);
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

unsafe fn center_cursor(hwnd: HWND, room: &RoomState) {
    if GetForegroundWindow() != hwnd { return; }
    let mut p = windows::Win32::Foundation::POINT { x: room.width / 2, y: room.height / 2 };
    let _ = ClientToScreen(hwnd, &mut p);
    let _ = SetCursorPos(p.x, p.y);
}

#[derive(Clone, Copy)]
struct Point3 { x: f32, y: f32, z: f32 }

impl Point3 {
    fn mix(self, other: Self, t: f32) -> Self {
        Self { x: self.x + (other.x - self.x) * t,
               y: self.y + (other.y - self.y) * t,
               z: self.z + (other.z - self.z) * t }
    }
}

struct Panel {
    hwnd: HWND,
    pid: u32,
    title: String,
    image: *mut GpBitmap,
    points: [Point3; 4],
}

impl Drop for Panel {
    fn drop(&mut self) {
        if !self.image.is_null() {
            unsafe { let _ = GdipDisposeImage(self.image as *mut GpImage); }
        }
    }
}

struct RenderCache {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    brush: *mut GpSolidFill,
    pen: *mut GpPen,
    font: *mut GpFont,
}

impl RenderCache {
    unsafe fn new(width: i32, height: i32) -> Result<Self, String> {
        let screen = GetDC(None);
        if screen.is_invalid() { return Err("Не удалось получить экран".into()); }
        let dc = CreateCompatibleDC(Some(screen));
        let bitmap = if !dc.is_invalid() { CreateCompatibleBitmap(screen, width, height) } else { HBITMAP(std::ptr::null_mut()) };
        let _ = ReleaseDC(None, screen);
        if dc.is_invalid() { return Err("Не удалось создать буфер 3D-обзора".into()); }
        if bitmap.is_invalid() { let _ = DeleteDC(dc); return Err("Не удалось создать изображение буфера".into()); }
        let old = SelectObject(dc, bitmap.into());
        if old.is_invalid() {
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(dc);
            return Err("Не удалось подключить буфер".into());
        }
        let mut cache = Self { dc, bitmap, old, brush: std::ptr::null_mut(), pen: std::ptr::null_mut(), font: std::ptr::null_mut() };
        let _ = GdipCreateSolidFill(0xFF1A2735, &mut cache.brush);
        let _ = GdipCreatePen1(0xFF34495D, 1.0, UnitPixel, &mut cache.pen);
        if cache.brush.is_null() || cache.pen.is_null() { return Err("Не удалось создать элементы отрисовки".into()); }
        let mut family: *mut GpFontFamily = std::ptr::null_mut();
        let _ = GdipCreateFontFamilyFromName(w!("Segoe UI"), std::ptr::null_mut(), &mut family);
        if !family.is_null() {
            let _ = GdipCreateFont(family, 16.0, 0, UnitPixel, &mut cache.font);
            let _ = GdipDeleteFontFamily(family);
        }
        Ok(cache)
    }
}

impl Drop for RenderCache {
    fn drop(&mut self) {
        unsafe {
            if !self.font.is_null() { let _ = GdipDeleteFont(self.font); }
            if !self.pen.is_null() { let _ = GdipDeletePen(self.pen); }
            if !self.brush.is_null() { let _ = GdipDeleteBrush(self.brush as *mut GpBrush); }
            let _ = SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

struct WindowRestore {
    hwnd: HWND,
    pid: u32,
    placement: WINDOWPLACEMENT,
    was_topmost: bool,
}

struct RoomState {
    panels: Vec<Panel>,
    width: i32,
    height: i32,
    camera_x: f32,
    camera_z: f32,
    yaw: f32,
    pitch: f32,
    old_foreground: HWND,
    hover: Option<usize>,
    drag: Option<usize>,
    interacting: Option<usize>,
    restore: Option<WindowRestore>,
    hook: Option<HHOOK>,
    instance: HINSTANCE,
    last_tick: Instant,
    pending_dx: f32,
    pending_dy: f32,
    cache: RenderCache,
}

impl RoomState {
    fn camera_point(&self, p: Point3) -> Point3 {
        let x = p.x - self.camera_x;
        let z = p.z - self.camera_z;
        let (sin, cos) = self.yaw.sin_cos();
        let rotated_x = x * cos - z * sin;
        let rotated_z = x * sin + z * cos;
        let y = p.y - 1.45;
        let (sin_p, cos_p) = self.pitch.sin_cos();
        Point3 { x: rotated_x, y: y * cos_p - rotated_z * sin_p, z: y * sin_p + rotated_z * cos_p }
    }

    fn project_camera(&self, p: Point3) -> Option<PointF> {
        if p.z <= NEAR { return None; }
        let focal = self.width as f32 * 0.72;
        Some(PointF { X: self.width as f32 * 0.5 + p.x * focal / p.z,
                      Y: self.height as f32 * 0.52 - p.y * focal / p.z })
    }

    fn project(&self, p: Point3) -> Option<PointF> { self.project_camera(self.camera_point(p)) }

    fn projected_panel(&self, panel: &Panel) -> Option<[PointF; 4]> {
        Some([self.project(panel.points[0])?, self.project(panel.points[1])?,
              self.project(panel.points[2])?, self.project(panel.points[3])?])
    }

    fn order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.panels.len()).collect();
        order.sort_by(|&a, &b| {
            let depth = |i: usize| {
                let p = self.panels[i].points;
                self.camera_point(p[0].mix(p[3], 0.5)).z
            };
            depth(b).total_cmp(&depth(a))
        });
        order
    }

    fn center_hit(&self) -> Option<usize> { self.hit(self.width as f32 * 0.5, self.height as f32 * 0.5) }

    fn hit(&self, x: f32, y: f32) -> Option<usize> {
        for i in self.order().into_iter().rev() {
            let Some(p) = self.projected_panel(&self.panels[i]) else { continue; };
            let corners = [p[0], p[1], p[3], p[2]];
            let mut positive = false;
            let mut negative = false;
            for j in 0..4 {
                let a = corners[j];
                let b = corners[(j + 1) % 4];
                let cross = (b.X - a.X) * (y - a.Y) - (b.Y - a.Y) * (x - a.X);
                positive |= cross > 0.0;
                negative |= cross < 0.0;
            }
            if !(positive && negative) { return Some(i); }
        }
        None
    }
}

pub struct Room {
    hwnd: HWND,
    state: *mut RoomState,
}

impl Room {
    pub unsafe fn new(instance: HINSTANCE, width: i32, height: i32) -> Result<Self, String> {
        if width <= 0 || height <= 0 { return Err("Не удалось определить размер экрана".into()); }
        let cache = RenderCache::new(width, height)?;
        let class = w!("QtilRoom3D");
        let wc = WNDCLASSW { lpfnWndProc: Some(room_proc), hInstance: instance, lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(), ..Default::default() };
        let _ = RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST, class, w!("Q-Tiler 3D"), WS_POPUP,
            0, 0, width, height, None, None, Some(instance), None,
        ).map_err(|e| format!("Не удалось создать 3D-окно: {e}"))?;
        let state = Box::into_raw(Box::new(RoomState {
            panels: Vec::new(), width, height, camera_x: 0.0, camera_z: 0.0, yaw: 0.0, pitch: 0.0,
            old_foreground: HWND(std::ptr::null_mut()), hover: None, drag: None,
            interacting: None, restore: None, hook: None, instance, last_tick: Instant::now(),
            pending_dx: 0.0, pending_dy: 0.0, cache,
        }));
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
        Ok(Self { hwnd, state })
    }

    pub unsafe fn is_visible(&self) -> bool { IsWindowVisible(self.hwnd).as_bool() }
    pub unsafe fn is_active(&self) -> bool { self.is_visible() || (*self.state).interacting.is_some() }

    pub unsafe fn open(&mut self, windows: &[HWND]) {
        let state = &mut *self.state;
        state.old_foreground = GetForegroundWindow();
        state.panels.clear();
        state.camera_x = 0.0;
        state.camera_z = 0.0;
        state.yaw = 0.0;
        state.pitch = 0.0;
        state.hover = None;
        state.drag = None;
        state.last_tick = Instant::now();
        state.pending_dx = 0.0;
        state.pending_dy = 0.0;
        for &hwnd in windows.iter().take(LIMIT) {
            let mut rect = RECT::default();
            if GetWindowRect(hwnd, &mut rect).is_err() { continue; }
            if rect.right <= rect.left || rect.bottom <= rect.top { continue; }
            let mut buf = [0u16; 256];
            let len = GetWindowTextW(hwnd, &mut buf).max(0) as usize;
            let title = String::from_utf16_lossy(&buf[..len.min(buf.len())]);
            let mut pid = 0;
            let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
            let image = super::capture_window(hwnd, &rect);
            let points = panel_position(state.panels.len());
            state.panels.push(Panel { hwnd, pid, title, image, points });
        }
        let _ = ShowWindow(self.hwnd, SW_SHOW);
        let _ = SetForegroundWindow(self.hwnd);
        center_cursor(self.hwnd, &*self.state);
        let _ = InvalidateRect(Some(self.hwnd), None, false);
    }

    pub unsafe fn close(&mut self) {
        let was_visible = self.is_visible();
        let old = (*self.state).old_foreground;
        stop_interaction(&mut *self.state);
        (*self.state).drag = None;
        let _ = ShowWindow(self.hwnd, SW_HIDE);
        if was_visible && !old.0.is_null() { let _ = SetForegroundWindow(old); }
    }
}

impl Drop for Room {
    fn drop(&mut self) {
        unsafe {
            stop_interaction(&mut *self.state);
            let _ = ShowWindow(self.hwnd, SW_HIDE);
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            let _ = DestroyWindow(self.hwnd);
            if !self.state.is_null() { drop(Box::from_raw(self.state)); }
        }
    }
}

unsafe fn stop_interaction(room: &mut RoomState) {
    ROOM_FOR_HOOK.store(0, Ordering::SeqCst);
    if let Some(hook) = room.hook.take() { let _ = UnhookWindowsHookEx(hook); }
    if let Some(saved) = room.restore.take() {
        let mut pid = 0;
        let _ = GetWindowThreadProcessId(saved.hwnd, Some(&mut pid));
        if pid != 0 && pid == saved.pid {
            if let Err(error) = SetWindowPlacement(saved.hwnd, &saved.placement) {
                eprintln!("Не удалось вернуть положение окна: {error}");
            }
            let z = if saved.was_topmost { HWND_TOPMOST } else { HWND_NOTOPMOST };
            if let Err(error) = SetWindowPos(saved.hwnd, Some(z), 0, 0, 0, 0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) {
                eprintln!("Не удалось вернуть порядок окна: {error}");
            }
        }
    }
    room.interacting = None;
}

unsafe fn enter_window(hwnd: HWND, room: &mut RoomState, i: usize) {
    let target = room.panels[i].hwnd;
    let mut pid = 0;
    let _ = GetWindowThreadProcessId(target, Some(&mut pid));
    if pid == 0 || pid != room.panels[i].pid { return; }
    let mut placement = WINDOWPLACEMENT { length: std::mem::size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
    if GetWindowPlacement(target, &mut placement).is_err() {
        let _ = MessageBoxW(Some(hwnd), w!("Не удалось получить положение этого окна."), w!("Q-Tiler"), MB_OK | MB_ICONWARNING);
        return;
    }
    let was_topmost = (GetWindowLongW(target, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0) != 0;
    ROOM_FOR_HOOK.store(hwnd.0 as isize, Ordering::SeqCst);
    let hook = match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), Some(room.instance), 0) {
        Ok(hook) => hook,
        Err(_) => {
            ROOM_FOR_HOOK.store(0, Ordering::SeqCst);
            let _ = MessageBoxW(Some(hwnd), w!("Не удалось включить возврат по средней кнопке мыши."), w!("Q-Tiler"), MB_OK | MB_ICONWARNING);
            return;
        }
    };
    room.hook = Some(hook);
    room.interacting = Some(i);
    room.restore = Some(WindowRestore { hwnd: target, pid, placement, was_topmost });
    room.drag = None;
    room.pending_dx = 0.0;
    room.pending_dy = 0.0;
    if IsIconic(target).as_bool() || IsZoomed(target).as_bool() {
        let _ = ShowWindow(target, SW_RESTORE);
    }
    let w = (room.width as f32 * 0.70) as i32;
    let h = (room.height as f32 * 0.70) as i32;
    let x = (room.width - w) / 2;
    let y = (room.height - h) / 2;
    if let Err(error) = SetWindowPos(target, Some(HWND_TOPMOST), x, y, w, h, SWP_SHOWWINDOW) {
        eprintln!("Не удалось поместить окно в обзор: {error}");
        stop_interaction(room);
        let _ = MessageBoxW(Some(hwnd), w!("Не удалось изменить положение окна. Оно может быть защищено Windows."), w!("Q-Tiler"), MB_OK | MB_ICONWARNING);
        return;
    }
    if !SetForegroundWindow(target).as_bool() {
        stop_interaction(room);
        let _ = MessageBoxW(Some(hwnd), w!("Windows не разрешила переключиться на это окно."), w!("Q-Tiler"), MB_OK | MB_ICONWARNING);
        return;
    }
    let _ = InvalidateRect(Some(hwnd), None, false);
}

unsafe fn return_to_room(hwnd: HWND, room: &mut RoomState) {
    stop_interaction(room);
    room.hover = room.center_hit();
    room.last_tick = Instant::now();
    room.pending_dx = 0.0;
    room.pending_dy = 0.0;
    let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
    let _ = SetForegroundWindow(hwnd);
    center_cursor(hwnd, room);
    let _ = InvalidateRect(Some(hwnd), None, false);
}

fn apply_mouse(room: &mut RoomState) -> bool {
    let dx = std::mem::take(&mut room.pending_dx);
    let dy = std::mem::take(&mut room.pending_dy);
    if dx == 0.0 && dy == 0.0 { return false; }
    let yaw_change = dx * 0.0025;
    let old_pitch = room.pitch;
    room.yaw = (room.yaw + yaw_change).rem_euclid(2.0 * PI);
    room.pitch = (room.pitch - dy * 0.0025).clamp(-1.15, 1.15);
    if let Some(i) = room.drag {
        let points = room.panels[i].points;
        let center = points[0].mix(points[3], 0.5);
        let distance = ((center.x - room.camera_x).powi(2) + (center.z - room.camera_z).powi(2)).sqrt().max(0.5);
        let vertical = (room.pitch - old_pitch) * distance;
        let (sin, cos) = yaw_change.sin_cos();
        for point in &mut room.panels[i].points {
            let x = point.x - room.camera_x;
            let z = point.z - room.camera_z;
            point.x = room.camera_x + x * cos + z * sin;
            point.z = room.camera_z - x * sin + z * cos;
            point.y += vertical;
        }
    }
    true
}

unsafe fn update_movement(hwnd: HWND, room: &mut RoomState) {
    let now = Instant::now();
    let dt = now.duration_since(room.last_tick).as_secs_f32().min(0.05);
    room.last_tick = now;
    if GetForegroundWindow() != hwnd || room.interacting.is_some() {
        room.drag = None;
        room.pending_dx = 0.0;
        room.pending_dy = 0.0;
        return;
    }
    let mut changed = apply_mouse(room);
    let down = |key: u8| (GetAsyncKeyState(key as i32) as u16 & 0x8000) != 0;
    let forward = down(b'W') as i32 - down(b'S') as i32;
    let right = down(b'D') as i32 - down(b'A') as i32;
    if forward != 0 || right != 0 {
        let length = ((forward * forward + right * right) as f32).sqrt();
        let f = forward as f32 / length;
        let r = right as f32 / length;
        let (sin, cos) = room.yaw.sin_cos();
        let speed = 2.7 * dt;
        let new_x = (room.camera_x + (f * sin + r * cos) * speed).clamp(-3.5, 3.5);
        let new_z = (room.camera_z + (f * cos - r * sin) * speed).clamp(-2.5, 5.0);
        if let Some(i) = room.drag {
            for point in &mut room.panels[i].points {
                point.x += new_x - room.camera_x;
                point.z += new_z - room.camera_z;
            }
        }
        room.camera_x = new_x;
        room.camera_z = new_z;
        changed = true;
    }
    if changed {
        room.hover = room.center_hit();
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

unsafe fn move_mouse(hwnd: HWND, room: &mut RoomState, x: i32, y: i32) {
    if GetForegroundWindow() != hwnd { return; }
    let dx = (x - room.width / 2) as f32;
    let dy = (y - room.height / 2) as f32;
    if dx == 0.0 && dy == 0.0 { return; }
    room.pending_dx = (room.pending_dx + dx).clamp(-1000.0, 1000.0);
    room.pending_dy = (room.pending_dy + dy).clamp(-1000.0, 1000.0);
    center_cursor(hwnd, room);
}

fn panel_position(i: usize) -> [Point3; 4] {
    let lo = 0.78;
    let hi = 2.35;
    if i < 3 {
        let centers = [0.0, -2.55, 2.55];
        let x = centers[i];
        [Point3 { x: x - 1.18, y: hi, z: 7.5 }, Point3 { x: x + 1.18, y: hi, z: 7.5 },
         Point3 { x: x - 1.18, y: lo, z: 7.5 }, Point3 { x: x + 1.18, y: lo, z: 7.5 }]
    } else {
        let side = if (i - 3) % 2 == 0 { -1.0 } else { 1.0 };
        let row = (i - 3) / 2;
        let z = 3.7 + row as f32 * 2.65;
        let (left, right) = if side < 0.0 { (z - 1.0, z + 1.0) } else { (z + 1.0, z - 1.0) };
        [Point3 { x: 4.2 * side, y: hi, z: left }, Point3 { x: 4.2 * side, y: hi, z: right },
         Point3 { x: 4.2 * side, y: lo, z: left }, Point3 { x: 4.2 * side, y: lo, z: right }]
    }
}

unsafe fn text(g: *mut GpGraphics, brush: *mut GpSolidFill, font: *mut GpFont, label: &str, rect: RectF, color: u32) {
    if font.is_null() { return; }
    let chars: Vec<u16> = label.encode_utf16().chain(std::iter::once(0)).collect();
    let _ = GdipSetSolidFillColor(brush, color);
    let _ = GdipDrawString(g, PCWSTR(chars.as_ptr()), -1, font, &rect, std::ptr::null_mut(), brush as *mut GpBrush);
}

unsafe fn draw_line(g: *mut GpGraphics, pen: *mut GpPen, room: &RoomState, a: Point3, b: Point3) {
    let mut a = room.camera_point(a);
    let mut b = room.camera_point(b);
    if a.z <= NEAR && b.z <= NEAR { return; }
    let clip = NEAR + 0.01;
    if a.z <= NEAR { a = a.mix(b, (clip - a.z) / (b.z - a.z)); }
    if b.z <= NEAR { b = b.mix(a, (clip - b.z) / (a.z - b.z)); }
    if let (Some(a), Some(b)) = (room.project_camera(a), room.project_camera(b)) {
        let _ = GdipDrawLine(g, pen, a.X, a.Y, b.X, b.Y);
    }
}

unsafe fn render_panel(g: *mut GpGraphics, brush: *mut GpSolidFill, pen: *mut GpPen, room: &RoomState, panel: &Panel, selected: bool) {
    let Some(p) = room.projected_panel(panel) else { return; };
    let poly = [p[0], p[1], p[3], p[2]];
    if poly.iter().all(|p| p.X < 0.0) || poly.iter().all(|p| p.X > room.width as f32)
        || poly.iter().all(|p| p.Y < 0.0) || poly.iter().all(|p| p.Y > room.height as f32) { return; }
    let _ = GdipSetSolidFillColor(brush, 0xFF252C37);
    let _ = GdipFillPolygon(g, brush as *mut GpBrush, poly.as_ptr(), 4, FillModeAlternate);
    if !panel.image.is_null() {
        let mut width = 0u32;
        let mut height = 0u32;
        let _ = GdipGetImageWidth(panel.image as *mut GpImage, &mut width);
        let _ = GdipGetImageHeight(panel.image as *mut GpImage, &mut height);
        if width > 0 && height > 0 {
            let segments = 4;
            for j in 0..segments {
                let a = j as f32 / segments as f32;
                let b = (j + 1) as f32 / segments as f32;
                let p0 = room.project(panel.points[0].mix(panel.points[1], a));
                let p1 = room.project(panel.points[0].mix(panel.points[1], b));
                let p2 = room.project(panel.points[2].mix(panel.points[3], a));
                if let (Some(p0), Some(p1), Some(p2)) = (p0, p1, p2) {
                    let dest = [p0, p1, p2];
                    let _ = GdipDrawImagePointsRect(g, panel.image as *mut GpImage, dest.as_ptr(), 3,
                        width as f32 * a, 0.0, width as f32 * (b - a), height as f32,
                        UnitPixel, std::ptr::null(), 0, std::ptr::null_mut());
                }
            }
        }
    }
    let _ = GdipSetPenColor(pen, if selected { 0xFFE2ECF5 } else { 0xFF829EBB });
    let _ = GdipSetPenWidth(pen, if selected { 3.0 } else { 2.0 });
    for j in 0..4 {
        let a = poly[j]; let b = poly[(j + 1) % 4];
        let _ = GdipDrawLine(g, pen, a.X, a.Y, b.X, b.Y);
    }
}

unsafe fn paint(room: &RoomState, hdc: HDC) {
    let dc = room.cache.dc;
    let mut g: *mut GpGraphics = std::ptr::null_mut();
    let _ = GdipCreateFromHDC(dc, &mut g);
    if !g.is_null() {
        let _ = GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
        let _ = GdipSetTextRenderingHint(g, TextRenderingHintAntiAliasGridFit);
        let _ = GdipSetInterpolationMode(g, InterpolationModeBilinear);
        let _ = GdipGraphicsClear(g, 0xFF121923);
        let brush = room.cache.brush;
        let pen = room.cache.pen;
        let _ = GdipSetPenColor(pen, 0xFF34495D);
        let _ = GdipSetPenWidth(pen, 1.0);
        if !brush.is_null() && !pen.is_null() {
            let _ = GdipSetSolidFillColor(brush, 0xFF17212C);
            let _ = GdipFillRectangle(g, brush as *mut GpBrush, 0.0, room.height as f32 * 0.52, room.width as f32, room.height as f32 * 0.48);
            for x in -8..=8 {
                draw_line(g, pen, room, Point3 { x: x as f32, y: 0.0, z: 0.4 }, Point3 { x: x as f32, y: 0.0, z: 14.0 });
            }
            for z in 1..=14 {
                draw_line(g, pen, room, Point3 { x: -8.0, y: 0.0, z: z as f32 }, Point3 { x: 8.0, y: 0.0, z: z as f32 });
            }
            for i in room.order() { render_panel(g, brush, pen, room, &room.panels[i], room.hover == Some(i)); }
            let font = room.cache.font;
            let hint = if room.interacting.is_some() { "Управление окном · СКМ — вернуться в комнату · Alt+X — выход" }
                       else if room.panels.is_empty() { "Нет доступных окон · Alt+X / Esc — выход" }
                       else { "WASD — движение   Мышь — обзор   ПКМ — двигать экран   ЛКМ — управление" };
            text(g, brush, font, hint, RectF { X: 24.0, Y: 20.0, Width: room.width as f32 - 48.0, Height: 28.0 }, 0xFFE5EBF2);
            let caption = room.hover.and_then(|i| room.panels.get(i)).map(|p| p.title.as_str())
                .unwrap_or("ЛКМ: перейти в окно · СКМ: вернуться в комнату · Alt+X / Esc: выход");
            text(g, brush, font, caption, RectF { X: 24.0, Y: room.height as f32 - 46.0, Width: room.width as f32 - 48.0, Height: 26.0 }, 0xFFA9B9C9);
            let cx = room.width as f32 * 0.5;
            let cy = room.height as f32 * 0.5;
            let _ = GdipSetPenColor(pen, 0xFFDDE5ED);
            let _ = GdipSetPenWidth(pen, 1.4);
            let _ = GdipDrawLine(g, pen, cx - 6.0, cy, cx + 6.0, cy);
            let _ = GdipDrawLine(g, pen, cx, cy - 6.0, cx, cy + 6.0);
        }
        let _ = GdipDeleteGraphics(g);
        let _ = BitBlt(hdc, 0, 0, room.width, room.height, Some(dc), 0, 0, SRCCOPY);
    }
}

unsafe extern "system" fn room_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut RoomState;
    match msg {
        WM_ERASEBKGND => return LRESULT(1),
        WM_SHOWWINDOW if !ptr.is_null() => {
            if wparam.0 != 0 {
                let _ = SetTimer(Some(hwnd), TIMER, 16, None);
            } else {
                let _ = KillTimer(Some(hwnd), TIMER);
            }
            return LRESULT(0);
        }
        WM_TIMER if !ptr.is_null() && wparam.0 == TIMER => {
            update_movement(hwnd, &mut *ptr);
            return LRESULT(0);
        }
        WM_SETCURSOR if !ptr.is_null() && (*ptr).interacting.is_none() && GetForegroundWindow() == hwnd => {
            let _ = SetCursor(None);
            return LRESULT(1);
        }
        WM_SETFOCUS if !ptr.is_null() && (*ptr).interacting.is_none() => {
            let mut rect = RECT::default();
            if GetClientRect(hwnd, &mut rect).is_ok() {
                let mut p = windows::Win32::Foundation::POINT { x: rect.right / 2, y: rect.bottom / 2 };
                let _ = ClientToScreen(hwnd, &mut p);
                let _ = SetCursorPos(p.x, p.y);
            }
        }
        WM_KEYDOWN if !ptr.is_null() => {
            if (*ptr).interacting.is_some() { return LRESULT(0); }
            if wparam.0 as u32 == VK_ESCAPE.0 as u32 {
                let old = (*ptr).old_foreground;
                (*ptr).drag = None;
                let _ = ShowWindow(hwnd, SW_HIDE);
                if !old.0.is_null() { let _ = SetForegroundWindow(old); }
            }
            return LRESULT(0);
        }
        WM_MOUSEMOVE if !ptr.is_null() => {
            if (*ptr).interacting.is_some() { return LRESULT(0); }
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            move_mouse(hwnd, &mut *ptr, x, y);
            return LRESULT(0);
        }
        WM_RBUTTONDOWN if !ptr.is_null() => {
            if (*ptr).interacting.is_some() { return LRESULT(0); }
            if apply_mouse(&mut *ptr) { let _ = InvalidateRect(Some(hwnd), None, false); }
            (*ptr).drag = (*ptr).center_hit();
            return LRESULT(0);
        }
        WM_RBUTTONUP if !ptr.is_null() => {
            if (*ptr).interacting.is_some() { return LRESULT(0); }
            if apply_mouse(&mut *ptr) { let _ = InvalidateRect(Some(hwnd), None, false); }
            (*ptr).drag = None;
            return LRESULT(0);
        }
        WM_LBUTTONDOWN if !ptr.is_null() => {
            if (*ptr).interacting.is_some() { return LRESULT(0); }
            if apply_mouse(&mut *ptr) { let _ = InvalidateRect(Some(hwnd), None, false); }
            if let Some(i) = (*ptr).center_hit() { enter_window(hwnd, &mut *ptr, i); }
            return LRESULT(0);
        }
        RETURN_TO_ROOM if !ptr.is_null() => {
            if (*ptr).interacting.is_some() { return_to_room(hwnd, &mut *ptr); }
            return LRESULT(0);
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            if !ptr.is_null() { paint(&*ptr, hdc); }
            let _ = EndPaint(hwnd, &ps);
            return LRESULT(0);
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}
