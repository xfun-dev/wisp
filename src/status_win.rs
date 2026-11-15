//! `wisp.exe status`: the Windows status window (RULES.md U1, U5, P8, B5). Opened from
//! the Start menu, it reads the client's status file once a second and quits when closed.
//! It is the Mac window's paper and ink: what is going on, a line drawing of this
//! computer above the Mac (or the Mac's pairing code being typed), the switch for the
//! lock screen and UAC prompts (W1), and the ways out. Pairing runs on a worker thread;
//! the background client picks up the saved key on its own.
//!
//! Drawing is GDI+ into a cached bitmap, redrawn only when something on it changes.
//! Nothing moves but the caret while a code is typed, so an open window costs no CPU.
//! Light or dark follows the Windows app setting, title bar included.

use crate::i18n::{tr, trf, Lang, T};
use crate::client_win::RUNNING_MUTEX;
use crate::gdiplus_win::{self as gp, rgb, rgba, Align, Canvas, Font, Path};
use crate::logic::pairing::typed;
use crate::logic::press::Press;
use crate::logic::status::{describe, Light, Tray, View};
use crate::win32::{app_icon, instance, register, wide, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, Hwnd, Msg, ShowWindow, TranslateMessage};
use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr;
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

type Hdc = *mut c_void;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct PaintStruct {
    hdc: Hdc,
    erase: i32,
    paint: Rect,
    restore: i32,
    inc_update: i32,
    reserved: [u8; 32],
}

#[repr(C)]
struct TrackMouse {
    size: u32,
    flags: u32,
    hwnd: Hwnd,
    hover_time: u32,
}

#[link(name = "user32")]
extern "system" {
    fn PostQuitMessage(code: i32);
    fn SetTimer(hwnd: Hwnd, id: usize, ms: u32, func: *mut c_void) -> usize;
    fn KillTimer(hwnd: Hwnd, id: usize) -> i32;
    fn InvalidateRect(hwnd: Hwnd, rect: *const Rect, erase: i32) -> i32;
    fn BeginPaint(hwnd: Hwnd, ps: *mut PaintStruct) -> Hdc;
    fn EndPaint(hwnd: Hwnd, ps: *const PaintStruct) -> i32;
    fn LoadCursorW(instance: *mut c_void, name: *const u16) -> *mut c_void;
    fn SetCursor(cursor: *mut c_void) -> *mut c_void;
    fn GetDpiForSystem() -> u32;
    fn AdjustWindowRectEx(rect: *mut Rect, style: u32, menu: i32, ex_style: u32) -> i32;
    fn SetProcessDPIAware() -> i32;
    fn PostMessageW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> i32;
    fn FindWindowW(class: *const u16, title: *const u16) -> Hwnd;
    fn SetCapture(hwnd: Hwnd) -> Hwnd;
    fn ReleaseCapture() -> i32;
    fn TrackMouseEvent(track: *mut TrackMouse) -> i32;
    fn GetDC(hwnd: Hwnd) -> Hdc;
    fn ReleaseDC(hwnd: Hwnd, hdc: Hdc) -> i32;
}

