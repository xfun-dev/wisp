//! `wisp.exe status`: the Windows status window (RULES.md U1, U5, P8, B4). Opened from
//! the Start menu, it reads the client's status file once a second and quits when closed.
//! It is the same desk as the Mac's settings window: this computer's screen on a monitor
//! arm, the Mac below it, and a drawer whose carved tray shows how long they have been
//! connected, or takes the Mac's pairing code. Pairing runs on a worker thread; the
//! background client picks up the saved key on its own.
//!
//! Drawing is GDI+ into a cached bitmap, redrawn only when something on it changes; a
//! 20 fps timer repaints just the small moving parts (the wisp, the steam, the caret).

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
use std::time::{SystemTime, UNIX_EPOCH};

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
    fn IsIconic(hwnd: Hwnd) -> i32;
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
}

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

type SetWindowAttribute = unsafe extern "system" fn(Hwnd, u32, *const c_void, u32) -> i32;

/// Colours the title bar (Windows 11). dwmapi.dll is loaded here, not linked, so the
/// background client, which shares this exe, never loads it (RULES.md R1).
unsafe fn tint_title_bar(hwnd: Hwnd) {
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
    let (caption, text) = (0x00D8E5EDu32, 0x0025374Au32);
    set(hwnd, DWMWA_CAPTION_COLOR, &caption as *const u32 as *const c_void, 4);
    set(hwnd, DWMWA_TEXT_COLOR, &text as *const u32 as *const c_void, 4);
}

const WM_DESTROY: u32 = 0x0002;
const WM_CLOSE: u32 = 0x0010;
const WM_PAINT: u32 = 0x000F;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_SETCURSOR: u32 = 0x0020;
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
const DWMWA_CAPTION_COLOR: u32 = 35;
const DWMWA_TEXT_COLOR: u32 = 36;
const STYLE: u32 = WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
const STATUS_TIMER: usize = 1;
const FRAME_TIMER: usize = 2;

/// The client area in logical pixels, and where things sit in it.
const W: f32 = 420.0;
const H: f32 = 568.0;
const SCENE_TOP: f32 = 42.0;
const DRAWER: f32 = 344.0;
const TRAY: (f32, f32, f32, f32) = (22.0, 368.0, 376.0, 124.0);
const FOOT: f32 = 510.0;
/// the desk drawing is in the Mac window's coordinates (RULES.md B4), shown from y = 38 down
const SCENE_SCALE: f32 = 420.0 / 460.0;

// the desk's colours, the same as the Mac's
const WALL: u32 = 0xEDE5D8;
const WOOD: u32 = 0xB8936A;
const WOOD_BAND: u32 = 0xA88A66;
const TRAY_FILL: u32 = 0xA47E54;
const ENGRAVE: u32 = 0x6E4F2E;
const CLAY: u32 = 0xF3E9DA;
const CLAY_SHADE: u32 = 0xD9C7AE;
const CLAY_DROP: u32 = 0x8D704F;
const TEAL: u32 = 0x3FA69A;
const TEAL_GLOW: u32 = 0x5FD6C6;
const CREAM: u32 = 0xFFF7EC;
const INK: u32 = 0x4A3725;
const INK_SOFT: u32 = 0x8C7660;
const AMBER: u32 = 0xD49A2A;
const RED: u32 = 0xC0503A;

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
    /// the clay handle: pair, or start Wisp
    Go,
    /// "重新配对" while connected
    Relink,
    /// back out of re-pairing
    Cancel,
    Quit,
}

struct Fonts {
    head: Font,
    body: Font,
    label: Font,
    value: Font,
    number: Font,
    digit: Font,
    link: Font,
    handle: Font,
    screen: Font,
    mac: Font,
    notice: Font,
}

impl Fonts {
    fn new() -> Fonts {
        let (yahei, segoe) = ("Microsoft YaHei UI", "Segoe UI");
        Fonts {
            head: Font::new(yahei, 17.0, true),
            body: Font::new(yahei, 12.0, false),
            label: Font::new(yahei, 11.0, true),
            value: Font::new(yahei, 19.0, true),
            number: Font::new(segoe, 19.0, true),
            digit: Font::new(segoe, 22.0, true),
            link: Font::new(yahei, 12.0, true),
            handle: Font::new(yahei, 14.0, true),
            screen: Font::new(yahei, 17.0, true),
            mac: Font::new(segoe, 13.0, true),
            notice: Font::new(yahei, 16.0, true),
        }
    }
}

