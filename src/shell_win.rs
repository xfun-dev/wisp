//! Wisp's presence on the Windows desktop, all on one UI thread:
//! - the cloak (RULES.md H3): while the Mac has control, a tiny, nearly transparent
//!   topmost window sits under the pointer and asks for no cursor. It belongs to this
//!   process, so if wisp exits or crashes the pointer comes back, and a local mouse
//!   moving off it shows the pointer again; no system-wide cursor setting is touched.
//! - the tray icon (RULES.md U10, B1): the 一缕 glyph coloured by the connection state, the status
//!   as its tooltip, a click for the status window, a right-click menu to quit.

use crate::client_win::read_status;
use crate::logic::icon::{mask, tinted};
use crate::logic::status::{describe, Light};
use crate::win32::{instance, register, wide, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, Hwnd, Msg, Point, ShowWindow};
use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr;
use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

#[repr(C)]
struct NotifyIconDataW {
    size: u32,
    hwnd: Hwnd,
    id: u32,
    flags: u32,
    callback: u32,
    icon: *mut c_void,
    tip: [u16; 128],
    state: u32,
    state_mask: u32,
    info: [u16; 256],
    version: u32,
    info_title: [u16; 64],
    info_flags: u32,
    guid: [u8; 16],
    balloon_icon: *mut c_void,
}

#[repr(C)]
struct BitmapInfo {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    x_ppm: i32,
    y_ppm: i32,
    colors_used: u32,
    colors_important: u32,
    colors: [u32; 1],
}

#[repr(C)]
struct IconInfo {
    is_icon: i32,
    x_hotspot: u32,
    y_hotspot: u32,
    mask: *mut c_void,
    color: *mut c_void,
}

#[link(name = "user32")]
extern "system" {
    fn PostThreadMessageW(thread: u32, msg: u32, wparam: usize, lparam: isize) -> i32;
    fn PostMessageW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> i32;
    fn SetWindowPos(hwnd: Hwnd, after: Hwnd, x: i32, y: i32, w: i32, h: i32, flags: u32) -> i32;
    fn SetLayeredWindowAttributes(hwnd: Hwnd, key: u32, alpha: u8, flags: u32) -> i32;
    fn SetCursor(cursor: *mut c_void) -> *mut c_void;
    fn GetCursorPos(p: *mut Point) -> i32;
    fn SetCursorPos(x: i32, y: i32) -> i32;
    fn GetSystemMetrics(index: i32) -> i32;
    fn CreateIconIndirect(info: *const IconInfo) -> *mut c_void;
    fn RegisterWindowMessageW(name: *const u16) -> u32;
    fn CreatePopupMenu() -> *mut c_void;
    fn AppendMenuW(menu: *mut c_void, flags: u32, id: usize, text: *const u16) -> i32;
    fn TrackPopupMenu(menu: *mut c_void, flags: u32, x: i32, y: i32, reserved: i32, hwnd: Hwnd, rect: *const c_void) -> i32;
    fn DestroyMenu(menu: *mut c_void) -> i32;
    fn SetForegroundWindow(hwnd: Hwnd) -> i32;
    fn FindWindowW(class: *const u16, title: *const u16) -> Hwnd;
    fn ChangeWindowMessageFilterEx(hwnd: Hwnd, msg: u32, action: u32, info: *mut c_void) -> i32;
}

#[link(name = "gdi32")]
extern "system" {
    fn CreateDIBSection(hdc: *mut c_void, info: *const BitmapInfo, usage: u32, bits: *mut *mut c_void, section: *mut c_void, offset: u32) -> *mut c_void;
    fn CreateBitmap(w: i32, h: i32, planes: u32, bpp: u32, bits: *const c_void) -> *mut c_void;
    fn DeleteObject(obj: *mut c_void) -> i32;
}

#[link(name = "shell32")]
extern "system" {
    fn Shell_NotifyIconW(msg: u32, data: *mut NotifyIconDataW) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentThreadId() -> u32;
}