#[link(name = "gdi32")]
extern "system" {
    fn CreateCompatibleDC(hdc: Hdc) -> Hdc;
    fn CreateCompatibleBitmap(hdc: Hdc, w: i32, h: i32) -> *mut c_void;
    fn SelectObject(hdc: Hdc, obj: *mut c_void) -> *mut c_void;
    fn DeleteObject(obj: *mut c_void) -> i32;
    fn DeleteDC(hdc: Hdc) -> i32;
    fn BitBlt(dst: Hdc, x: i32, y: i32, w: i32, h: i32, src: Hdc, sx: i32, sy: i32, rop: u32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn OpenMutexW(access: u32, inherit: i32, name: *const u16) -> *mut c_void;
    fn CloseHandle(handle: *mut c_void) -> i32;
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

#[link(name = "advapi32")]
extern "system" {
    fn RegGetValueW(key: isize, sub: *const u16, value: *const u16, flags: u32, kind: *mut u32, data: *mut c_void, size: *mut u32) -> i32;
}

type SetWindowAttribute = unsafe extern "system" fn(Hwnd, u32, *const c_void, u32) -> i32;

const WM_DESTROY: u32 = 0x0002;
const WM_CLOSE: u32 = 0x0010;
const WM_PAINT: u32 = 0x000F;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_SETCURSOR: u32 = 0x0020;
const WM_SETTINGCHANGE: u32 = 0x001A;
const WM_CHAR: u32 = 0x0102;
const WM_TIMER: u32 = 0x0113;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_CAPTURECHANGED: u32 = 0x0215;
const WM_PAIRED: u32 = 0x8000 + 1;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const CW_USEDEFAULT: i32 = 0x8000_0000u32 as i32;
const SW_SHOW: i32 = 5;
const IDC_ARROW: usize = 32512;
const IDC_HAND: usize = 32649;
const SYNCHRONIZE: u32 = 0x0010_0000;
const SRCCOPY: u32 = 0x00CC_0020;
const TME_LEAVE: u32 = 0x0002;
const HTCLIENT: isize = 1;
const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
const RRF_RT_REG_DWORD: u32 = 0x10;
const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
const DWMWA_CAPTION_COLOR: u32 = 35;
const DWMWA_TEXT_COLOR: u32 = 36;
const STYLE: u32 = WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
const STATUS_TIMER: usize = 1;
const CARET_TIMER: usize = 2;
/// how long the switch waits for the administrator's answer before showing the truth again
const SERVICE_WAIT_SECS: u64 = 90;

/// The client area in logical pixels, and where things sit in it.
const W: f32 = 400.0;
const H: f32 = 440.0;
const PAD: f32 = 28.0;
const MID: f32 = 128.0;
const MID_H: f32 = 176.0;
const ROW: f32 = MID + MID_H;
const FOOT: f32 = H - 48.0;
/// the primary button, right of the headline
const BUTTON: (f32, f32, f32, f32) = (W - PAD - 100.0, 44.0, 100.0, 32.0);
/// the lock-screen switch
const SWITCH: (f32, f32, f32, f32) = (W - PAD - 40.0, ROW + 26.0, 40.0, 22.0);

/// Paper and ink, one set for each appearance (the Mac's, RULES.md B4).
#[derive(Clone, Copy)]
struct Ink {
    paper: u32,
    text: u32,
    soft: u32,
    faint: u32,
    rule: u32,
    ghost: u32,
    sheet: u32,
    celadon: u32,
    ochre: u32,
    rust: u32,
}

const LIGHT: Ink = Ink {
    paper: 0xF3EFE6,
    text: 0x23211D,
    soft: 0x635E55,
    faint: 0x8C867A,
    rule: 0xDDD5C6,
    ghost: 0xB9B1A1,
    sheet: 0xF8F5EE,
    celadon: 0x4E7068,
    ochre: 0x8F6326,
    rust: 0xA8452F,
};

const DARK: Ink = Ink {
    paper: 0x1C1B19,
    text: 0xE9E4DA,
    soft: 0xA9A397,
    faint: 0x7D776C,
    rule: 0x34312C,
    ghost: 0x4A463F,
    sheet: 0x23211E,
    celadon: 0x8DB3A8,
    ochre: 0xD2A266,
    rust: 0xE08A72,
};

/// Whether Windows apps are set to dark.
fn dark_mode() -> bool {
    let (key, value) = (wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"), wide("AppsUseLightTheme"));
    let mut light = 1u32;
    let mut size = 4u32;
    let ok = unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_DWORD, ptr::null_mut(), &mut light as *mut u32 as *mut c_void, &mut size) } == 0;
    ok && light == 0
}

/// 0xRRGGBB as a COLORREF (0x00BBGGRR).
fn colorref(c: u32) -> u32 {
    (c & 0xFF) << 16 | (c & 0xFF00) | (c >> 16 & 0xFF)
}

/// Colours the title bar paper (Windows 11; older versions ignore it). dwmapi.dll is
/// loaded here, not linked, so the background client, which shares this exe, never
/// loads it (RULES.md R1).
unsafe fn tint_title_bar(hwnd: Hwnd, ink: &Ink, dark: bool) {
    // from System32 only, never a look-alike next to the exe
    let dll = LoadLibraryExW(wide("dwmapi.dll").as_ptr(), ptr::null_mut(), 0x800);
    if dll.is_null() {
        return;
    }
    let f = GetProcAddress(dll, b"DwmSetWindowAttribute\0".as_ptr());
    if f.is_null() {
        return;
    }
    // SAFETY: DwmSetWindowAttribute has this signature
    let set: SetWindowAttribute = std::mem::transmute(f);
    let (on, caption, text) = (dark as u32, colorref(ink.paper), colorref(ink.text));
    set(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &on as *const u32 as *const c_void, 4);
    set(hwnd, DWMWA_CAPTION_COLOR, &caption as *const u32 as *const c_void, 4);
    set(hwnd, DWMWA_TEXT_COLOR, &text as *const u32 as *const c_void, 4);
}

fn client_running() -> bool {
    let name = wide(RUNNING_MUTEX);
    let handle = unsafe { OpenMutexW(SYNCHRONIZE, 0, name.as_ptr()) };
    if handle.is_null() {
        return false;
    }
    unsafe { CloseHandle(handle) };
    true
}

fn dpi_scale() -> f32 {
    unsafe { GetDpiForSystem() as f32 / 96.0 }
}

fn current_view() -> View {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    describe(crate::client_win::read_status().as_deref(), client_running(), now)
}

/// What the last pairing attempt said, and whether it worked.
type Outcome = (String, bool);

/// Handed from the pairing thread back to the window.
static PAIRED: Mutex<Option<Outcome>> = Mutex::new(None);

/// Things that react to the pointer.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
    /// the outlined button: pair, or start Wisp
    Go,
    /// "Pair with Another Mac" while connected
    Relink,
    /// back out of re-pairing
    Cancel,
    /// the lock-screen switch (RULES.md W1)
    Service,
    Quit,
}

