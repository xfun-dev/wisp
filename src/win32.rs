//! Win32 window plumbing shared by the pointer cloak and the status window, declared
//! once so both agree on every signature.

use std::ffi::c_void;
use std::ptr;

pub type Hwnd = *mut c_void;
pub type WndProc = unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize;

#[repr(C)]
pub struct WndClassExW {
    pub size: u32,
    pub style: u32,
    pub wnd_proc: WndProc,
    pub cls_extra: i32,
    pub wnd_extra: i32,
    pub instance: *mut c_void,
    pub icon: *mut c_void,
    pub cursor: *mut c_void,
    pub background: *mut c_void,
    pub menu_name: *const u16,
    pub class_name: *const u16,
    pub icon_small: *mut c_void,
}

#[repr(C)]
#[derive(Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
pub struct Msg {
    pub hwnd: Hwnd,
    pub message: u32,
    pub wparam: usize,
    pub lparam: isize,
    pub time: u32,
    pub pt: Point,
    pub private: u32,
}

#[link(name = "user32")]
extern "system" {
    pub fn RegisterClassExW(class: *const WndClassExW) -> u16;
    pub fn CreateWindowExW(
        ex_style: u32,
        class: *const u16,
        name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: Hwnd,
        menu: *mut c_void,
        instance: *mut c_void,
        param: *mut c_void,
    ) -> Hwnd;
    pub fn DefWindowProcW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
    pub fn GetMessageW(msg: *mut Msg, hwnd: Hwnd, min: u32, max: u32) -> i32;
    pub fn TranslateMessage(msg: *const Msg) -> i32;
    pub fn DispatchMessageW(msg: *const Msg) -> isize;
    pub fn ShowWindow(hwnd: Hwnd, cmd: i32) -> i32;
    fn LoadIconW(instance: *mut c_void, name: *const u16) -> *mut c_void;
}

#[link(name = "kernel32")]
extern "system" {
    pub fn GetModuleHandleW(name: *const u16) -> *mut c_void;
}

/// This executable's module handle, which windows must be created with to find classes registered by `register`.
pub fn instance() -> *mut c_void {
    unsafe { GetModuleHandleW(ptr::null()) }
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// The app icon compiled into the exe (ui/win/wisp.rc), at the system's standard size;
/// Windows picks the small size from the same resource.
pub fn app_icon() -> *mut c_void {
    unsafe { LoadIconW(instance(), 1 as *const u16) }
}

/// Registers a window class for this module; `class` must stay alive while windows of it exist.
pub fn register(class: &[u16], wnd_proc: WndProc, cursor: *mut c_void, background: *mut c_void, icon: *mut c_void) {
    let wc = WndClassExW {
        size: std::mem::size_of::<WndClassExW>() as u32,
        style: 0,
        wnd_proc,
        cls_extra: 0,
        wnd_extra: 0,
        instance: instance(),
        icon,
        cursor,
        background,
        menu_name: ptr::null(),
        class_name: class.as_ptr(),
        icon_small: ptr::null_mut(),
    };
    unsafe { RegisterClassExW(&wc) };
}