const WM_NULL: u32 = 0x0000;
const WM_CLOSE: u32 = 0x0010;
const WM_SETCURSOR: u32 = 0x0020;
const WM_MOUSEACTIVATE: u32 = 0x0021;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_RBUTTONUP: u32 = 0x0205;
const MA_NOACTIVATE: isize = 3;
const WM_COVER: u32 = 0x8000 + 1;
const WM_UNCOVER: u32 = 0x8000 + 2;
const WM_REFRESH: u32 = 0x8000 + 3;
const WM_TRAY: u32 = 0x8000 + 4;
/// posted by the status window's "退出 Wisp": quit as the tray menu's item does
pub const WM_QUIT_WISP: u32 = 0x8000 + 5;
const WS_POPUP: u32 = 0x8000_0000;
const WS_EX_TOPMOST: u32 = 0x0000_0008;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const WS_EX_LAYERED: u32 = 0x0008_0000;
const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
const LWA_ALPHA: u32 = 0x2;
const SWP_NOACTIVATE: u32 = 0x0010;
const SWP_SHOWWINDOW: u32 = 0x0040;
const SW_HIDE: i32 = 0;
const SW_RESTORE: i32 = 9;
const HWND_TOPMOST: Hwnd = -1isize as Hwnd;
const SM_CXSMICON: i32 = 49;
const NIM_ADD: u32 = 0;
const NIM_MODIFY: u32 = 1;
const NIM_DELETE: u32 = 2;
const NIF_MESSAGE: u32 = 0x1;
const NIF_ICON: u32 = 0x2;
const NIF_TIP: u32 = 0x4;
const MF_STRING: u32 = 0x0;
const MF_SEPARATOR: u32 = 0x800;
const TPM_RIGHTBUTTON: u32 = 0x2;
const TPM_RETURNCMD: u32 = 0x100;
const MENU_OPEN: usize = 1;
const MENU_QUIT: usize = 2;
const MENU_SERVICE: usize = 3;
const MF_CHECKED: u32 = 0x8;
const MSGFLT_ALLOW: u32 = 1;
/// side of the square kept under the pointer, in pixels
const CLOAK_SIZE: i32 = 64;

/// Builds a tray icon of about `size` pixels from the glyph mask, tinted `rgb`.
unsafe fn make_icon(size: i32, rgb: (u8, u8, u8)) -> *mut c_void {
    let (side, coverage) = mask(size as u32);
    let size = side as i32;
    let px = tinted(coverage, rgb);
    let info = BitmapInfo {
        size: 40,
        width: size,
        height: -size, // top-down rows
        planes: 1,
        bit_count: 32,
        compression: 0,
        size_image: 0,
        x_ppm: 0,
        y_ppm: 0,
        colors_used: 0,
        colors_important: 0,
        colors: [0],
    };
    let mut bits = ptr::null_mut();
    let color = CreateDIBSection(ptr::null_mut(), &info, 0, &mut bits, ptr::null_mut(), 0);
    if color.is_null() {
        return ptr::null_mut();
    }
    ptr::copy_nonoverlapping(px.as_ptr(), bits as *mut u32, px.len());
    // an all-zero mask: the colour bitmap's alpha decides what shows
    let mask_bits = vec![0u8; ((size as usize + 15) / 16 * 2) * size as usize];
    let mask = CreateBitmap(size, size, 1, 1, mask_bits.as_ptr() as *const c_void);
    let icon = CreateIconIndirect(&IconInfo { is_icon: 1, x_hotspot: 0, y_hotspot: 0, mask, color });
    DeleteObject(color);
    DeleteObject(mask);
    icon
}

struct Tray {
    hwnd: Hwnd,
    connected: *mut c_void,
    searching: *mut c_void,
    /// broadcast when Explorer restarts; the icon has to be added again
    taskbar_created: u32,
}

thread_local! {
    static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
}