struct Fonts {
    label: Font,
    head: Font,
    body: Font,
    mono: Font,
    button: Font,
    row: Font,
    digit: Font,
    drawing: Font,
}

impl Fonts {
    /// Each language in a face Windows ships that has its letters (GDI+ does not
    /// fall back to another font on its own): Hangul is not in YaHei, for one.
    fn new() -> Fonts {
        let face = match crate::i18n::lang() {
            Lang::ZhHans => "Microsoft YaHei UI",
            Lang::ZhHant => "Microsoft JhengHei UI",
            Lang::Ja => "Yu Gothic UI",
            Lang::Ko => "Malgun Gothic",
            Lang::En => "Segoe UI",
        };
        Fonts {
            label: Font::new(face, 12.0, false),
            head: Font::new(face, 21.0, true),
            body: Font::new(face, 12.0, false),
            mono: Font::new("Consolas", 12.5, false),
            button: Font::new(face, 13.0, true),
            row: Font::new(face, 14.0, false),
            digit: Font::new(face, 28.0, false),
            drawing: Font::new(face, 12.0, true),
        }
    }
}

/// The picture, kept between paints.
struct Cache {
    dc: Hdc,
    bitmap: *mut c_void,
    old: *mut c_void,
    w: i32,
    h: i32,
}

impl Cache {
    unsafe fn new(w: i32, h: i32) -> Cache {
        let screen = GetDC(ptr::null_mut());
        let dc = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, w, h);
        ReleaseDC(ptr::null_mut(), screen);
        let old = SelectObject(dc, bitmap);
        Cache { dc, bitmap, old, w, h }
    }
}

impl Drop for Cache {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

struct Ui {
    fonts: Fonts,
    ink: Ink,
    view: View,
    /// WispService installed: the lock screen and UAC prompts are reachable (W1)
    service: bool,
    /// asked to switch the service to this, and when; shown until it is so
    service_asked: Option<(bool, Instant)>,
    /// digits typed so far
    code: String,
    busy: bool,
    outcome: Option<Outcome>,
    /// connected, but the user asked to pair again
    repairing: bool,
    hover: Option<Hit>,
    pressed: Press<Hit>,
    caret_on: bool,
    scale: f32,
    /// the picture, and whether it needs drawing again
    still: Option<Cache>,
    dirty: bool,
}

impl Ui {
    fn pairing(&self) -> bool {
        matches!(self.view.tray, Tray::Pair { .. }) || self.repairing
    }

    fn connected(&self) -> bool {
        matches!(self.view.tray, Tray::Connected { .. })
    }

    fn stopped(&self) -> bool {
        matches!(self.view.tray, Tray::Stopped)
    }

    fn ready(&self) -> bool {
        self.code.len() == 6 && !self.busy
    }