/// The static picture, kept between paints.
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
    view: View,
    /// digits typed so far
    code: String,
    busy: bool,
    outcome: Option<Outcome>,
    /// connected, but the user asked to pair again
    repairing: bool,
    hover: Option<Hit>,
    pressed: Press<Hit>,
    frame: u32,
    scale: f32,
    /// the static picture, and whether it needs drawing again
    still: Option<Cache>,
    back: Option<Cache>,
    dirty: bool,
}

impl Ui {
    fn pairing(&self) -> bool {
        matches!(self.view.tray, Tray::Pair { .. }) || self.repairing
    }

    fn connected(&self) -> bool {
        matches!(self.view.tray, Tray::Connected { .. })
    }

    fn ready(&self) -> bool {
        self.code.len() == 6 && !self.busy
    }
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

fn with_ui<T>(f: impl FnOnce(&mut Ui) -> T) -> Option<T> {
    UI.with(|ui| ui.borrow_mut().as_mut().map(f))
}

// MARK: - geometry

/// The handle's box in the footer row.
fn handle_box(ui: &Ui) -> Option<(f32, f32, f32, f32)> {
    (ui.pairing() || matches!(ui.view.tray, Tray::Stopped)).then_some((22.0, FOOT + 2.0, 132.0, 36.0))
}

fn hit_test(ui: &Ui, x: f32, y: f32) -> Option<Hit> {
    let inside = |(bx, by, bw, bh): (f32, f32, f32, f32)| x >= bx && x <= bx + bw && y >= by && y <= by + bh;
    if let Some(b) = handle_box(ui) {
        if inside(b) {
            return Some(Hit::Go);
        }
    }
    if ui.repairing && inside((172.0, FOOT + 8.0, 40.0, 24.0)) {
        return Some(Hit::Cancel);
    }
    if ui.connected() && !ui.repairing && inside((22.0, FOOT + 8.0, 64.0, 24.0)) {
        return Some(Hit::Relink);
    }
    if inside((W - 22.0 - 70.0, FOOT + 8.0, 70.0, 24.0)) {
        return Some(Hit::Quit);
    }
    None
}

/// The monitor arm's elbow: clamped at `base`, reaching `mount`, elbow up (the Mac's sum).
fn elbow(base: (f32, f32), mount: (f32, f32)) -> (f32, f32) {
    let (l1, l2) = (135.0f32, 125.0f32);
    let (dx, dy) = (mount.0 - base.0, mount.1 - base.1);
    let len = (dx * dx + dy * dy).sqrt();
    let d = len.clamp((l1 - l2).abs() + 1.0, l1 + l2 - 1.0);
    let a = (l1 * l1 - l2 * l2 + d * d) / (2.0 * d);
    let h = (l1 * l1 - a * a).max(0.0).sqrt();
    let (ux, uy) = (dx / len, dy / len);
    let (px, py) = (base.0 + a * ux, base.1 + a * uy);
    let (e1, e2) = ((px - h * uy, py + h * ux), (px + h * uy, py - h * ux));
    if e1.1 < e2.1 { e1 } else { e2 }
}

// MARK: - drawing

/// Text with the engraved look: a light edge one pixel below.
fn engraved(c: &Canvas, s: &str, font: &Font, color: u32, x: f32, y: f32, w: f32, h: f32, align: Align) {
    c.text(s, font, rgba(0xFFEBCD, 0.35), x, y + 1.0, w, h, align);
    c.text(s, font, rgb(color), x, y, w, h, align);
}

/// A soft contact shadow, as the Mac draws one with a blurred ellipse.
fn contact(c: &Canvas, x: f32, y: f32, w: f32, h: f32, opacity: f32) {
    c.soft(x - 8.0, y - 5.0, w + 16.0, h + 10.0, rgba(0x8A6640, opacity));
}

fn draw_scene(c: &Canvas, ui: &Ui) {
    let live = ui.connected();
    let stopped = matches!(ui.view.tray, Tray::Stopped);
    let unpaired = matches!(ui.view.tray, Tray::Pair { fresh: true, .. });
    // the wall, its window light, the skirting, the desk
    c.fill(&Path::poly(&[(300.0, 0.0), (420.0, 0.0), (360.0, 262.0), (250.0, 262.0)]), rgba(0xF5EEE3, 0.7));
    c.fill(&Path::poly(&[(0.0, 252.0), (460.0, 252.0), (460.0, 262.0), (0.0, 262.0)]), rgb(0xE3D8C8));
    c.fill_linear(&Path::poly(&[(34.0, 262.0), (426.0, 262.0), (460.0, 372.0), (0.0, 372.0)]), (0.0, 262.0), (0.0, 372.0), rgb(0xD9BC95), rgb(0xCDAA7E));
    c.stroke(&Path::new().move_to(34.0, 262.0).line_to(426.0, 262.0), rgb(0xE9D3B2), 2.0);
    let grain = Path::new().move_to(60.0, 290.0).quad_to((200.0, 284.0), 400.0, 292.0).move_to(20.0, 330.0).quad_to((180.0, 322.0), 440.0, 334.0);
    c.stroke(&grain, rgba(0xC49C6E, 0.45), 1.2);

    // the plant
    c.within(412.0, 0.0, 1.0, |c| {
        contact(c, -26.0, 295.0, 52.0, 10.0, 0.3);
        let pot = Path::new().move_to(-18.0, 262.0).line_to(18.0, 262.0).line_to(14.0, 298.0).quad_to((0.0, 302.0), -14.0, 298.0).close();
        c.fill_linear(&pot, (-18.0, 0.0), (18.0, 0.0), rgb(0xD98463), rgb(0xB9654A));
        c.fill(&Path::round_rect(-20.0, 258.0, 40.0, 8.0, 3.0), rgb(0xE39572));
        for (p, color) in [
            ([-4.0, 236.0, -26.0, 226.0, -30.0, 206.0, -12.0, 210.0, -2.0, 228.0], 0x6FAE7E),
            ([4.0, 230.0, 22.0, 218.0, 30.0, 196.0, 34.0, 218.0, 14.0, 236.0], 0x5C9C6C),
            ([-2.0, 228.0, 6.0, 206.0, 2.0, 184.0, 14.0, 204.0, 10.0, 232.0], 0x7DBE8A),
        ] {
            let leaf = Path::new().move_to(0.0, 258.0).curve_to((p[0], p[1]), (p[2], p[3]), p[4], p[5]).curve_to((p[6], p[7]), (p[8], p[9]), 0.0, 258.0).close();
            c.fill(&leaf, rgb(color));
        }
    });

    // the monitor arm, clamped to the desk's back edge
    let base = (330.0, 248.0);
    let mount = (146.0 + 84.0, 94.0 + 52.0);
    let bend = elbow(base, mount);
    contact(c, base.0 - 16.0, base.1 + 18.0, 32.0, 6.0, 0.3);
    c.fill_linear(&Path::round_rect(base.0 - 11.0, base.1 + 6.0, 22.0, 16.0, 3.0), (0.0, base.1 + 6.0), (0.0, base.1 + 22.0), rgb(0xD2D6DC), rgb(0xAEB4BC));
    c.fill(&Path::round_rect(base.0 - 4.0, base.1 - 2.0, 8.0, 12.0, 2.0), rgb(0xC7CCD2));
    let arm = Path::new().move_to(base.0, base.1).line_to(bend.0, bend.1).line_to(mount.0, mount.1);
    c.stroke(&arm, rgb(0xAEB4BC), 9.0);
    c.stroke(&arm, rgb(0xE2E5E9), 6.0);
    for (x, y) in [base, bend] {
        let joint = Path::ellipse(x - 5.5, y - 5.5, 11.0, 11.0);
        c.fill(&joint, rgb(0xD2D6DC));
        c.stroke(&joint, rgb(0xAEB4BC), 1.3);
    }

    // this computer's screen
    c.within(146.0, 94.0, 1.0, |c| {
        c.soft(-6.0, 6.0, 180.0, 116.0, rgba(0x6B5236, 0.3));
        c.fill_linear(&Path::round_rect(0.0, 0.0, 168.0, 104.0, 10.0), (0.0, 0.0), (0.0, 104.0), rgb(0xEEF0F3), rgb(0xD2D6DC));
        c.fill(&Path::round_rect(5.0, 5.0, 158.0, 94.0, 6.0), rgb(0x1F242B));
        let (top, bottom) = if live { (0x8FE0D3, 0x2F8F84) } else if stopped { (0xC9BFB4, 0x8E8276) } else { (0xF5D79A, 0xC9923A) };
        c.fill_linear(&Path::round_rect(9.0, 9.0, 150.0, 86.0, 3.0), (0.0, 9.0), (0.0, 95.0), rgb(top), rgb(bottom));
        let hills = Path::new().move_to(9.0, 72.0).quad_to((50.0, 58.0), 90.0, 70.0).quad_to((130.0, 82.0), 159.0, 64.0).line_to(159.0, 95.0).line_to(9.0, 95.0).close();
        c.fill(&hills, rgba(0xFFFFFF, 0.16));
        c.fill(&Path::poly(&[(9.0, 87.0), (159.0, 87.0), (159.0, 95.0), (9.0, 95.0)]), rgba(0x000000, 0.22));
        c.fill(&Path::round_rect(78.0, 89.0, 12.0, 4.0, 1.0), rgba(0xFFFFFF, 0.7));
        c.text("这台电脑", &ui.fonts.screen, rgba(0xFFFFFF, 0.95), 9.0, 30.0, 150.0, 30.0, Align::Center);
    });

    // the Mac
    let fade = if unpaired { 0.3 } else { 1.0 };
    contact(c, 140.0, 353.0, 180.0, 14.0, 0.35);
    c.fill_linear(&Path::round_rect(166.0, 234.0, 128.0, 96.0, 9.0), (0.0, 234.0), (0.0, 330.0), rgba(0xEEF0F3, fade), rgba(0xD2D6DC, fade));
    c.fill(&Path::round_rect(170.0, 238.0, 120.0, 88.0, 6.0), rgba(0x1F242B, fade));
    c.fill_linear(&Path::round_rect(174.0, 243.0, 112.0, 79.0, 3.0), (174.0, 243.0), (286.0, 322.0), rgba(0xF2C9A0, fade), rgba(0xC98D7A, fade));
    c.fill(&Path::poly(&[(174.0, 243.0), (286.0, 243.0), (286.0, 248.0), (174.0, 248.0)]), rgba(0xFFFFFF, 0.35 * fade));
    c.text(if unpaired { "?" } else { "Mac" }, &ui.fonts.mac, rgba(0xFFFFFF, 0.85 * fade), 174.0, 272.0, 112.0, 24.0, Align::Center);
    let (top, bot, tl, tr, bl, br) = (330.0f32, 354.0f32, 166.0f32, 294.0f32, 152.0f32, 308.0f32);
    let edge = |y: f32| {
        let t = (y - top) / (bot - top);
        (tl + (bl - tl) * t, tr + (br - tr) * t)
    };
    c.fill_linear(&Path::poly(&[(tl, top), (tr, top), (br, bot), (bl, bot)]), (0.0, top), (0.0, bot), rgba(0xE2E5E9, fade), rgba(0xC7CCD2, fade));
    c.fill(&Path::poly(&[(bl, bot), (br, bot), (br - 2.5, bot + 4.5), (bl + 2.5, bot + 4.5)]), rgba(0xAEB4BC, fade));
    let m = 8.5;
    let ((l0, r0), (l1, r1)) = (edge(331.4), edge(343.6));
    c.fill(&Path::poly(&[(l0 + m - 1.0, 331.4), (r0 - m + 1.0, 331.4), (r1 - m + 1.0, 343.6), (l1 + m - 1.0, 343.6)]), rgba(0xB3B9C1, fade));
    let rows: [(f32, f32, &[f32]); 4] = [
        (332.1, 2.2, &[1.0; 13]),
        (334.9, 2.4, &[1.4, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.4]),
        (337.9, 2.5, &[1.8, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.8]),
        (341.0, 2.5, &[1.2, 1.0, 1.2, 6.2, 1.2, 1.0, 1.2]),
    ];
    for (y, h, units) in rows {
        let ((la, ra), (lb, rb)) = (edge(y), edge(y + h));
        let (l, r) = ((la + lb) / 2.0 + m, (ra + rb) / 2.0 - m);
        let gap = 0.8;
        let unit = (r - l - gap * (units.len() - 1) as f32) / units.iter().sum::<f32>();
        let mut x = l;
        for u in units {
            let w = unit * u;
            c.fill(&Path::round_rect(x, y, w, h, 0.6), rgba(0x2E333A, fade));
            x += w + gap;
        }
    }
    c.fill(&Path::poly(&[(210.0, 345.2), (250.0, 345.2), (252.0, 352.4), (208.0, 352.4)]), rgba(0xD5D9DE, fade));

    // the mug (its steam moves, so it is drawn each frame)
    contact(c, 75.0, 351.0, 44.0, 10.0, 0.3);
    c.fill(&Path::new().move_to(80.0, 324.0).line_to(114.0, 324.0).line_to(111.0, 354.0).quad_to((97.0, 358.0), 83.0, 354.0).close(), rgb(CLAY));
    c.fill(&Path::ellipse(80.0, 320.0, 34.0, 8.0), rgb(0x7A5236));
    c.stroke(&Path::new().move_to(113.0, 330.0).curve_to((124.0, 330.0), (124.0, 346.0), 111.0, 346.0), rgb(CLAY), 4.0);
}

fn draw_still(c: &Canvas, ui: &Ui) {
    c.fill(&Path::poly(&[(0.0, 0.0), (W, 0.0), (W, H), (0.0, H)]), rgb(WALL));
    c.within(0.0, SCENE_TOP - 38.0 * SCENE_SCALE, SCENE_SCALE, |c| draw_scene(c, ui));

    // the sign on the wall
    let dot = match ui.view.light {
        Light::Green => TEAL,
        Light::Orange => AMBER,
        Light::Red => RED,
    };
    c.fill(&Path::ellipse(18.0, 19.0, 18.0, 18.0), rgba(dot, 0.2));
    c.fill(&Path::ellipse(22.0, 23.0, 10.0, 10.0), rgb(dot));
    c.text(&ui.view.headline, &ui.fonts.head, rgb(INK), 44.0, 14.0, W - 60.0, 28.0, Align::Left);
    c.paragraph(&ui.view.detail, &ui.fonts.body, rgb(INK_SOFT), 44.0, 42.0, 250.0, 36.0);
    // the developer bridge is never open unnoticed (RULES.md X5)
    if let Some(line) = &ui.view.bridge {
        c.text(line, &ui.fonts.link, rgb(TEAL), 44.0, 76.0, 300.0, 18.0, Align::Left);
    }

    // the drawer and its tray
    c.fill(&Path::poly(&[(0.0, DRAWER), (W, DRAWER), (W, H), (0.0, H)]), rgb(WOOD));
    c.fill(&Path::poly(&[(0.0, DRAWER), (W, DRAWER), (W, DRAWER + 10.0), (0.0, DRAWER + 10.0)]), rgb(WOOD_BAND));
    let (tx, ty, tw, th) = TRAY;
    let tray = Path::round_rect(tx, ty, tw, th, 18.0);
    c.stroke(&Path::new().move_to(tx + 14.0, ty + th + 0.6).line_to(tx + tw - 14.0, ty + th + 0.6), rgba(0xFFEBCD, 0.3), 1.0);
    c.fill(&tray, rgb(TRAY_FILL));
    c.clipped(&tray, |c| {
        c.fill_linear(&Path::poly(&[(tx, ty), (tx + tw, ty), (tx + tw, ty + 14.0), (tx, ty + 14.0)]), (0.0, ty), (0.0, ty + 14.0), rgba(0x3C230A, 0.42), rgba(0x3C230A, 0.0));
        c.stroke(&Path::new().move_to(tx, ty + th - 1.0).line_to(tx + tw, ty + th - 1.0), rgba(0xFFEBCD, 0.18), 2.0);
    });
    let label = |c: &Canvas, s: &str, x: f32, y: f32, w: f32, color: u32| c.text(s, &ui.fonts.label, color, x, y, w, 16.0, Align::Center);
    let soft_cream = rgba(0xE9D6BA, 0.8);
    match &ui.view.tray {
        Tray::Connected { since, host } if !ui.repairing => {
            let half = tw / 2.0;
            label(c, "已连接", tx, ty + 34.0, half, soft_cream);
            c.text(since, &ui.fonts.value, rgb(CREAM), tx, ty + 54.0, half, 32.0, Align::Center);
            label(c, "Mac 地址", tx + half, ty + 34.0, half, soft_cream);
            c.text(host, &ui.fonts.number, rgb(CREAM), tx + half, ty + 54.0, half, 32.0, Align::Center);
            c.stroke(&Path::new().move_to(tx + half, ty + 30.0).line_to(tx + half, ty + 94.0), rgba(0x46290F, 0.35), 2.0);
            c.stroke(&Path::new().move_to(tx + half + 1.5, ty + 30.0).line_to(tx + half + 1.5, ty + 94.0), rgba(0xFFEBCD, 0.18), 1.0);
        }
        Tray::Stopped => {
            label(c, "状态", tx, ty + 36.0, tw, soft_cream);
            c.text("Wisp 没在运行，键鼠不会过来", &ui.fonts.notice, rgb(CREAM), tx, ty + 56.0, tw, 30.0, Align::Center);
        }
        _ => draw_slots(c, ui),
    }

    // the footer: the handle on the left, words on the right
    if let Some((hx, hy, hw, hh)) = handle_box(ui) {
        let down = ui.pressed.held() == Some(Hit::Go) && ui.hover == Some(Hit::Go);
        let enabled = !ui.pairing() || ui.ready();
        let a = if enabled { 1.0 } else { 0.55 };
        let y = if down { hy + 3.0 } else { hy };
        c.soft(hx - 6.0, hy + 2.0, hw + 12.0, hh + 12.0, rgba(0x321C08, 0.25 * a));
        c.fill(&Path::round_rect(hx, hy + if down { 4.0 } else { 4.0 }, hw, hh, hh / 2.0), rgba(CLAY_DROP, a));
        let cap = Path::round_rect(hx, y, hw, hh, hh / 2.0);
        c.fill(&cap, rgba(if ui.hover == Some(Hit::Go) && enabled { 0xF8F0E4 } else { CLAY }, a));
        c.clipped(&cap, |c| c.fill(&Path::poly(&[(hx, y + hh - 4.0), (hx + hw, y + hh - 4.0), (hx + hw, y + hh), (hx, y + hh)]), rgba(CLAY_SHADE, a)));
        let words = match (&ui.view.tray, ui.busy) {
            (Tray::Stopped, _) => "启动 Wisp",
            (_, true) => "配对中…",
            _ => "配  对",
        };
        c.text(words, &ui.fonts.handle, rgba(INK, a), hx, y, hw, hh - 3.0, Align::Center);
    }
    let link = |c: &Canvas, s: &str, hit: Hit, x: f32, w: f32, align: Align| {
        let color = if ui.hover == Some(hit) { 0x3E2A14 } else { ENGRAVE };
        engraved(c, s, &ui.fonts.link, color, x, FOOT + 8.0, w, 24.0, align);
    };
    if ui.repairing {
        link(c, "取消", Hit::Cancel, 172.0, 40.0, Align::Left);
    } else if ui.connected() {
        link(c, "重新配对", Hit::Relink, 22.0, 80.0, Align::Left);
    }
    link(c, "退出 Wisp", Hit::Quit, W - 22.0 - 120.0, 120.0, Align::Right);
}

/// The pairing tray: a line of words over six carved slots, digits as clay keycaps.
fn draw_slots(c: &Canvas, ui: &Ui) {
    let (tx, ty, tw, _) = TRAY;
    let prompt = match &ui.view.tray {
        Tray::Pair { prompt, .. } => *prompt,
        _ => "输入 Mac 上显示的新配对码",
    };
    let (words, color) = match (&ui.outcome, ui.busy) {
        (_, true) => ("正在和 Mac 核对配对码", rgba(0xE9D6BA, 0.8)),
        (Some((said, ok)), false) => (said.as_str(), rgb(if *ok { 0xD6F3EE } else { 0xFFD2C2 })),
        (None, false) => (prompt, rgba(0xE9D6BA, 0.8)),
    };
    c.text(words, &ui.fonts.label, color, tx, ty + 22.0, tw, 16.0, Align::Center);
    let digits: Vec<char> = ui.code.chars().collect();
    for i in 0..6 {
        let (x, y) = slot_origin(i);
        let slot = Path::round_rect(x, y, 38.0, 44.0, 10.0);
        c.fill(&slot, rgb(0x94704A));
        c.clipped(&slot, |c| {
            c.fill_linear(&Path::poly(&[(x, y), (x + 38.0, y), (x + 38.0, y + 9.0), (x, y + 9.0)]), (0.0, y), (0.0, y + 9.0), rgba(0x3C230A, 0.45), rgba(0x3C230A, 0.0));
        });
        if let Some(ch) = digits.get(i) {
            c.fill(&Path::round_rect(x + 1.0, y + 4.0, 36.0, 40.0, 9.0), rgb(CLAY_DROP));
            let cap = Path::round_rect(x + 1.0, y, 36.0, 40.0, 9.0);
            c.fill(&cap, rgb(CLAY));
            c.clipped(&cap, |c| c.fill(&Path::poly(&[(x, y + 35.0), (x + 38.0, y + 35.0), (x + 38.0, y + 40.0), (x, y + 40.0)]), rgb(CLAY_SHADE)));
            c.text(&ch.to_string(), &ui.fonts.digit, rgb(INK), x, y, 38.0, 36.0, Align::Center);
        }
    }
}

fn slot_origin(i: usize) -> (f32, f32) {
    let (tx, ty, tw, _) = TRAY;
    let width = 6.0 * 38.0 + 5.0 * 8.0 + 10.0;
    let x = tx + (tw - width) / 2.0 + i as f32 * 46.0 + if i >= 3 { 10.0 } else { 0.0 };
    (x, ty + 50.0)
}

/// The moving parts, drawn over the still picture every frame.
fn draw_moving(c: &Canvas, ui: &Ui) {
    let t = ui.frame as f32 * 0.05;
    c.within(0.0, SCENE_TOP - 38.0 * SCENE_SCALE, SCENE_SCALE, |c| {
        // steam over the mug
        let rise = ((t / 1.5 * std::f32::consts::PI).sin() + 1.0) / 2.0;
        c.within(0.0, -5.0 * rise, 1.0, |c| {
            let steam = Path::new().move_to(92.0, 318.0).curve_to((86.0, 308.0), (98.0, 302.0), 92.0, 292.0).move_to(102.0, 316.0).curve_to((96.0, 306.0), (108.0, 300.0), 102.0, 290.0);
            c.stroke(&steam, rgba(0xFFFFFF, 0.9 - 0.5 * rise), 2.5);
        });
        // the wisp between the Mac and this screen
        if ui.connected() {
            let wisp = Path::new().move_to(230.0, 244.0).curve_to((206.0, 232.0), (254.0, 214.0), 230.0, 200.0);
            for (w, a) in [(12.0, 0.12), (8.0, 0.18)] {
                c.stroke(&wisp, rgba(TEAL_GLOW, a), w);
            }
            let phase = (t / 0.6).fract() * (9.0 / 3.5);
            c.stroke_dashed(&wisp, rgb(TEAL), 3.5, &[0.01, 9.0 / 3.5], -phase);
        }
    });
    if ui.pairing() && !ui.busy && ui.code.len() < 6 && (ui.frame / 10) % 2 == 0 {
        let (x, y) = slot_origin(ui.code.len());
        c.fill(&Path::round_rect(x + 18.0, y + 11.0, 2.0, 22.0, 1.0), rgba(CREAM, 0.85));
    }
}

/// The parts of the window (in device pixels) that move between frames.
fn moving_rects(scale: f32) -> [Rect; 3] {
    let r = |x: f32, y: f32, w: f32, h: f32| Rect {
        left: (x * scale) as i32 - 2,
        top: (y * scale) as i32 - 2,
        right: ((x + w) * scale) as i32 + 2,
        bottom: ((y + h) * scale) as i32 + 2,
    };
    let s = |v: f32, top: bool| if top { SCENE_TOP + (v - 38.0) * SCENE_SCALE } else { v * SCENE_SCALE };
    [
        r(s(80.0, false), s(280.0, true), s(36.0, false), s(46.0, false) + 6.0),
        r(s(196.0, false), s(190.0, true), s(68.0, false), s(66.0, false)),
        r(TRAY.0, TRAY.1 + 50.0, TRAY.2, 44.0),
    ]
}

unsafe fn paint(hwnd: Hwnd) {
    let mut ps = std::mem::zeroed::<PaintStruct>();
    let hdc = BeginPaint(hwnd, &mut ps);
    with_ui(|ui| {
        let (w, h) = ((W * ui.scale).ceil() as i32, (H * ui.scale).ceil() as i32);
        if ui.still.as_ref().is_none_or(|c| c.w != w || c.h != h) {
            ui.still = Some(Cache::new(w, h));
            ui.back = Some(Cache::new(w, h));
            ui.dirty = true;
        }
        let (still, back) = (ui.still.as_ref().unwrap().dc, ui.back.as_ref().unwrap().dc);
        if ui.dirty {
            ui.dirty = false;
            let c = Canvas::on(still, ui.scale);
            draw_still(&c, ui);
        }
        let p = ps.paint;
        BitBlt(back, p.left, p.top, p.right - p.left, p.bottom - p.top, still, p.left, p.top, SRCCOPY);
        {
            let c = Canvas::on(back, ui.scale);
            draw_moving(&c, ui);
        }
        BitBlt(hdc, p.left, p.top, p.right - p.left, p.bottom - p.top, back, p.left, p.top, SRCCOPY);
    });
    EndPaint(hwnd, &ps);
}

/// Something on the still picture changed: draw it again on the next paint.
unsafe fn refresh(hwnd: Hwnd) {
    with_ui(|ui| ui.dirty = true);
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
            Ok(name) => (format!("已与 {name} 配对，正在连接…"), true),
            Err(e) => (e.to_string(), false),
        };
        *PAIRED.lock().unwrap() = Some(outcome);
        unsafe { PostMessageW(window as Hwnd, WM_PAIRED, 0, 0) };
    });
}