fn tray_data(tray: &Tray) -> NotifyIconDataW {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let view = describe(read_status().as_deref(), true, now);
    let mut data: NotifyIconDataW = unsafe { std::mem::zeroed() };
    data.size = std::mem::size_of::<NotifyIconDataW>() as u32;
    data.hwnd = tray.hwnd;
    data.id = 1;
    data.flags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.callback = WM_TRAY;
    data.icon = if view.light == Light::Green { tray.connected } else { tray.searching };
    for (slot, unit) in data.tip.iter_mut().take(127).zip(format!("Wisp：{}", view.headline).encode_utf16()) {
        *slot = unit;
    }
    data
}

fn update_tray(op: u32) {
    TRAY.with(|t| {
        if let Some(tray) = &*t.borrow() {
            let mut data = tray_data(tray);
            unsafe { Shell_NotifyIconW(op, &mut data) };
        }
    });
}

/// The open status window, if any.
fn status_window() -> Hwnd {
    let class = wide("WispStatus");
    unsafe { FindWindowW(class.as_ptr(), ptr::null()) }
}

/// Brings the status window forward, or opens one.
fn open_status() {
    let existing = status_window();
    if !existing.is_null() {
        unsafe {
            ShowWindow(existing, SW_RESTORE);
            SetForegroundWindow(existing);
        }
    } else if let Ok(exe) = std::env::current_exe() {
        // from WispService's SYSTEM client, the window still opens as the user (W4)
        let _ = crate::service_win::run_as_user(&exe, "status");
    }
}

/// Switches WispService on or off: a separate `service-switch` run as the user asks
/// for the administrator (UAC) and does it (RULES.md W1).
fn switch_service(on: bool) {
    if let Ok(exe) = std::env::current_exe() {
        let _ = crate::service_win::run_as_user(&exe, if on { "service-switch on" } else { "service-switch off" });
    }
}

unsafe fn tray_menu(hwnd: Hwnd) {
    let menu = CreatePopupMenu();
    AppendMenuW(menu, MF_STRING, MENU_OPEN, wide("打开状态窗口").as_ptr());
    let service = crate::service_win::installed();
    let check = if service { MF_CHECKED } else { 0 };
    AppendMenuW(menu, MF_STRING | check, MENU_SERVICE, wide("锁屏和权限弹窗也能控制").as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
    AppendMenuW(menu, MF_STRING, MENU_QUIT, wide("退出 Wisp").as_ptr());
    let mut p = Point::default();
    GetCursorPos(&mut p);
    // without this the menu would not close when clicking elsewhere
    SetForegroundWindow(hwnd);
    let chosen = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, p.x, p.y, 0, hwnd, ptr::null()) as usize;
    PostMessageW(hwnd, WM_NULL, 0, 0);
    DestroyMenu(menu);
    match chosen {
        MENU_OPEN => open_status(),
        MENU_SERVICE => switch_service(!service),
        MENU_QUIT => quit("托盘"),
        _ => {}
    }
}

/// Quits the client, closing the status window too; only a crash leaves it up, saying so.
unsafe fn quit(from: &str) -> ! {
    log!("wisp: 用户从{from}退出");

    let window = status_window();
    if !window.is_null() {
        PostMessageW(window, WM_CLOSE, 0, 0);
    }
    update_tray(NIM_DELETE);
    // as WispService's client, the exit code tells the service not to start it again (W6)
    std::process::exit(if crate::client_win::is_system() { crate::service_win::QUIT_EXIT as i32 } else { 0 });
}