    /// what the switch shows: the request while it waits, else the truth
    fn service_shown(&self) -> bool {
        self.service_asked.map_or(self.service, |(on, _)| on)
    }
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

fn with_ui<T>(f: impl FnOnce(&mut Ui) -> T) -> Option<T> {
    UI.with(|ui| ui.borrow_mut().as_mut().map(f))
}

// MARK: - words

/// The small coloured line, the big one, and the line under it.
fn words(ui: &Ui) -> (String, String, String) {
    let v = &ui.view;
    match &v.tray {
        Tray::Connected { since, host } if !ui.repairing => {
            (trf(T::ConnectedFor, &[since]), v.who.clone(), host.clone())
        }
        Tray::Connected { .. } => (tr(T::Connected).into(), tr(T::AnotherMac).into(), tr(T::Replaces).into()),
        Tray::Pair { fresh: true, .. } => (tr(T::Unpaired).into(), tr(T::PairThisPc).into(), tr(T::PairHowTo).into()),
        Tray::Pair { fresh: false, .. } => (tr(T::Searching).into(), tr(T::SearchingMac).into(), v.detail.clone()),
        Tray::Stopped => (tr(T::Stopped).into(), "Wisp".into(), v.detail.clone()),
    }
}

fn dot(ui: &Ui) -> u32 {
    match ui.view.light {
        Light::Green => ui.ink.celadon,
        Light::Orange if matches!(ui.view.tray, Tray::Pair { fresh: true, .. }) => ui.ink.faint,
        Light::Orange => ui.ink.ochre,
        Light::Red => ui.ink.rust,
    }
}

// MARK: - geometry

fn button(ui: &Ui) -> Option<&'static str> {
    if ui.stopped() {
        Some(tr(T::StartWisp))
    } else if ui.pairing() {
        Some(if ui.busy { tr(T::PairingNow) } else { tr(T::Pair) })
    } else {
        None
    }
}

/// The footer's left link, if any, and its width.
fn left_link(ui: &Ui) -> Option<(&'static str, Hit, f32)> {
    if ui.repairing {
        Some((tr(T::Cancel), Hit::Cancel, 80.0))
    } else if ui.connected() {
        Some((tr(T::PairAnother), Hit::Relink, 200.0))
    } else {
        None
    }
}

fn hit_test(ui: &Ui, x: f32, y: f32) -> Option<Hit> {
    let inside = |(bx, by, bw, bh): (f32, f32, f32, f32)| x >= bx && x <= bx + bw && y >= by && y <= by + bh;
    if button(ui).is_some() && inside(BUTTON) {
        return Some(Hit::Go);
    }
    // the whole row toggles, not just the knob
    if inside((0.0, ROW, W, FOOT - ROW)) {
        return Some(Hit::Service);
    }
    if let Some((_, hit, w)) = left_link(ui) {
        if inside((PAD - 4.0, FOOT + 8.0, w + 8.0, 28.0)) {
            return Some(hit);
        }
    }
    if inside((W - PAD - 120.0, FOOT + 8.0, 124.0, 28.0)) {
        return Some(Hit::Quit);
    }
    None
}

fn slot_x(i: usize) -> f32 {
    let width = 6.0 * 30.0 + 5.0 * 12.0 + 12.0;
    (W - width) / 2.0 + i as f32 * 42.0 + if i >= 3 { 12.0 } else { 0.0 }
}

const SLOT_Y: f32 = MID + 56.0;

// MARK: - drawing

fn hairline(c: &Canvas, y: f32, color: u32) {
    c.fill(&Path::poly(&[(0.0, y), (W, y), (W, y + 1.0), (0.0, y + 1.0)]), rgb(color));
}

fn draw(c: &Canvas, ui: &Ui) {
    let ink = ui.ink;
    c.fill(&Path::poly(&[(0.0, 0.0), (W, 0.0), (W, H), (0.0, H)]), rgb(ink.paper));

    // the header: a coloured word, the name, the line under it, the button
    let (label, head, line) = words(ui);
    let d = dot(ui);
    c.fill(&Path::ellipse(PAD, 23.0, 7.0, 7.0), rgb(d));
    c.text(&label, &ui.fonts.label, rgb(d), PAD + 13.0, 16.0, 250.0, 20.0, Align::Left);
    c.text(&head, &ui.fonts.head, rgb(ink.text), PAD - 1.0, 40.0, if button(ui).is_some() { 240.0 } else { W - 2.0 * PAD }, 34.0, Align::Left);
    let mono = ui.connected() && !ui.repairing;
    c.paragraph(&line, if mono { &ui.fonts.mono } else { &ui.fonts.body }, rgb(ink.soft), PAD, 78.0, if button(ui).is_some() { 240.0 } else { W - 2.0 * PAD }, 34.0);
    // the developer bridge is never open unnoticed (RULES.md X5)
    if let Some(note) = &ui.view.bridge {
        c.text(note, &ui.fonts.body, rgb(ink.faint), PAD, 98.0, 240.0, 18.0, Align::Left);
    }
    if let Some(words) = button(ui) {
        let (bx, by, bw, bh) = BUTTON;
        let enabled = !ui.pairing() || ui.ready() || ui.stopped();
        let down = enabled && ui.pressed.held() == Some(Hit::Go) && ui.hover == Some(Hit::Go);
        let pill = Path::round_rect(bx, by, bw, bh, bh / 2.0);
        let line = if enabled { ink.text } else { ink.ghost };
        if down {
            c.fill(&pill, rgb(ink.text));
        } else if enabled && ui.hover == Some(Hit::Go) {
            c.fill(&pill, rgba(ink.text, 0.06));
        }
        c.stroke(&Path::round_rect(bx + 0.5, by + 0.5, bw - 1.0, bh - 1.0, bh / 2.0), rgb(line), 1.0);
        c.text(words, &ui.fonts.button, rgb(if down { ink.paper } else { line }), bx, by, bw, bh, Align::Center);
    }
    hairline(c, MID - 1.0, ink.rule);

    // the middle: the drawing, the code being typed, or why nothing works
    if ui.stopped() {
        c.paragraph_centered(tr(T::BackendStopped), &ui.fonts.body, rgb(ink.soft), PAD + 12.0, MID + MID_H / 2.0 - 18.0, W - 2.0 * PAD - 24.0, 40.0);
    } else if ui.pairing() {
        draw_code(c, ui);
    } else {
        draw_desk(c, ui);
    }
    hairline(c, ROW, ink.rule);

    // the lock-screen switch (RULES.md W1)
    let on = ui.service_shown();
    c.text(tr(T::LockSwitch), &ui.fonts.row, rgb(ink.text), PAD, ROW + 16.0, W - 2.0 * PAD - 52.0, 22.0, Align::Left);
    let note = if ui.service_asked.is_some() { tr(T::WaitingAdmin) } else { tr(T::AdminOnce) };
    c.text(note, &ui.fonts.body, rgb(ink.soft), PAD, ROW + 40.0, W - 2.0 * PAD - 52.0, 20.0, Align::Left);
    let (sx, sy, sw, sh) = SWITCH;
    let track = Path::round_rect(sx, sy, sw, sh, sh / 2.0);
    if on {
        c.fill(&track, rgb(ink.text));
        c.fill(&Path::ellipse(sx + sw - sh + 3.0, sy + 3.0, sh - 6.0, sh - 6.0), rgb(ink.paper));
    } else {
        c.stroke(&Path::round_rect(sx + 0.5, sy + 0.5, sw - 1.0, sh - 1.0, sh / 2.0), rgb(if ui.hover == Some(Hit::Service) { ink.text } else { ink.faint }), 1.0);
        c.fill(&Path::ellipse(sx + 4.0, sy + 4.0, sh - 8.0, sh - 8.0), rgb(ink.faint));
    }
    hairline(c, FOOT, ink.rule);

    // the footer: re-pair or cancel on the left, quit on the right
    let link = |c: &Canvas, s: &str, hit: Hit, x: f32, w: f32, align: Align| {
        let color = if ui.hover == Some(hit) { ink.text } else { ink.soft };
        c.text(s, &ui.fonts.body, rgb(color), x, FOOT + 10.0, w, 24.0, align);
        if ui.hover == Some(hit) {
            let (ux, uw) = match align {
                Align::Right => (x + w - w.min(56.0), w.min(56.0)),
                _ => (x, w),
            };
            c.fill(&Path::poly(&[(ux, FOOT + 31.0), (ux + uw, FOOT + 31.0), (ux + uw, FOOT + 32.0), (ux, FOOT + 32.0)]), rgb(ink.text));
        }
    };
    if let Some((s, hit, w)) = left_link(ui) {
        link(c, s, hit, PAD, w, Align::Left);
    }
    link(c, tr(T::Quit), Hit::Quit, W - PAD - 120.0, 120.0, Align::Right);
}

/// This computer above the Mac, in one line each, with the wisp between them.
fn draw_desk(c: &Canvas, ui: &Ui) {
    let ink = ui.ink;
    let live = ui.connected();
    let (mx, my, mw, mh) = (148.0, MID + 12.0, 104.0, 60.0);
    c.fill(&Path::round_rect(mx, my, mw, mh, 4.0), rgb(ink.sheet));
    c.stroke(&Path::round_rect(mx, my, mw, mh, 4.0), rgb(ink.text), 1.2);
    c.text(tr(T::ThisPc), &ui.fonts.drawing, rgb(ink.text), mx, my, mw, mh, Align::Center);
    let (lx, ly, lw, lh) = (161.0, MID + 92.0, 78.0, 44.0);
    c.fill(&Path::round_rect(lx, ly, lw, lh, 3.0), rgb(ink.sheet));
    c.stroke(&Path::round_rect(lx, ly, lw, lh, 3.0), rgb(ink.text), 1.2);
    c.text("Mac", &ui.fonts.drawing, rgb(ink.soft), lx, ly, lw, lh, Align::Center);
    let base = Path::poly(&[(lx - 14.0, ly + lh + 4.0), (lx + lw + 14.0, ly + lh + 4.0), (lx + lw + 7.0, ly + lh + 10.0), (lx - 7.0, ly + lh + 10.0)]);
    c.stroke(&base, rgb(ink.text), 1.2);
    let smoke = Path::new().move_to(200.0, ly - 3.0).curve_to((193.0, ly - 10.0), (208.0, my + mh + 12.0), 200.0, my + mh + 4.0);
    c.stroke(&smoke, rgb(if live { ink.celadon } else { ink.ghost }), 1.6);
    c.text(tr(T::SetOnMac), &ui.fonts.body, rgb(ink.faint), 0.0, MID + MID_H - 24.0, W, 20.0, Align::Center);
}

/// The Mac's code being typed: six figures over hairlines, the caret where the next goes.
fn draw_code(c: &Canvas, ui: &Ui) {
    let ink = ui.ink;
    let prompt = match &ui.view.tray {
        Tray::Pair { prompt, .. } => *prompt,
        _ => tr(T::EnterNewCode),
    };
    let (said, color) = match (&ui.outcome, ui.busy) {
        (_, true) => (tr(T::Checking), ink.soft),
        (Some((said, ok)), false) => (said.as_str(), if *ok { ink.celadon } else { ink.rust }),
        (None, false) => (prompt, ink.soft),
    };
    c.text(said, &ui.fonts.body, rgb(color), PAD, MID + 18.0, W - 2.0 * PAD, 20.0, Align::Center);
    let digits: Vec<char> = ui.code.chars().collect();
    for i in 0..6 {
        let x = slot_x(i);
        let line = if i < digits.len() || i == digits.len() { ink.text } else { ink.ghost };
        c.fill(&Path::poly(&[(x, SLOT_Y + 44.0), (x + 30.0, SLOT_Y + 44.0), (x + 30.0, SLOT_Y + 45.0), (x, SLOT_Y + 45.0)]), rgb(line));
        if let Some(ch) = digits.get(i) {
            c.text(&ch.to_string(), &ui.fonts.digit, rgb(ink.text), x - 4.0, SLOT_Y, 38.0, 42.0, Align::Center);
        }
    }
    if ui.caret_on && !ui.busy && digits.len() < 6 {
        let x = slot_x(digits.len()) + 14.0;
        c.fill(&Path::poly(&[(x, SLOT_Y + 10.0), (x + 1.5, SLOT_Y + 10.0), (x + 1.5, SLOT_Y + 38.0), (x, SLOT_Y + 38.0)]), rgb(ink.text));
    }
    c.text(tr(T::TypeDigits), &ui.fonts.body, rgb(ink.faint), 0.0, MID + MID_H - 30.0, W, 20.0, Align::Center);
}

fn caret_rect(scale: f32) -> Rect {
    Rect {
        left: (slot_x(0) * scale) as i32 - 4,
        top: (SLOT_Y * scale) as i32,
        right: ((slot_x(5) + 40.0) * scale) as i32 + 4,
        bottom: ((SLOT_Y + 46.0) * scale) as i32 + 2,
    }
}

unsafe fn paint(hwnd: Hwnd) {
    let mut ps = std::mem::zeroed::<PaintStruct>();
    let hdc = BeginPaint(hwnd, &mut ps);
    with_ui(|ui| {
        let (w, h) = ((W * ui.scale).ceil() as i32, (H * ui.scale).ceil() as i32);
        if ui.still.as_ref().is_none_or(|c| c.w != w || c.h != h) {
            ui.still = Some(Cache::new(w, h));
            ui.dirty = true;
        }
        let still = ui.still.as_ref().unwrap().dc;
        if ui.dirty {
            ui.dirty = false;
            let c = Canvas::on(still, ui.scale);
            draw(&c, ui);
        }
        let p = ps.paint;
        BitBlt(hdc, p.left, p.top, p.right - p.left, p.bottom - p.top, still, p.left, p.top, SRCCOPY);
    });
    EndPaint(hwnd, &ps);
}

/// Something changed: draw it again on the next paint, and blink the caret only while
/// a code is being typed.
unsafe fn refresh(hwnd: Hwnd) {
    let typing = with_ui(|ui| {
        ui.dirty = true;
        ui.pairing() && !ui.busy && ui.code.len() < 6
    });
    if typing == Some(true) {
        SetTimer(hwnd, CARET_TIMER, 530, ptr::null_mut());
    } else {
        KillTimer(hwnd, CARET_TIMER);
    }
    InvalidateRect(hwnd, ptr::null(), 0);
}

unsafe fn start_pairing(hwnd: Hwnd) {
    let Some(code) = with_ui(|ui| if ui.ready() { typed(&ui.code) } else { None }).flatten() else { return };
    with_ui(|ui| {
        ui.busy = true;
        ui.outcome = None;
    });
    refresh(hwnd);
    let window = hwnd as usize;
    std::thread::spawn(move || {
        let outcome = match crate::client_win::pair(&code, crate::proto::DEFAULT_PORT) {
            Ok(name) => (trf(T::PairedConnecting, &[&name]), true),
            Err(e) => (e.to_string(), false),
        };
        *PAIRED.lock().unwrap() = Some(outcome);
        unsafe { PostMessageW(window as Hwnd, WM_PAIRED, 0, 0) };
    });
}

/// Quitting from here does what the tray's "Quit Wisp" does: the client exits and closes this window.
unsafe fn quit(hwnd: Hwnd) {
    let tray = FindWindowW(wide("WispTray").as_ptr(), ptr::null());
    if !tray.is_null() {
        PostMessageW(tray, crate::shell_win::WM_QUIT_WISP, 0, 0);
    }
    PostMessageW(hwnd, WM_CLOSE, 0, 0);
}

fn start_client() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("client").spawn();
    }
}