/// Quitting from here does what the tray's "退出 Wisp" does: the client exits and closes this window.
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
            if with_ui(|ui| matches!(ui.view.tray, Tray::Stopped)) == Some(true) {
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
        Hit::Quit => quit(hwnd),
    }
    refresh(hwnd);
}

fn logical(lparam: isize, scale: f32) -> (f32, f32) {
    let (x, y) = ((lparam & 0xFFFF) as i16 as f32, ((lparam >> 16) & 0xFFFF) as i16 as f32);
    (x / scale, y / scale)
}

unsafe extern "system" fn wnd_proc(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_TIMER if wparam == STATUS_TIMER => {
            let view = current_view();
            let changed = with_ui(|ui| {
                if ui.view == view {
                    return false;
                }
                if matches!(view.tray, Tray::Connected { .. }) && !matches!(ui.view.tray, Tray::Connected { .. }) {
                    // just paired: back to the connected tray
                    ui.repairing = false;
                    ui.code.clear();
                    ui.outcome = None;
                }
                ui.view = view;
                true
            });
            if changed == Some(true) {
                refresh(hwnd);
            }
            0
        }
        WM_TIMER if wparam == FRAME_TIMER => {
            if IsIconic(hwnd) != 0 {
                return 0;
            }
            if let Some((scale, wisp, caret)) = with_ui(|ui| {
                ui.frame = ui.frame.wrapping_add(1);
                // the caret only changes when it blinks
                (ui.scale, ui.connected(), ui.pairing() && ui.frame % 10 == 0)
            }) {
                let [steam, wisp_rect, caret_rect] = moving_rects(scale);
                InvalidateRect(hwnd, &steam, 0);
                if wisp {
                    InvalidateRect(hwnd, &wisp_rect, 0);
                }
                if caret {
                    InvalidateRect(hwnd, &caret_rect, 0);
                }
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
                    }
                    '\u{8}' => {
                        ui.code.pop();
                    }
                    '\r' if ui.code.len() == 6 => go = true,
                    '\r' => ui.outcome = Some(("配对码是 6 位数字".into(), false)),
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
                    // still connected (or back already): the connected tray, not a stuck pairing one
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
        // the title bar in the wall's colour (Windows 11; older versions ignore it)
        tint_title_bar(hwnd);
        UI.with(|ui| {
            *ui.borrow_mut() = Some(Ui {
                fonts: Fonts::new(),
                view: current_view(),
                code: String::new(),
                busy: false,
                outcome: None,
                repairing: false,
                hover: None,
                pressed: Press::default(),
                frame: 0,
                scale,
                still: None,
                back: None,
                dirty: true,
            })
        });
        ShowWindow(hwnd, SW_SHOW);
        SetTimer(hwnd, STATUS_TIMER, 1000, ptr::null_mut());
        SetTimer(hwnd, FRAME_TIMER, 50, ptr::null_mut());
        let mut msg = std::mem::zeroed::<Msg>();
        while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
