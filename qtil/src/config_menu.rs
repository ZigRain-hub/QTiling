#![allow(unsafe_op_in_unsafe_fn, dead_code)]

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::time::Instant;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Graphics::GdiPlus::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{w, PCWSTR};
use std::result::Result::Ok;

const WIN_W: i32 = 900;
const WIN_H: i32 = 580;
const SIDE_FULL: f32 = 230.0;
const SIDE_MINI: f32 = 68.0;
const NAV_Y: f32 = 104.0;
const NAV_H: f32 = 40.0;
const NAV_STEP: f32 = 46.0;
const TIMER_ID: usize = 1;
const PI: f32 = std::f32::consts::PI;
const TAU: f32 = std::f32::consts::PI * 2.0;

const NAV: [&str; 7] = ["Главная", "Бинды", "Оформление", "Туториал", "О программе", "Тайлинг", "Шрифт"];

const BINDS: [(&str, &str); 13] = [
    ("Alt+Q", "Собрать / разобрать тайлинг (3 последних окна)"),
    ("Alt+A", "Добавить окно (до 5)"),
    ("Alt+S", "Убрать окно"),
    ("Alt+R", "Отдельный тайлинг одного приложения"),
    ("Alt+F", "Закрыть последнее окно тайлинга"),
    ("Alt+W", "Главное окно (держи Alt, жми W)"),
    ("Alt+D", "Exposé со змеями"),
    ("Alt+X", "3D-обзор окон"),
    ("Alt+C", "Бесконечный холст (клик по окну — работать в нём)"),
    ("Alt+V", "Холст: снять выбор с окна"),
    ("Alt+E", "Зеркало раскладки"),
    ("Alt+Z", "Это меню"),
    ("Ctrl+Alt+Q", "Выход"),
];

const STEPS: [&str; 5] = [
    "Alt+Q — собери последние окна в тайлинг",
    "Alt+A / Alt+S — добавь или убери окно (до 5)",
    "Держи Alt, жми W — сделай окно главным",
    "Alt+D — Exposé, клик по карточке или Esc",
    "Alt+E — зеркально отразить раскладку",
];

const C_PILL: u32 = 0xFF344659;
const C_PILL_TEXT: u32 = 0xFFF1F5F9;
const C_TEXT: u32 = 0xFFEDF0F2;
const C_DIM: u32 = 0xFFABB5BE;


pub const COLORS: [(u8, u8, u8); 6] = [
    (81, 154, 230), (137, 171, 213), (49, 190, 151),
    (207, 155, 75), (0, 229, 255), (255, 43, 214),
];
const THEMES: [(&str, usize, usize); 3] = [
    ("Синий", 0, 1), ("Изумруд", 2, 0), ("Неон", 4, 5),
];

#[derive(Clone, Copy, PartialEq, Eq)]
struct Preferences { duration_ms: u32, tile_duration_ms: u32, color_a: usize, color_b: usize, theme: usize, font: usize }

impl Default for Preferences {
    fn default() -> Self { Self { duration_ms: 1000, tile_duration_ms: 192, color_a: 0, color_b: 1, theme: 0, font: 0 } }
}

fn preferences_path() -> std::path::PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("Q-Tiler").join("settings.ini")
}

fn load_preferences() -> Preferences {
    let mut p = Preferences::default();
    if let Ok(text) = std::fs::read_to_string(preferences_path()) {
        for line in text.lines() {
            if let Some((key, value)) = line.split_once('=') {
                match key {
                    "duration_ms" => if let Ok(v) = value.parse::<u32>() { p.duration_ms = v.clamp(10, 5000); },
                    "tile_duration_ms" => if let Ok(v) = value.parse::<u32>() { p.tile_duration_ms = v.clamp(10, 5000); },
                    "color_a" => if let Ok(v) = value.parse::<usize>() { if v < COLORS.len() { p.color_a = v; } },
                    "color_b" => if let Ok(v) = value.parse::<usize>() { if v < COLORS.len() { p.color_b = v; } },
                    "theme" => if let Ok(v) = value.parse::<usize>() { if v <= THEMES.len() { p.theme = v; } },
                    "font" => if let Ok(v) = value.parse::<usize>() { if v < 3 { p.font = v; } },
                    _ => {}
                }
            }
        }
    }
    p
}

fn save_preferences(p: Preferences) -> std::io::Result<()> {
    let path = preferences_path();
    if let Some(dir) = path.parent() { std::fs::create_dir_all(dir)?; }
    std::fs::write(path, format!("duration_ms={}\ntile_duration_ms={}\ncolor_a={}\ncolor_b={}\ntheme={}\nfont={}\n",
        p.duration_ms, p.tile_duration_ms, p.color_a, p.color_b, p.theme, p.font))
}

type RegistryKey = *mut c_void;

#[link(name = "advapi32")]
unsafe extern "system" {
    #[link_name = "RegOpenKeyExW"]
    fn reg_open_key(root: RegistryKey, subkey: *const u16, options: u32, access: u32, result: *mut RegistryKey) -> i32;
    #[link_name = "RegCreateKeyExW"]
    fn reg_create_key(root: RegistryKey, subkey: *const u16, reserved: u32, class: *mut u16,
        options: u32, access: u32, security: *const c_void, result: *mut RegistryKey, disposition: *mut u32) -> i32;
    #[link_name = "RegQueryValueExW"]
    fn reg_query_value(key: RegistryKey, name: *const u16, reserved: *mut u32,
        kind: *mut u32, data: *mut u8, size: *mut u32) -> i32;
    #[link_name = "RegSetValueExW"]
    fn reg_set_value(key: RegistryKey, name: *const u16, reserved: u32, kind: u32, data: *const u8, size: u32) -> i32;
    #[link_name = "RegDeleteValueW"]
    fn reg_delete_value(key: RegistryKey, name: *const u16) -> i32;
    #[link_name = "RegCloseKey"]
    fn reg_close_key(key: RegistryKey) -> i32;
}

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const STARTUP_NAME: &str = "Q-Tiler";
const KEY_QUERY_VALUE: u32 = 0x0001;
const KEY_SET_VALUE: u32 = 0x0002;
const REG_SZ: u32 = 1;
const ERROR_FILE_NOT_FOUND: i32 = 2;

struct OpenKey(RegistryKey);