unsafe fn act(hwnd: Hwnd, hit: Hit) {
    match hit {
        Hit::Go => {
            if with_ui(|ui| ui.stopped()) == Some(true) {
                start_client();
            } else {
                start_pairing(hwnd);
            }
        }
        Hit::Relink => {
            with_ui(|ui| {
                ui.repairing = true;
                ui.code.clear();
                ui.outcome = None;
            });
        }
        Hit::Cancel => {
            with_ui(|ui| {
                ui.repairing = false;
                ui.code.clear();
                ui.outcome = None;
            });
        }
        Hit::Service => {
            // one request at a time; the UAC prompt answers it (RULES.md W1)
            if let Some(on) = with_ui(|ui| {
                if ui.service_asked.is_some() {
                    return None;
                }
                let on = !ui.service;
                ui.service_asked = Some((on, Instant::now()));
                Some(on)
            })
            .flatten()
            {
                std::thread::spawn(move || crate::shell_win::switch_service(on));
            }
        }
        Hit::Quit => quit(hwnd),
    }
    refresh(hwnd);
}

fn logical(lparam: isize, scale: f32) -> (f32, f32) {
    let (x, y) = ((lparam & 0xFFFF) as i16 as f32, ((lparam >> 16) & 0xFFFF) as i16 as f32);
    (x / scale, y / scale)
}