unsafe extern "system" fn cloak_proc(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize {
    match msg {
        WM_SETCURSOR => {
            SetCursor(ptr::null_mut());
            1
        }
        WM_MOUSEACTIVATE => MA_NOACTIVATE,
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe extern "system" fn tray_proc(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize {
    if msg == WM_QUIT_WISP {
        quit("状态窗口");
    }
    if msg == WM_TRAY {
        match lparam as u32 & 0xFFFF {
            WM_LBUTTONUP => open_status(),
            WM_RBUTTONUP => tray_menu(hwnd),
            _ => {}
        }
        return 0;
    }
    let restarted = TRAY.with(|t| t.borrow().as_ref().is_some_and(|tray| msg == tray.taskbar_created));
    if restarted {
        update_tray(NIM_ADD);
        return 0;
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

unsafe fn window(class: &[u16], proc_: crate::win32::WndProc, ex_style: u32) -> Hwnd {
    register(class, proc_, ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
    CreateWindowExW(ex_style, class.as_ptr(), ptr::null(), WS_POPUP, 0, 0, 0, 0, ptr::null_mut(), ptr::null_mut(), instance(), ptr::null_mut())
}

/// Handle to the UI thread.
pub struct Shell {
    thread: u32,
}

impl Shell {
    /// Starts the UI thread. Either part may be missing if Windows refuses its window;
    /// the client works regardless.
    pub fn spawn() -> Shell {
        let (ready, started) = mpsc::channel();
        std::thread::spawn(move || unsafe {
            // no class cursor: the cloak exists to show none
            let cloak_class = wide("WispCloak");
            let cloak = window(&cloak_class, cloak_proc, WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE);
            // alpha 1 of 255: invisible to the eye, but still hit-tested, so it decides the cursor
            SetLayeredWindowAttributes(cloak, 0, 1, LWA_ALPHA);

            let tray_class = wide("WispTray");
            let hwnd = window(&tray_class, tray_proc, WS_EX_TOOLWINDOW);
            if !hwnd.is_null() {
                let size = GetSystemMetrics(SM_CXSMICON).max(16);
                TRAY.with(|t| {
                    *t.borrow_mut() = Some(Tray {
                        hwnd,
                        connected: make_icon(size, (0x2F, 0xD5, 0xBE)),
                        searching: make_icon(size, (0xF2, 0xA3, 0x3A)),
                        taskbar_created: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
                    })
                });
                if crate::client_win::is_system() {
                    // as SYSTEM, messages from Explorer and the user's status window
                    // are filtered out unless allowed (W3)
                    let taskbar = TRAY.with(|t| t.borrow().as_ref().map_or(0, |t| t.taskbar_created));
                    for m in [WM_TRAY, WM_QUIT_WISP, taskbar] {
                        ChangeWindowMessageFilterEx(hwnd, m, MSGFLT_ALLOW, ptr::null_mut());
                    }
                }
                update_tray(NIM_ADD);
            }
            let _ = ready.send(GetCurrentThreadId());

            let mut msg = std::mem::zeroed::<Msg>();
            while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
                match (msg.hwnd.is_null(), msg.message) {
                    (true, WM_COVER) if !cloak.is_null() => {
                        let mut p = Point::default();
                        GetCursorPos(&mut p);
                        let half = CLOAK_SIZE / 2;
                        let flags = SWP_NOACTIVATE | SWP_SHOWWINDOW;
                        SetWindowPos(cloak, HWND_TOPMOST, p.x - half, p.y - half, CLOAK_SIZE, CLOAK_SIZE, flags);
                        // re-place the pointer where it is, so Windows asks the cloak for a cursor now
                        SetCursorPos(p.x, p.y);
                    }
                    (true, WM_UNCOVER) if !cloak.is_null() => {
                        ShowWindow(cloak, SW_HIDE);
                    }
                    (true, WM_REFRESH) => update_tray(NIM_MODIFY),
                    _ => {
                        DispatchMessageW(&msg);
                    }
                }
            }
        });
        Shell { thread: started.recv().unwrap_or(0) }
    }

    fn post(&self, msg: u32) {
        unsafe { PostThreadMessageW(self.thread, msg, 0, 0) };
    }

    /// Hides the pointer where it is now.
    pub fn cover(&self) {
        self.post(WM_COVER);
    }

    /// Shows the pointer again.
    pub fn uncover(&self) {
        self.post(WM_UNCOVER);
    }

    /// Re-reads the status file into the tray icon's colour and tooltip.
    pub fn refresh(&self) {
        self.post(WM_REFRESH);
    }
}