impl Drop for OpenKey {
    fn drop(&mut self) { unsafe { reg_close_key(self.0); } }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn current_user_key() -> RegistryKey {
    0x8000_0001u32 as i32 as isize as RegistryKey
}

fn startup_command() -> std::io::Result<Vec<u16>> {
    let path = std::env::current_exe()?;
    let mut command = Vec::new();
    command.push('"' as u16);
    command.extend(path.as_os_str().encode_wide());
    command.push('"' as u16);
    command.push(0);
    Ok(command)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StartupStatus {
    Off,
    On,
    OtherCopy,
    Error,
}

fn startup_status() -> StartupStatus {
    let expected = match startup_command() {
        Ok(command) => command,
        Err(_) => return StartupStatus::Error,
    };
    let mut raw = std::ptr::null_mut();
    let result = unsafe {
        reg_open_key(current_user_key(), wide(RUN_KEY).as_ptr(), 0, KEY_QUERY_VALUE, &mut raw)
    };
    if result == ERROR_FILE_NOT_FOUND { return StartupStatus::Off; }
    if result != 0 { return StartupStatus::Error; }
    let key = OpenKey(raw);
    let name = wide(STARTUP_NAME);
    let mut kind = 0u32;
    let mut size = 0u32;
    let result = unsafe {
        reg_query_value(key.0, name.as_ptr(), std::ptr::null_mut(), &mut kind, std::ptr::null_mut(), &mut size)
    };
    if result == ERROR_FILE_NOT_FOUND { return StartupStatus::Off; }
    if result != 0 { return StartupStatus::Error; }
    if kind != REG_SZ { return StartupStatus::OtherCopy; }
    if size > 65536 || size % 2 != 0 { return StartupStatus::Error; }
    let mut data = vec![0u16; (size as usize / 2).max(1)];
    let result = unsafe {
        reg_query_value(key.0, name.as_ptr(), std::ptr::null_mut(), &mut kind,
            data.as_mut_ptr() as *mut u8, &mut size)
    };
    if result != 0 { return StartupStatus::Error; }
    if kind != REG_SZ { return StartupStatus::OtherCopy; }
    let len = (size as usize / 2).min(data.len());
    let actual = String::from_utf16_lossy(&data[..len]);
    let expected = String::from_utf16_lossy(&expected);
    if actual.trim_end_matches('\0').eq_ignore_ascii_case(expected.trim_end_matches('\0')) {
        StartupStatus::On
    } else {
        StartupStatus::OtherCopy
    }
}

fn set_startup(enabled: bool) -> std::io::Result<()> {
    let command = if enabled { Some(startup_command()?) } else { None };
    let mut raw = std::ptr::null_mut();
    let result = unsafe {
        if enabled {
            reg_create_key(current_user_key(), wide(RUN_KEY).as_ptr(), 0, std::ptr::null_mut(),
                0, KEY_SET_VALUE, std::ptr::null(), &mut raw, std::ptr::null_mut())
        } else {
            reg_open_key(current_user_key(), wide(RUN_KEY).as_ptr(), 0, KEY_SET_VALUE, &mut raw)
        }
    };
    if !enabled && result == ERROR_FILE_NOT_FOUND { return Ok(()); }
    if result != 0 { return Err(std::io::Error::from_raw_os_error(result)); }
    let key = OpenKey(raw);
    let name = wide(STARTUP_NAME);
    let result = unsafe {
        if let Some(command) = command {
            reg_set_value(key.0, name.as_ptr(), 0, REG_SZ, command.as_ptr() as *const u8, (command.len() * 2) as u32)
        } else {
            reg_delete_value(key.0, name.as_ptr())
        }
    };
    if result == 0 || (!enabled && result == ERROR_FILE_NOT_FOUND) { Ok(()) }
    else { Err(std::io::Error::from_raw_os_error(result)) }
}

pub struct SettingsState {
    prefs: Preferences,
    applied: Preferences,
    save_error: bool,
    applied_notice: bool,
    font_missing: bool,
    startup: StartupStatus,
    startup_error: bool,
    pub anim_t: f32,
    pub tab: usize,
    pub pill_y: f32,
    pub side_w: f32,
    pub collapsed: bool,
    pub hover: i32,
    pub hover_anim: [f32; 11],
    pub tab_fade: f32,
    pub spin: f32,
    pub intro_done: bool,
    pub base_x: i32,
    pub base_y: i32,
    pub screen_w: i32,
    pub screen_h: i32,
    pub last: Instant,

    font_title: *mut GpFont,
    font_big: *mut GpFont,
    font_body: *mut GpFont,
    font_small: *mut GpFont,
    fmt_left: *mut GpStringFormat,
    fmt_center: *mut GpStringFormat,
}

impl SettingsState {
    pub fn tile_duration_ms(&self) -> u32 { self.applied.tile_duration_ms }
    pub fn overlay_duration(&self) -> f32 { self.applied.duration_ms as f32 / 1000.0 }
    pub fn overlay_colors(&self) -> ((u8, u8, u8), (u8, u8, u8)) {
        (COLORS[self.applied.color_a], COLORS[self.applied.color_b])
    }
    fn apply(&mut self) {
        self.save_error = save_preferences(self.prefs).is_err();
        self.applied_notice = !self.save_error;
        if !self.save_error { self.applied = self.prefs; }
    }
}


fn lerp(a: f32, b: f32, t: f32) -> f32 { a + (b - a) * t }
fn smooth(t: f32) -> f32 { let t = t.clamp(0.0, 1.0); t * t * (3.0 - 2.0 * t) }
fn smoother(t: f32) -> f32 { let t = t.clamp(0.0, 1.0); t * t * t * (t * (t * 6.0 - 15.0) + 10.0) }

fn lerp_col(c1: u32, c2: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let ch = |s: u32| -> u32 {
        let a = ((c1 >> s) & 0xFF) as f32;
        let b = ((c2 >> s) & 0xFF) as f32;
        (a + (b - a) * t) as u32
    };
    (ch(24) << 24) | (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

fn ca(c: u32, a: f32) -> u32 {
    let al = (((c >> 24) & 0xFF) as f32 * a.clamp(0.0, 1.0)) as u32;
    (al << 24) | (c & 0x00FF_FFFF)
}

fn in_rect(x: f32, y: f32, rx: f32, ry: f32, rw: f32, rh: f32) -> bool {
    x >= rx && x <= rx + rw && y >= ry && y <= ry + rh
}

unsafe fn rounded_path(l: f32, t: f32, w: f32, h: f32, r: f32) -> *mut GpPath {
    let mut p: *mut GpPath = std::ptr::null_mut();
    let _ = GdipCreatePath(FillModeAlternate, &mut p);
    let r = r.min(w / 2.0).min(h / 2.0).max(0.5);
    let d = r * 2.0;
    let _ = GdipAddPathArc(p, l, t, d, d, 180.0, 90.0);
    let _ = GdipAddPathArc(p, l + w - d, t, d, d, 270.0, 90.0);
    let _ = GdipAddPathArc(p, l + w - d, t + h - d, d, d, 0.0, 90.0);
    let _ = GdipAddPathArc(p, l, t + h - d, d, d, 90.0, 90.0);
    let _ = GdipClosePathFigure(p);
    p
}

const FONT_NAMES: [&str; 3] = ["Segoe UI", "Pixelify Sans", "Trebuchet MS"];

unsafe fn make_font(name: &str, size: f32, style: i32) -> *mut GpFont {
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut family: *mut GpFontFamily = std::ptr::null_mut();
    let _ = GdipCreateFontFamilyFromName(PCWSTR(wide.as_ptr()), std::ptr::null_mut(), &mut family);
    if family.is_null() { return std::ptr::null_mut(); }
    let mut font: *mut GpFont = std::ptr::null_mut();
    let _ = GdipCreateFont(family, size, style, UnitPixel, &mut font);
    let _ = GdipDeleteFontFamily(family);
    font
}

impl SettingsState {
    unsafe fn select_font(&mut self, index: usize) -> bool {
        let name = FONT_NAMES[index];
        let fonts = [make_font(name, 20.0, 1), make_font(name, 27.0, 1),
                     make_font(name, 15.0, 0), make_font(name, 13.0, 0)];
        if fonts.iter().any(|font| font.is_null()) {
            for font in fonts { if !font.is_null() { let _ = GdipDeleteFont(font); } }
            return false;
        }
        for font in [self.font_title, self.font_big, self.font_body, self.font_small] {
            if !font.is_null() { let _ = GdipDeleteFont(font); }
        }
        [self.font_title, self.font_big, self.font_body, self.font_small] = fonts;
        true
    }
}


struct Gfx {
    g: *mut GpGraphics,
    brush: *mut GpSolidFill,
    pen: *mut GpPen,
    fl: *mut GpStringFormat,
    fc: *mut GpStringFormat,
    title: *mut GpFont,
    big: *mut GpFont,
    body: *mut GpFont,
    small: *mut GpFont,
}

impl Gfx {
    unsafe fn new(g: *mut GpGraphics, st: &SettingsState) -> Self {
        let mut brush: *mut GpSolidFill = std::ptr::null_mut();
        let _ = GdipCreateSolidFill(0xFFFFFFFFu32, &mut brush);
        let mut pen: *mut GpPen = std::ptr::null_mut();
        let _ = GdipCreatePen1(0xFFFFFFFFu32, 1.0f32, UnitPixel, &mut pen);
        let _ = GdipSetPenStartCap(pen, LineCapRound);
        let _ = GdipSetPenEndCap(pen, LineCapRound);
        Gfx {
            g, brush, pen,
            fl: st.fmt_left, fc: st.fmt_center,
            title: st.font_title, big: st.font_big, body: st.font_body, small: st.font_small,
        }
    }

    unsafe fn free(&self) {
        let _ = GdipDeleteBrush(self.brush as *mut GpBrush);
        let _ = GdipDeletePen(self.pen);
    }

    unsafe fn fill_rect(&self, c: u32, x: f32, y: f32, w: f32, h: f32) {
        let _ = GdipSetSolidFillColor(self.brush, c);
        let _ = GdipFillRectangle(self.g, self.brush as *mut GpBrush, x, y, w, h);
    }

    unsafe fn fill_rr(&self, c: u32, x: f32, y: f32, w: f32, h: f32, r: f32) {
        let p = rounded_path(x, y, w, h, r);
        let _ = GdipSetSolidFillColor(self.brush, c);
        let _ = GdipFillPath(self.g, self.brush as *mut GpBrush, p);
        let _ = GdipDeletePath(p);
    }

    unsafe fn stroke_rr(&self, c: u32, width: f32, x: f32, y: f32, w: f32, h: f32, r: f32) {
        let p = rounded_path(x, y, w, h, r);
        let _ = GdipSetPenColor(self.pen, c);
        let _ = GdipSetPenWidth(self.pen, width);
        let _ = GdipDrawPath(self.g, self.pen, p);
        let _ = GdipDeletePath(p);
    }

    unsafe fn fill_ellipse(&self, c: u32, x: f32, y: f32, w: f32, h: f32) {
        let _ = GdipSetSolidFillColor(self.brush, c);
        let _ = GdipFillEllipse(self.g, self.brush as *mut GpBrush, x, y, w, h);
    }

    unsafe fn stroke_ellipse(&self, c: u32, width: f32, x: f32, y: f32, w: f32, h: f32) {
        let _ = GdipSetPenColor(self.pen, c);
        let _ = GdipSetPenWidth(self.pen, width);
        let _ = GdipDrawEllipse(self.g, self.pen, x, y, w, h);
    }

    unsafe fn line(&self, c: u32, width: f32, x1: f32, y1: f32, x2: f32, y2: f32) {
        let _ = GdipSetPenColor(self.pen, c);
        let _ = GdipSetPenWidth(self.pen, width);
        let _ = GdipDrawLine(self.g, self.pen, x1, y1, x2, y2);
    }

    unsafe fn poly(&self, c: u32, pts: &[PointF]) {
        let _ = GdipSetSolidFillColor(self.brush, c);
        let _ = GdipFillPolygon(self.g, self.brush as *mut GpBrush, pts.as_ptr(), pts.len() as i32, FillModeAlternate);
    }

    unsafe fn text(&self, s: &str, font: *mut GpFont, center: bool, x: f32, y: f32, w: f32, h: f32, c: u32) {
        if font.is_null() { return; }
        let wide: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
        let rect = RectF { X: x.round(), Y: y.round(), Width: w.round(), Height: h.round() };
        let _ = GdipSetSolidFillColor(self.brush, c);
        let fmt = if center { self.fc } else { self.fl };
        let _ = GdipDrawString(self.g, PCWSTR(wide.as_ptr()), -1, font, &rect, fmt, self.brush as *mut GpBrush);
    }
}

unsafe fn draw_orbits(gx: &Gfx, cx: f32, cy: f32, size: f32, ang: f32, alpha: f32, flip: bool) {
    let a = size * 0.5;
    let b = size * 0.17;
    let n = 64;
    let top_ref = cy - size * 0.5;
    let width = (size * 0.045).max(1.5);
    for i in 0..3 {
        let th = ang + i as f32 * PI / 3.0;
        let (sn, cs) = th.sin_cos();
        let mut prev: Option<(f32, f32)> = None;
        for k in 0..=n {
            let ph = k as f32 / n as f32 * TAU;
            let (ex, ey) = (a * ph.cos(), b * ph.sin());
            let rx = ex * cs - ey * sn;
            let mut ry = ex * sn + ey * cs;
            if flip { ry = -ry; }
            let (x, y) = (cx + rx, cy + ry);
            if let Some((px, py)) = prev {
                let depth = (ph.sin() + 1.0) * 0.5;
                let mut al = alpha * (0.35 + 0.65 * depth);
                if flip { al *= (1.0 - (y - top_ref) / size).clamp(0.0, 1.0); }
                let col = lerp_col(C_PILL, 0xFFFFFFFF, depth);
                gx.line(ca(col, al), width, px, py, x, y);
            }
            prev = Some((x, y));
        }
    }
    let r = size * 0.055;
    gx.fill_ellipse(ca(0xFFFFFFFF, alpha * if flip { 0.3 } else { 1.0 }), cx - r, cy - r, r * 2.0, r * 2.0);
}

unsafe fn draw_icon(gx: &Gfx, i: usize, cx: f32, cy: f32, col: u32, glyph_col: u32) {
    match i {
        0 => {
            gx.line(col, 1.7, cx - 8.0, cy, cx, cy - 7.0);
            gx.line(col, 1.7, cx, cy - 7.0, cx + 8.0, cy);
            gx.line(col, 1.7, cx - 5.5, cy - 1.0, cx - 5.5, cy + 7.0);
            gx.line(col, 1.7, cx - 5.5, cy + 7.0, cx + 5.5, cy + 7.0);
            gx.line(col, 1.7, cx + 5.5, cy + 7.0, cx + 5.5, cy - 1.0);
        }
        1 => {
            gx.stroke_rr(col, 1.6, cx - 9.0, cy - 6.0, 18.0, 12.0, 3.0);
            for k in 0..3 {
                gx.fill_ellipse(col, cx - 5.5 + k as f32 * 4.6, cy - 3.0, 2.4, 2.4);
            }
            gx.line(col, 1.6, cx - 4.0, cy + 2.5, cx + 4.0, cy + 2.5);
        }
        2 => {
            gx.stroke_ellipse(col, 1.6, cx - 8.0, cy - 8.0, 16.0, 16.0);
            gx.fill_ellipse(col, cx - 4.5, cy - 4.5, 3.0, 3.0);
            gx.fill_ellipse(col, cx + 1.5, cy - 4.5, 3.0, 3.0);
            gx.fill_ellipse(col, cx - 1.5, cy + 1.0, 3.0, 3.0);
        }
        3 => {
            gx.fill_rr(col, cx - 8.0, cy - 8.0, 16.0, 16.0, 4.0);
            gx.text("?", gx.small, true, cx - 8.0, cy - 8.0, 16.0, 16.0, glyph_col);
        }
        5 => {
            gx.stroke_rr(col, 1.5, cx - 8.0, cy - 7.0, 16.0, 14.0, 2.0);
            gx.line(col, 1.5, cx, cy - 7.0, cx, cy + 7.0);
            gx.line(col, 1.5, cx, cy, cx + 8.0, cy);
        }
        6 => {
            gx.text("T", gx.body, true, cx - 9.0, cy - 10.0, 18.0, 20.0, col);
        }
        _ => {
            gx.fill_ellipse(col, cx - 8.0, cy - 8.0, 16.0, 16.0);
            gx.text("i", gx.small, true, cx - 8.0, cy - 8.0, 16.0, 16.0, glyph_col);
        }
    }
}


fn btn_rects(side_w: f32) -> [(f32, f32, f32, f32); 2] {
    let cw = WIN_W as f32 - side_w;
    let cx = side_w + cw / 2.0;
    let (w1, w2, h, gap) = (150.0f32, 190.0f32, 40.0f32, 16.0f32);
    let x0 = cx - (w1 + gap + w2) / 2.0;
    let y = 440.0;
    [(x0, y, w1, h), (x0 + w1 + gap, y, w2, h)]
}

fn hit_test(st: &SettingsState, x: f32, y: f32) -> i32 {
    if in_rect(x, y, WIN_W as f32 - 42.0, 16.0, 28.0, 28.0) { return 110; }
    if in_rect(x, y, st.side_w / 2.0 - 20.0, WIN_H as f32 - 56.0, 40.0, 36.0) { return 100; }
    for i in 0..NAV.len() {
        if in_rect(x, y, 12.0, NAV_Y + i as f32 * NAV_STEP, st.side_w - 24.0, NAV_H) { return i as i32; }
    }
    if st.tab == 2 {
        let x0 = st.side_w + 36.0;
        for i in 0..3 {
            if in_rect(x, y, x0 + i as f32 * 132.0, 140.0, 120.0, 36.0) { return 200 + i as i32; }
        }
        for i in 0..4 {
            if in_rect(x, y, x0 + i as f32 * 100.0, 231.0, 88.0, 36.0) { return 210 + i as i32; }
        }
        if in_rect(x, y, x0 + 412.0, 231.0, 40.0, 36.0) { return 214; }
        if in_rect(x, y, x0 + 460.0, 231.0, 40.0, 36.0) { return 215; }
        if in_rect(x, y, x0, 510.0, 140.0, 38.0) { return 240; }
        for i in 0..6 {
            if in_rect(x, y, x0 + i as f32 * 48.0, 330.0, 36.0, 36.0) { return 220 + i as i32; }
            if in_rect(x, y, x0 + i as f32 * 48.0, 419.0, 36.0, 36.0) { return 230 + i as i32; }
        }
    }
    if st.tab == 6 {
        let x0 = st.side_w + 36.0;
        for i in 0..3 {
            if in_rect(x, y, x0, 126.0 + i as f32 * 84.0, 500.0, 68.0) { return 400 + i as i32; }
        }
        if in_rect(x, y, x0, 510.0, 140.0, 38.0) { return 240; }
    }
    if st.tab == 5 {
        let x0 = st.side_w + 36.0;
        for i in 0..4 {
            if in_rect(x, y, x0 + i as f32 * 100.0, 202.0, 88.0, 36.0) { return 300 + i as i32; }
        }
        if in_rect(x, y, x0 + 412.0, 202.0, 40.0, 36.0) { return 304; }
        if in_rect(x, y, x0 + 460.0, 202.0, 40.0, 36.0) { return 305; }
        if in_rect(x, y, x0, 510.0, 140.0, 38.0) { return 240; }
    }
    if st.tab == 0 {
        let x0 = st.side_w + 36.0;
        let rw = WIN_W as f32 - st.side_w - 72.0;
        if in_rect(x, y, x0, 380.0, rw, 44.0) { return 500; }
        for (j, r) in btn_rects(st.side_w).iter().enumerate() {
            if in_rect(x, y, r.0, r.1, r.2, r.3) { return 101 + j as i32; }
        }
    }
    -1
}

fn hover_index(code: i32) -> Option<usize> {
    match code {
        0..=4 => Some(code as usize),
        5 => Some(9),
        6 => Some(10),
        100 => Some(5),
        101 => Some(6),
        102 => Some(7),
        110 => Some(8),
        _ => None,
    }
}


pub unsafe fn create_settings_window(hinstance: HINSTANCE, screen_w: i32, screen_h: i32) -> (HWND, *mut SettingsState) {
    let class = w!("QtilSettings");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(settings_wnd_proc),
        hInstance: hinstance,
        lpszClassName: class,
        ..Default::default()
    };
    RegisterClassW(&wc);

    let x = (screen_w - WIN_W) / 2;
    let y = (screen_h - WIN_H) / 2;

    let hwnd = CreateWindowExW(
        WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED,
        class, w!("Q-Tiler Settings"),
        WS_POPUP,
        x, y, WIN_W, WIN_H,
        Some(HWND(std::ptr::null_mut())), Some(HMENU(std::ptr::null_mut())), Some(hinstance), None,
    ).unwrap();

    let mut fmt_left: *mut GpStringFormat = std::ptr::null_mut();
    let _ = GdipCreateStringFormat(0, 0, &mut fmt_left);
    let _ = GdipSetStringFormatAlign(fmt_left, StringAlignmentNear);
    let _ = GdipSetStringFormatLineAlign(fmt_left, StringAlignmentCenter);

    let mut fmt_center: *mut GpStringFormat = std::ptr::null_mut();
    let _ = GdipCreateStringFormat(0, 0, &mut fmt_center);
    let _ = GdipSetStringFormatAlign(fmt_center, StringAlignmentCenter);
    let _ = GdipSetStringFormatLineAlign(fmt_center, StringAlignmentCenter);

    let prefs = load_preferences();
    let st = Box::new(SettingsState {
        prefs,
        applied: prefs,
        save_error: false,
        applied_notice: false,
        font_missing: false,
        startup: startup_status(),
        startup_error: false,
        anim_t: 0.0,
        tab: 0,
        pill_y: NAV_Y,
        side_w: SIDE_FULL,
        collapsed: false,
        hover: -1,
        hover_anim: [0.0; 11],
        tab_fade: 1.0,
        spin: 0.0,
        intro_done: true,
        base_x: x,
        base_y: y,
        screen_w,
        screen_h,
        last: Instant::now(),
        font_title: make_font("Segoe UI", 20.0, 1),
        font_big: make_font("Segoe UI", 27.0, 1),
        font_body: make_font("Segoe UI", 15.0, 0),
        font_small: make_font("Segoe UI", 13.0, 0),
        fmt_left,
        fmt_center,
    });
    let mut st = st;
    if !st.select_font(prefs.font) {
        st.font_missing = true;
        st.prefs.font = 0;
        st.applied.font = 0;
    }
    let ptr = Box::into_raw(st);
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr as isize);

    let rgn = CreateRoundRectRgn(0, 0, WIN_W + 1, WIN_H + 1, 16, 16);
    let _ = SetWindowRgn(hwnd, Some(rgn), true);
    let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
    let _ = ShowWindow(hwnd, SW_HIDE);

    (hwnd, ptr)
}

pub unsafe fn free_settings_state(ptr: *mut SettingsState) {
    if !ptr.is_null() {
        drop(Box::from_raw(ptr));
    }
}


unsafe fn tick(hwnd: HWND, st: &mut SettingsState) {
    let now = Instant::now();
    let dt = (now - st.last).as_secs_f32().min(0.05);
    st.last = now;
    st.anim_t += dt;
    let k = |rate: f32| 1.0 - (-dt * rate).exp();

    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    let _ = ScreenToClient(hwnd, &mut pt);
    st.hover = hit_test(st, pt.x as f32, pt.y as f32);

    let side_target = if st.collapsed { SIDE_MINI } else { SIDE_FULL };
    st.side_w += (side_target - st.side_w) * k(12.0);
    if (st.side_w - side_target).abs() < 0.1 { st.side_w = side_target; }
    let pill_target = NAV_Y + st.tab as f32 * NAV_STEP;
    st.pill_y += (pill_target - st.pill_y) * k(14.0);
    if (st.pill_y - pill_target).abs() < 0.05 { st.pill_y = pill_target; }

    for i in 0..11 {
        let target = if hover_index(st.hover) == Some(i) { 1.0 } else { 0.0 };
        st.hover_anim[i] += (target - st.hover_anim[i]) * k(16.0);
    }

    st.tab_fade = (st.tab_fade + dt * 3.0).min(1.0);
    st.spin += dt * 0.45;

    if !st.intro_done {
        let p = smoother(st.anim_t / 0.32);
        let alpha = (p * 255.0) as u8;
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA);
        let _ = SetWindowPos(hwnd, None, st.base_x, st.base_y + ((1.0 - p) * 24.0) as i32, 0, 0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        if p >= 1.0 {
            st.intro_done = true;
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
            let _ = SetWindowPos(hwnd, None, st.base_x, st.base_y, 0, 0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }
}


unsafe fn paint(st: &SettingsState, hdc: HDC) {
    let (w, h) = (WIN_W as f32, WIN_H as f32);
    let mem_dc = CreateCompatibleDC(Some(hdc));
    let mem_bmp = CreateCompatibleBitmap(hdc, WIN_W, WIN_H);
    let old = SelectObject(mem_dc, mem_bmp.into());

    let mut g: *mut GpGraphics = std::ptr::null_mut();
    let _ = GdipCreateFromHDC(mem_dc, &mut g);
    let _ = GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
    let _ = GdipSetTextRenderingHint(g, TextRenderingHintAntiAliasGridFit);
    let gx = Gfx::new(g, st);

    gx.fill_rect(0xFF1A1D21, 0.0, 0.0, w, h);

    let sw = st.side_w;
    gx.fill_rect(0xFF14171A, 0.0, 0.0, sw, h);
    gx.fill_rect(0x1CFFFFFF, sw, 0.0, 1.0, h);

    let la = ((sw - SIDE_MINI) / (SIDE_FULL - SIDE_MINI)).clamp(0.0, 1.0);
    let la = la * la;

    gx.fill_rr(C_PILL, 20.0, 24.0, 28.0, 28.0, 5.0);
    gx.text("Q", gx.body, true, 20.0, 24.0, 28.0, 28.0, 0xFFFFFFFF);
    if la > 0.02 {
        gx.text("Q-Tiler", gx.title, false, 62.0, 22.0, 160.0, 24.0, ca(C_TEXT, la));
        gx.text("tiling for Windows", gx.small, false, 62.0, 44.0, 160.0, 18.0, ca(C_DIM, la));
    }

    gx.fill_rr(C_PILL, 12.0, st.pill_y, sw - 24.0, NAV_H, 6.0);

    for i in 0..NAV.len() {
        let iy = NAV_Y + i as f32 * NAV_STEP;
        let act = (1.0 - (st.pill_y - iy).abs() / NAV_STEP).clamp(0.0, 1.0);
        let hover = st.hover_anim[if i == 5 { 9 } else if i == 6 { 10 } else { i }];
        let hv = hover * (1.0 - act);
        if hv > 0.01 {
            gx.fill_rr(ca(0x1EFFFFFF, hv), 12.0, iy, sw - 24.0, NAV_H, 10.0);
        }
        let col = lerp_col(lerp_col(C_DIM, C_TEXT, hover), C_PILL_TEXT, act);
        let glyph = lerp_col(C_PILL_TEXT, C_PILL, act);
        draw_icon(&gx, i, 34.0, iy + NAV_H / 2.0, col, glyph);
        if la > 0.02 {
            gx.text(NAV[i], gx.body, false, 58.0, iy, 170.0, NAV_H, ca(col, la));
        }
    }

    {
        let (ccx, ccy) = (sw / 2.0, h - 34.0);
        let hv = st.hover_anim[5];
        if hv > 0.01 { gx.fill_rr(ca(0x1EFFFFFF, hv), ccx - 18.0, ccy - 16.0, 36.0, 32.0, 8.0); }
        let col = lerp_col(C_DIM, C_TEXT, hv);
        let d = if st.collapsed { 1.0 } else { -1.0 };
        gx.line(col, 2.0, ccx - 2.0 * d, ccy - 6.0, ccx + 4.0 * d, ccy);
        gx.line(col, 2.0, ccx + 4.0 * d, ccy, ccx - 2.0 * d, ccy + 6.0);
    }

    {
        let (bx, by) = (w - 42.0, 16.0);
        let hv = st.hover_anim[8];
        if hv > 0.01 { gx.fill_rr(ca(0x55FF5F56, hv), bx, by, 28.0, 28.0, 14.0); }
        let col = lerp_col(C_DIM, C_TEXT, hv);
        let (mx, my) = (bx + 14.0, by + 14.0);
        gx.line(col, 1.7, mx - 5.0, my - 5.0, mx + 5.0, my + 5.0);
        gx.line(col, 1.7, mx + 5.0, my - 5.0, mx - 5.0, my + 5.0);
    }

    draw_content(&gx, st);

    gx.stroke_rr(0x22FFFFFF, 1.0, 0.5, 0.5, w - 1.0, h - 1.0, 8.0);

    gx.free();
    let _ = GdipDeleteGraphics(g);
    let _ = BitBlt(hdc, 0, 0, WIN_W, WIN_H, Some(mem_dc), 0, 0, SRCCOPY);
    SelectObject(mem_dc, old);
    let _ = DeleteObject(mem_bmp.into());
    let _ = DeleteDC(mem_dc);
}

unsafe fn draw_content(gx: &Gfx, st: &SettingsState) {
    let sw = st.side_w;
    let cw = WIN_W as f32 - sw;
    let cx = sw + cw / 2.0;
    let f = smooth(st.tab_fade);
    let yo = 0.0;
    let tx = |c: u32| ca(c, f);
    let x0 = sw + 36.0;
    let rw = cw - 72.0;

    if st.tab == 2 || st.tab == 5 || st.tab == 6 {
        gx.fill_rect(tx(0xFF343A40), x0, 488.0, rw, 1.0);
    }

    match st.tab {
        0 => {
            gx.text("Управление окнами", gx.big, false, x0, 110.0 + yo, rw, 44.0, tx(C_TEXT));
            gx.text("Тайлинг и быстрый доступ к окнам", gx.body, false, x0, 164.0 + yo, rw, 30.0, tx(C_DIM));
            gx.fill_rr(tx(0xFF24292E), x0, 225.0 + yo, rw, 140.0, 8.0);
            gx.text("Быстрый старт", gx.title, false, x0 + 22.0, 240.0 + yo, rw - 44.0, 30.0, tx(C_TEXT));
            gx.text("Alt+Q — организовать окна\nAlt+D — обзор окон\nAlt+Z — настройки", gx.body, false,
                x0 + 22.0, 282.0 + yo, rw - 44.0, 70.0, tx(C_DIM));
            let startup_hover = st.hover == 500;
            gx.fill_rr(tx(if startup_hover { 0xFF2B3239 } else { 0xFF24292E }), x0, 380.0, rw, 44.0, 6.0);
            gx.text("Автозапуск при входе в Windows", gx.body, false, x0 + 16.0, 380.0, rw - 205.0, 44.0, tx(C_TEXT));
            let label = if st.startup_error { "Ошибка" } else { match st.startup {
                StartupStatus::Off => "Выключен",
                StartupStatus::On => "Включён",
                StartupStatus::OtherCopy => "Другая копия",
                StartupStatus::Error => "Ошибка чтения",
            }};
            let status_color = if st.startup_error || st.startup == StartupStatus::Error { 0xFFFFC29B }
                else if st.startup == StartupStatus::On { 0xFF9FC6A8 } else { C_DIM };
            gx.text(label, gx.small, false, x0 + rw - 165.0, 380.0, 120.0, 44.0, tx(status_color));
            let sx = x0 + rw - 40.0;
            gx.fill_rr(tx(if st.startup == StartupStatus::On { 0xFF597F6B } else { 0xFF4A5057 }), sx, 392.0, 26.0, 20.0, 10.0);
            gx.fill_ellipse(tx(0xFFF1F3F4), sx + if st.startup == StartupStatus::On { 11.0 } else { 3.0 }, 395.0, 14.0, 14.0);
            let rects = btn_rects(sw);
            let labels = ["О программе", "Горячие клавиши"];
            for (j, r) in rects.iter().enumerate() {
                let hv = st.hover_anim[6 + j];
                gx.fill_rr(tx(lerp_col(0xFF263440, 0xFF34495D, hv)), r.0, r.1 + yo, r.2, r.3, 6.0);
                gx.text(labels[j], gx.body, true, r.0, r.1 + yo, r.2, r.3, tx(C_TEXT));
            }
        }
        1 => {
            gx.text("Бинды", gx.title, false, x0, 28.0 + yo, 300.0, 30.0, tx(C_TEXT));
            gx.text("глобальные горячие клавиши", gx.small, false, x0, 56.0 + yo, 400.0, 18.0, tx(C_DIM));
            for (i, (key, desc)) in BINDS.iter().enumerate() {
                let rf = smooth(st.tab_fade * 1.4 - i as f32 * 0.08);
                let ry = 90.0 + i as f32 * 34.0;
                gx.fill_rr(ca(0x0FFFFFFF, rf), x0, ry, rw, 32.0, 6.0);
                gx.fill_rr(ca(0xFF304763, rf), x0 + 8.0, ry + 4.0, 120.0, 24.0, 6.0);
                gx.text(key, gx.body, true, x0 + 8.0, ry + 4.0, 120.0, 24.0, ca(C_TEXT, rf));
                gx.text(desc, gx.body, false, x0 + 148.0, ry, rw - 156.0, 32.0, ca(C_TEXT, rf));
            }
        }
        2 => {
            gx.text("Оформление", gx.title, false, x0, 28.0 + yo, rw, 30.0, tx(C_TEXT));
            gx.text("Параметры обзора окон · Alt+D", gx.small, false, x0, 57.0 + yo, rw, 20.0, tx(C_DIM));
            gx.text("Тема", gx.body, false, x0, 104.0 + yo, rw, 24.0, tx(C_TEXT));
            for (i, (name, _, _)) in THEMES.iter().enumerate() {
                let x = x0 + i as f32 * 132.0;
                let selected = st.prefs.theme == i;
                gx.fill_rr(tx(if selected { C_PILL } else { 0xFF292E33 }), x, 140.0 + yo, 120.0, 36.0, 6.0);
                gx.text(name, gx.body, true, x, 140.0 + yo, 120.0, 36.0, tx(C_TEXT));
            }
            gx.text("Длительность перехода", gx.body, false, x0, 191.0 + yo, 300.0, 24.0, tx(C_TEXT));
            gx.text(&format!("{:.2} с", st.prefs.duration_ms as f32 / 1000.0), gx.body, false,
                x0 + 410.0, 191.0 + yo, 110.0, 24.0, tx(C_TEXT));
            for (i, (label, ms)) in [("0,01 с", 10u32), ("0,25 с", 250), ("0,50 с", 500), ("1,00 с", 1000)].iter().enumerate() {
                let x = x0 + i as f32 * 100.0;
                gx.fill_rr(tx(if st.prefs.duration_ms == *ms { C_PILL } else { 0xFF292E33 }), x, 231.0 + yo, 88.0, 36.0, 6.0);
                gx.text(label, gx.small, true, x, 231.0 + yo, 88.0, 36.0, tx(C_TEXT));
            }
            for (i, label) in ["−", "+"].iter().enumerate() {
                let x = x0 + 412.0 + i as f32 * 48.0;
                gx.fill_rr(tx(0xFF292E33), x, 231.0 + yo, 40.0, 36.0, 6.0);
                gx.text(label, gx.title, true, x, 231.0 + yo, 40.0, 36.0, tx(C_TEXT));
            }
            gx.text("− / + : шаг 0,01 с · диапазон 0,01–5,00 с", gx.small, false,
                x0, 273.0 + yo, rw, 20.0, tx(C_DIM));
            gx.text("Первый цвет", gx.body, false, x0, 302.0 + yo, rw, 24.0, tx(C_TEXT));
            gx.text("Второй цвет", gx.body, false, x0, 391.0 + yo, rw, 24.0, tx(C_TEXT));
            for row in 0..2 {
                for (i, &(r, g, b)) in COLORS.iter().enumerate() {
                    let x = x0 + i as f32 * 48.0;
                    let y = if row == 0 { 330.0 } else { 419.0 } + yo;
                    let color = 0xFF000000 | ((r as u32) << 16) | ((g as u32) << 8) | b as u32;
                    gx.fill_rr(tx(color), x, y, 36.0, 36.0, 6.0);
                    let selected = if row == 0 { st.prefs.color_a == i } else { st.prefs.color_b == i };
                    if selected { gx.stroke_rr(tx(0xFFFFFFFF), 2.0, x - 2.0, y - 2.0, 40.0, 40.0, 8.0); }
                }
            }
            let px = x0 + 320.0;
            let pw = (rw - 320.0).max(0.0);
            if pw >= 150.0 {
                gx.text("В ОБЗОРЕ ALT+D", gx.small, false, px, 302.0 + yo, pw, 24.0, tx(C_DIM));
                gx.fill_rr(tx(0xFF14181C), px, 330.0 + yo, pw, 125.0, 8.0);
                let (ar, ag, ab) = COLORS[st.prefs.color_a];
                let (br, bg, bb) = COLORS[st.prefs.color_b];
                let a = 0xFF000000 | ((ar as u32) << 16) | ((ag as u32) << 8) | ab as u32;
                let b = 0xFF000000 | ((br as u32) << 16) | ((bg as u32) << 8) | bb as u32;
                gx.fill_rr(tx(a), px + 14.0, 346.0 + yo, pw - 28.0, 6.0, 3.0);
                gx.fill_rr(tx(0xFF292E33), px + 35.0, 368.0 + yo, pw - 70.0, 46.0, 5.0);
                gx.stroke_rr(tx(b), 2.0, px + 35.0, 368.0 + yo, pw - 70.0, 46.0, 5.0);
                gx.fill_rr(tx(b), px + 14.0, 433.0 + yo, pw - 28.0, 6.0, 3.0);
            }
            let dirty = st.prefs != st.applied;
            let button = if st.hover == 240 { 0xFF42658C } else { 0xFF304F70 };
            gx.fill_rr(tx(button), x0, 510.0 + yo, 140.0, 38.0, 6.0);
            gx.text("Применить", gx.body, true, x0, 510.0 + yo, 140.0, 38.0, tx(C_TEXT));
            let msg = if st.save_error { "Ошибка сохранения настроек" }
                else if dirty { "Нажмите «Применить»" }
                else if st.applied_notice { "Готово: применено" } else { "Нет изменений" };
            gx.text(msg, gx.small, false, x0 + 156.0, 510.0 + yo, rw - 156.0, 38.0,
                tx(if st.save_error { 0xFFFFB4A6 } else { C_DIM }));
        }
        3 => {
            gx.text("Туториал", gx.title, false, x0, 28.0 + yo, 300.0, 30.0, tx(C_TEXT));
            gx.text("как пользоваться Q-Tiler", gx.small, false, x0, 56.0 + yo, 400.0, 18.0, tx(C_DIM));
            for (i, s) in STEPS.iter().enumerate() {
                let rf = smooth(st.tab_fade * 1.4 - i as f32 * 0.08);
                let ry = 100.0 + i as f32 * 60.0;
                gx.fill_rr(ca(0x0FFFFFFF, rf), x0, ry, rw, 48.0, 12.0);
                gx.fill_ellipse(ca(C_PILL, rf), x0 + 10.0, ry + 10.0, 28.0, 28.0);
                gx.text(&format!("{}", i + 1), gx.body, true, x0 + 10.0, ry + 10.0, 28.0, 28.0, ca(C_PILL_TEXT, rf));
                gx.text(s, gx.body, false, x0 + 52.0, ry, rw - 60.0, 48.0, ca(C_TEXT, rf));
            }
        }
        5 => {
            gx.text("Тайлинг", gx.title, false, x0, 28.0 + yo, rw, 30.0, tx(C_TEXT));
            gx.text("Анимация перемещения окон", gx.small, false, x0, 57.0 + yo, rw, 22.0, tx(C_DIM));
            gx.text("Время перехода", gx.body, false, x0, 126.0 + yo, 300.0, 26.0, tx(C_TEXT));
            gx.text(&format!("{:.2} с", st.prefs.tile_duration_ms as f32 / 1000.0),
                gx.body, false, x0 + 410.0, 126.0 + yo, 110.0, 26.0, tx(C_TEXT));
            for (i, (label, ms)) in [("0,01 с", 10u32), ("0,10 с", 100), ("0,25 с", 250), ("1,00 с", 1000)].iter().enumerate() {
                let x = x0 + i as f32 * 100.0;
                gx.fill_rr(tx(if st.prefs.tile_duration_ms == *ms { C_PILL } else { 0xFF292E33 }), x, 202.0 + yo, 88.0, 36.0, 6.0);
                gx.text(label, gx.small, true, x, 202.0 + yo, 88.0, 36.0, tx(C_TEXT));
            }
            for (i, label) in ["−", "+"].iter().enumerate() {
                let x = x0 + 412.0 + i as f32 * 48.0;
                gx.fill_rr(tx(0xFF292E33), x, 202.0 + yo, 40.0, 36.0, 6.0);
                gx.text(label, gx.title, true, x, 202.0 + yo, 40.0, 36.0, tx(C_TEXT));
            }
            gx.text("− / +: шаг 0,01 с · диапазон 0,01–5,00 с", gx.small, false,
                x0, 248.0 + yo, rw, 26.0, tx(C_DIM));
            gx.text("Применяется к формированию, расформированию, добавлению, удалению и перестановке окон.",
                gx.body, false, x0, 300.0 + yo, rw, 64.0, tx(C_DIM));
            gx.text("Alt+R: отдельный тайлинг одного приложения. Alt+F закрывает последнее окно. Alt+Q возвращает основной тайлинг.",
                gx.small, false, x0, 381.0 + yo, rw, 90.0, tx(C_DIM));
            let dirty = st.prefs != st.applied;
            let button = if st.hover == 240 { 0xFF42658C } else { 0xFF304F70 };
            gx.fill_rr(tx(button), x0, 510.0 + yo, 140.0, 38.0, 6.0);
            gx.text("Применить", gx.body, true, x0, 510.0 + yo, 140.0, 38.0, tx(C_TEXT));
            let msg = if st.save_error { "Ошибка сохранения настроек" } else if dirty { "Нажмите «Применить»" }
                else if st.applied_notice { "Готово: применено" } else { "Нет изменений" };
            gx.text(msg, gx.small, false, x0 + 156.0, 510.0 + yo, rw - 156.0, 38.0,
                tx(if st.save_error { 0xFFFFB4A6 } else { C_DIM }));
        }
        6 => {
            gx.text("Шрифт", gx.title, false, x0, 28.0, rw, 30.0, tx(C_TEXT));
            gx.text("Только для окна настроек", gx.small, false, x0, 57.0, rw, 20.0, tx(C_DIM));
            let labels = ["Обычный", "Пиксельный", "Мягкий"];
            let descriptions = ["Segoe UI · системный шрифт", "Pixelify Sans · требуется установка", "Trebuchet MS · системный шрифт"];
            for i in 0..3 {
                let y = 126.0 + i as f32 * 84.0;
                let selected = st.prefs.font == i;
                let hover = st.hover == 400 + i as i32;
                let bg = if selected { 0xFF293441 } else if hover { 0xFF272C31 } else { 0xFF22262B };
                gx.fill_rr(tx(bg), x0, y, 500.0, 68.0, 6.0);
                gx.stroke_rr(tx(if selected { 0xFF7D9BBB } else { 0xFF383E45 }), 1.0, x0, y, 500.0, 68.0, 6.0);
                gx.text(labels[i], gx.body, false, x0 + 18.0, y + 10.0, 420.0, 22.0, tx(C_TEXT));
                gx.text(descriptions[i], gx.small, false, x0 + 18.0, y + 36.0, 420.0, 18.0, tx(C_DIM));
                gx.stroke_ellipse(tx(if selected { 0xFF9DB7CF } else { 0xFF737D87 }), 1.5, x0 + 465.0, y + 26.0, 16.0, 16.0);
                if selected { gx.fill_ellipse(tx(0xFF9DB7CF), x0 + 469.0, y + 30.0, 8.0, 8.0); }
            }
            let hint = if st.font_missing { "Шрифт недоступен. Установите его и перезапустите Q-Tiler." }
                       else { "Выбор виден сразу. Нажмите «Применить», чтобы сохранить." };
            gx.text(hint, gx.small, false, x0, 395.0, rw, 50.0,
                tx(if st.font_missing { 0xFFFFC29B } else { C_DIM }));
            let dirty = st.prefs != st.applied;
            let button = if st.hover == 240 { 0xFF42658C } else { 0xFF344F68 };
            gx.fill_rr(tx(button), x0, 510.0, 140.0, 38.0, 6.0);
            gx.text("Применить", gx.body, true, x0, 510.0, 140.0, 38.0, tx(C_TEXT));
            let msg = if st.save_error { "Ошибка сохранения настроек" } else if dirty { "Есть несохранённые изменения" }
                else if st.applied_notice { "Сохранено" } else { "Нет изменений" };
            gx.text(msg, gx.small, false, x0 + 156.0, 510.0, rw - 156.0, 38.0,
                tx(if st.save_error { 0xFFFFB4A6 } else { C_DIM }));
        }
        _ => {
            gx.fill_rr(tx(C_PILL), cx - 28.0, 112.0 + yo, 56.0, 56.0, 10.0);
            gx.text("Q", gx.big, true, cx - 28.0, 112.0 + yo, 56.0, 56.0, tx(C_TEXT));
            gx.text("Q-Tiler", gx.big, true, sw, 224.0 + yo, cw, 40.0, tx(C_TEXT));
            gx.text("v1.0.0", gx.body, true, sw, 268.0 + yo, cw, 22.0, tx(C_DIM));
            gx.text("Rust + WinAPI + GDI+", gx.body, true, sw, 308.0 + yo, cw, 22.0, tx(C_TEXT));
            gx.text("Менеджер окон для Windows", gx.small, true, sw, 336.0 + yo, cw, 20.0, tx(C_DIM));
        }
    }
}


unsafe extern "system" fn settings_wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        let sp = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SettingsState;

        match msg {
            WM_ERASEBKGND => return LRESULT(1),

            WM_SHOWWINDOW => {
                if !sp.is_null() {
                    let s = &mut *sp;
                    if wparam.0 != 0 {
                        s.startup = startup_status();
                        s.startup_error = false;
                        s.anim_t = 0.0;
                        s.intro_done = false;
                        s.tab_fade = 0.0;
                        s.last = Instant::now();
                        s.base_x = (s.screen_w - WIN_W) / 2;
                        s.base_y = (s.screen_h - WIN_H) / 2;
                        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 0, LWA_ALPHA);
                        let _ = SetWindowPos(hwnd, None, s.base_x, s.base_y + 24, 0, 0,
                            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
                        let _ = SetTimer(Some(hwnd), TIMER_ID, 16, None);
                    } else {
                        let _ = KillTimer(Some(hwnd), TIMER_ID);
                    }
                }
                return LRESULT(0);
            }

            WM_TIMER => {
                if wparam.0 == TIMER_ID && !sp.is_null() {
                    tick(hwnd, &mut *sp);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    let _ = UpdateWindow(hwnd);
                }
                return LRESULT(0);
            }

            WM_NCHITTEST => {
                if !sp.is_null() {
                    let mut pt = POINT {
                        x: (lparam.0 & 0xFFFF) as i16 as i32,
                        y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
                    };
                    let _ = ScreenToClient(hwnd, &mut pt);
                    let (x, y) = (pt.x as f32, pt.y as f32);
                    if y >= 0.0 && y < 64.0 && hit_test(&*sp, x, y) < 0 {
                        return LRESULT(HTCAPTION as isize);
                    }
                }
                return LRESULT(HTCLIENT as isize);
            }

            WM_SETCURSOR => {
                if (lparam.0 & 0xFFFF) as u32 == HTCLIENT {
                    let hand = !sp.is_null() && (*sp).hover >= 0;
                    if let Ok(c) = LoadCursorW(None, if hand { IDC_HAND } else { IDC_ARROW }) {
                        let _ = SetCursor(Some(c));
                    }
                    return LRESULT(1);
                }
            }

            WM_LBUTTONDOWN => {
                if !sp.is_null() {
                    let s = &mut *sp;
                    let x = (lparam.0 & 0xFFFF) as i16 as f32;
                    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
                    let hit = hit_test(s, x, y);
                    if (200..=235).contains(&hit) || (300..=305).contains(&hit) {
                        s.applied_notice = false;
                        s.save_error = false;
                    }
                    match hit {
                        i @ 0..=6 => {
                            let i = i as usize;
                            if i != s.tab { s.tab = i; s.tab_fade = 0.0; }
                        }
                        100 => s.collapsed = !s.collapsed,
                        101 => { s.tab = 4; s.tab_fade = 0.0; }
                        102 => { s.tab = 1; s.tab_fade = 0.0; }
                        110 => { let _ = ShowWindow(hwnd, SW_HIDE); }
                        500 => {
                            s.startup = startup_status();
                            if s.startup == StartupStatus::Error {
                                s.startup_error = true;
                            } else {
                                let enable = s.startup != StartupStatus::On;
                                s.startup_error = set_startup(enable).is_err();
                                s.startup = startup_status();
                            }
                        }
                        i @ 200..=202 => {
                            let i = (i - 200) as usize;
                            s.prefs.theme = i;
                            s.prefs.color_a = THEMES[i].1;
                            s.prefs.color_b = THEMES[i].2;
                        }
                        i @ 210..=213 => {
                            s.prefs.duration_ms = [10, 250, 500, 1000][(i - 210) as usize];
                        }
                        214 => { s.prefs.duration_ms = s.prefs.duration_ms.saturating_sub(10).max(10); }
                        215 => { s.prefs.duration_ms = (s.prefs.duration_ms + 10).min(5000); }
                        i @ 220..=225 => { s.prefs.color_a = (i - 220) as usize; s.prefs.theme = 3; }
                        i @ 230..=235 => { s.prefs.color_b = (i - 230) as usize; s.prefs.theme = 3; }
                        i @ 300..=303 => s.prefs.tile_duration_ms = [10, 100, 250, 1000][(i - 300) as usize],
                        304 => s.prefs.tile_duration_ms = s.prefs.tile_duration_ms.saturating_sub(10).max(10),
                        305 => s.prefs.tile_duration_ms = (s.prefs.tile_duration_ms + 10).min(5000),
                        i @ 400..=402 => {
                            let index = (i - 400) as usize;
                            s.applied_notice = false;
                            s.save_error = false;
                            if s.select_font(index) {
                                s.prefs.font = index;
                                s.font_missing = false;
                            } else {
                                s.font_missing = true;
                            }
                        }
                        240 => s.apply(),
                        _ => {}
                    }
                }
                return LRESULT(0);
            }

            WM_KEYDOWN => {
                if !sp.is_null() {
                    let s = &mut *sp;
                    let vk = wparam.0 as u16;
                    if vk == VK_ESCAPE.0 {
                        let _ = ShowWindow(hwnd, SW_HIDE);
                    } else if vk == VK_DOWN.0 {
                        s.tab = (s.tab + 1) % NAV.len();
                        s.tab_fade = 0.0;
                    } else if vk == VK_UP.0 {
                        s.tab = (s.tab + NAV.len() - 1) % NAV.len();
                        s.tab_fade = 0.0;
                    }
                }
                return LRESULT(0);
            }

            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                if !sp.is_null() { paint(&*sp, hdc); }
                let _ = EndPaint(hwnd, &ps);
                return LRESULT(0);
            }

            _ => {}
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}