/// Follows the Windows app setting: paper or ink, title bar included.
unsafe fn apply_theme(hwnd: Hwnd) {
    let dark = dark_mode();
    if let Some(ink) = with_ui(|ui| {
        ui.ink = if dark { DARK } else { LIGHT };
        ui.ink
    }) {
        tint_title_bar(hwnd, &ink, dark);
    }
}

unsafe extern "system" fn wnd_proc(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_SETTINGCHANGE => {
            apply_theme(hwnd);
            refresh(hwnd);
            0
        }
        WM_TIMER if wparam == STATUS_TIMER => {
            let view = current_view();
            let service = crate::service_win::installed();
            let changed = with_ui(|ui| {
                let mut changed = false;
                if ui.view != view {
                    if matches!(view.tray, Tray::Connected { .. }) && !matches!(ui.view.tray, Tray::Connected { .. }) {
                        // just paired: back to the drawing
                        ui.repairing = false;
                        ui.code.clear();
                        ui.outcome = None;
                    }
                    ui.view = view;
                    changed = true;
                }
                if ui.service != service {
                    ui.service = service;
                    changed = true;
                }
                // answered (or refused, or left too long): show the truth again
                if let Some((on, at)) = ui.service_asked {
                    if on == service || at.elapsed().as_secs() > SERVICE_WAIT_SECS {
                        ui.service_asked = None;
                        changed = true;
                    }
                }
                changed
            });
            if changed == Some(true) {
                refresh(hwnd);
            }
            0
        }
        WM_TIMER if wparam == CARET_TIMER => {
            if let Some(scale) = with_ui(|ui| {
                ui.caret_on = !ui.caret_on;
                ui.dirty = true;
                ui.scale
            }) {
                InvalidateRect(hwnd, &caret_rect(scale), 0);
            }
            0
        }
        WM_CAPTURECHANGED => {
            // pressed and then dragged away by Alt+Tab or the like: let go
            if with_ui(|ui| ui.pressed.lost()) == Some(true) {
                refresh(hwnd);
            }
            0
        }
        WM_MOUSEMOVE => {
            let mut track = TrackMouse { size: std::mem::size_of::<TrackMouse>() as u32, flags: TME_LEAVE, hwnd, hover_time: 0 };
            TrackMouseEvent(&mut track);
            let changed = with_ui(|ui| {
                let (x, y) = logical(lparam, ui.scale);
                let hit = hit_test(ui, x, y);
                let changed = hit != ui.hover;
                ui.hover = hit;
                changed
            });
            if changed == Some(true) {
                refresh(hwnd);
            }
            0
        }
        WM_MOUSELEAVE => {
            with_ui(|ui| ui.hover = None);
            refresh(hwnd);
            0
        }
        WM_SETCURSOR if (lparam & 0xFFFF) == HTCLIENT => {
            let on = with_ui(|ui| ui.hover.is_some_and(|h| h != Hit::Go || !ui.pairing() || ui.ready())).unwrap_or(false);
            SetCursor(LoadCursorW(ptr::null_mut(), (if on { IDC_HAND } else { IDC_ARROW }) as *const u16));
            1
        }
        WM_LBUTTONDOWN => {
            // capture first: taking it can send WM_CAPTURECHANGED, which must not cancel this press
            SetCapture(hwnd);
            with_ui(|ui| {
                let (x, y) = logical(lparam, ui.scale);
                let on = hit_test(ui, x, y);
                ui.pressed.down(on);
            });
            refresh(hwnd);
            0
        }
        WM_LBUTTONUP => {
            let clicked = with_ui(|ui| {
                let (x, y) = logical(lparam, ui.scale);
                let on = hit_test(ui, x, y);
                // ReleaseCapture sends WM_CAPTURECHANGED right here, which must not cancel this click
                ui.pressed.up(on, |_| {})
            })
            .flatten();
            ReleaseCapture();
            match clicked {
                Some(hit) => act(hwnd, hit),
                None => refresh(hwnd),
            }
            0
        }
        WM_CHAR => {
            let ch = char::from_u32(wparam as u32).unwrap_or('\0');
            let mut go = false;
            let mut cancel = false;
            with_ui(|ui| {
                if !ui.pairing() || ui.busy {
                    return;
                }
                // a Chinese IME in full-width mode types ０-９
                let ch = match ch {
                    '０'..='９' => char::from_u32(ch as u32 - '０' as u32 + '0' as u32).unwrap_or(ch),
                    _ => ch,
                };
                match ch {
                    '0'..='9' if ui.code.len() < 6 => {
                        ui.code.push(ch);
                        ui.outcome = None;
                        ui.caret_on = true;
                    }
                    '\u{8}' => {
                        ui.code.pop();
                        ui.caret_on = true;
                    }
                    '\r' if ui.code.len() == 6 => go = true,
                    '\r' => ui.outcome = Some((tr(T::CodeSix).into(), false)),
                    '\u{1b}' => cancel = ui.repairing,
                    _ => {}
                }
            });
            if go {
                start_pairing(hwnd);
            } else if cancel {
                act(hwnd, Hit::Cancel);
            } else {
                refresh(hwnd);
            }
            0
        }
        WM_PAIRED => {
            let outcome = PAIRED.lock().unwrap().take();
            with_ui(|ui| {
                ui.busy = false;
                if outcome.as_ref().is_some_and(|o| o.1) {
                    ui.code.clear();
                    // still connected (or back already): the drawing, not a stuck pairing
                    ui.repairing = false;
                }
                ui.outcome = outcome;
            });
            refresh(hwnd);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

pub fn run() {
    // opening Wisp from the Start menu means "make Wisp run": after quitting from the
    // tray, this is how it comes back without logging in again (RULES.md U5)
    if !client_running() {
        start_client();
    }
    if !gp::startup() {
        return;
    }
    unsafe {
        SetProcessDPIAware();
        let scale = dpi_scale();
        let class = wide("WispStatus");
        let title = wide("Wisp");
        register(&class, wnd_proc, LoadCursorW(ptr::null_mut(), IDC_ARROW as *const u16), ptr::null_mut(), app_icon());
        let mut size = Rect { left: 0, top: 0, right: (W * scale).ceil() as i32, bottom: (H * scale).ceil() as i32 };
        AdjustWindowRectEx(&mut size, STYLE, 0, 0);
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            STYLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            size.right - size.left,
            size.bottom - size.top,
            ptr::null_mut(),
            ptr::null_mut(),
            instance(),
            ptr::null_mut(),
        );
        if hwnd.is_null() {
            return;
        }
        UI.with(|ui| {
            *ui.borrow_mut() = Some(Ui {
                fonts: Fonts::new(),
                ink: LIGHT,
                view: current_view(),
                service: crate::service_win::installed(),
                service_asked: None,
                code: String::new(),
                busy: false,
                outcome: None,
                repairing: false,
                hover: None,
                pressed: Press::default(),
                caret_on: true,
                scale,
                still: None,
                dirty: true,
            })
        });
        apply_theme(hwnd);
        ShowWindow(hwnd, SW_SHOW);
        SetTimer(hwnd, STATUS_TIMER, 1000, ptr::null_mut());
        refresh(hwnd);
        let mut msg = std::mem::zeroed::<Msg>();
        while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
