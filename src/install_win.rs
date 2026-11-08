//! `Wisp.exe` is its own installer (RULES.md I1–I3). Double-clicked (no arguments) or
//! run as `wisp install`, it copies itself to %LOCALAPPDATA%\Wisp\wisp.exe, starts at
//! logon, puts "Wisp" in the Start menu and in Settings → Apps (to uninstall), starts
//! the background client and opens the status window. `wisp uninstall` undoes all of it.

use crate::client_win::RUNNING_MUTEX;
use crate::win32::{wide, Hwnd};
use std::ffi::c_void;
use std::io;
use std::path::{Path, PathBuf};
use std::ptr;
use std::time::Duration;

#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);

const CLSID_SHELL_LINK: Guid = Guid(0x0002_1401, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_SHELL_LINK_W: Guid = Guid(0x0002_14F9, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_PERSIST_FILE: Guid = Guid(0x0000_010B, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);


#[link(name = "advapi32")]
extern "system" {
    fn RegCreateKeyExW(key: isize, sub: *const u16, reserved: u32, class: *const u16, options: u32, access: u32, security: *mut c_void, out: *mut isize, disposition: *mut u32) -> i32;
    fn RegSetValueExW(key: isize, name: *const u16, reserved: u32, kind: u32, data: *const u8, size: u32) -> i32;
    fn RegDeleteValueW(key: isize, name: *const u16) -> i32;
    fn RegDeleteTreeW(key: isize, sub: *const u16) -> i32;
    fn RegCloseKey(key: isize) -> i32;
}

#[link(name = "user32")]
extern "system" {
    fn FindWindowW(class: *const u16, title: *const u16) -> Hwnd;
    fn PostMessageW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> i32;
    fn MessageBoxW(hwnd: Hwnd, text: *const u16, caption: *const u16, kind: u32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    fn OpenMutexW(access: u32, inherit: i32, name: *const u16) -> *mut c_void;
    fn CloseHandle(handle: *mut c_void) -> i32;
    fn GetStdHandle(which: u32) -> *mut c_void;
    fn SetHandleInformation(handle: *mut c_void, mask: u32, flags: u32) -> i32;
}

/// Children started here must not inherit whoever ran the installer: a script reading
/// its output (scripts/deploy.sh) would otherwise wait for the long-lived client to end.
fn keep_handles_to_myself() {
    const HANDLE_FLAG_INHERIT: u32 = 1;
    for which in [-10i32 as u32, -11i32 as u32, -12i32 as u32] {
        unsafe {
            let h = GetStdHandle(which);
            if !h.is_null() && h as isize != -1 {
                SetHandleInformation(h, HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
}

const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
const KEY_WRITE: u32 = 0x0002_0006;
const REG_SZ: u32 = 1;
const REG_DWORD: u32 = 4;
const SYNCHRONIZE: u32 = 0x0010_0000;
const WM_CLOSE: u32 = 0x0010;
const MB_ICONINFORMATION: u32 = 0x40;
const MB_ICONERROR: u32 = 0x10;
const COINIT_APARTMENTTHREADED: u32 = 0x2;
const CLSCTX_INPROC_SERVER: u32 = 0x1;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Wisp";

fn install_dir() -> PathBuf {
    crate::data_dir()
}

fn start_menu_link() -> PathBuf {
    PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join(r"Microsoft\Windows\Start Menu\Programs\Wisp.lnk")
}

fn client_running() -> bool {
    let handle = unsafe { OpenMutexW(SYNCHRONIZE, 0, wide(RUNNING_MUTEX).as_ptr()) };
    if handle.is_null() {
        return false;
    }
    unsafe { CloseHandle(handle) };
    true
}

/// Asks a running client (and its status window) to quit, and waits until it has.
/// Closes the status window, leaving the client alone.
fn close_status_window() {
    unsafe {
        let status = FindWindowW(wide("WispStatus").as_ptr(), ptr::null());
        if !status.is_null() {
            PostMessageW(status, WM_CLOSE, 0, 0);
        }
    }
    std::thread::sleep(Duration::from_millis(300));
}

/// Asks the per-user client to quit, without waiting for the shared running mark.
pub fn stop_user_client() {
    unsafe {
        let tray = FindWindowW(wide("WispTray").as_ptr(), ptr::null());
        if !tray.is_null() {
            PostMessageW(tray, crate::shell_win::WM_QUIT_WISP, 0, 0);
        }
    }
}

pub fn stop_running() {
    unsafe {
        let status = FindWindowW(wide("WispStatus").as_ptr(), ptr::null());
        if !status.is_null() {
            PostMessageW(status, WM_CLOSE, 0, 0);
        }
        let tray = FindWindowW(wide("WispTray").as_ptr(), ptr::null());
        if !tray.is_null() {
            PostMessageW(tray, crate::shell_win::WM_QUIT_WISP, 0, 0);
        }
    }
    for _ in 0..50 {
        if !client_running() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // the exe stays locked for a moment after the process ends
    std::thread::sleep(Duration::from_millis(300));
}

fn reg_set(sub: &str, values: &[(&str, Value)]) -> io::Result<()> {
    let mut key = 0isize;
    let status = unsafe { RegCreateKeyExW(HKEY_CURRENT_USER, wide(sub).as_ptr(), 0, ptr::null(), 0, KEY_WRITE, ptr::null_mut(), &mut key, ptr::null_mut()) };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status));
    }
    for (name, value) in values {
        let name = wide(name);
        let status = match value {
            Value::Text(s) => {
                let data = wide(s);
                unsafe { RegSetValueExW(key, name.as_ptr(), 0, REG_SZ, data.as_ptr() as *const u8, (data.len() * 2) as u32) }
            }
            Value::Number(n) => unsafe { RegSetValueExW(key, name.as_ptr(), 0, REG_DWORD, n as *const u32 as *const u8, 4) },
        };
        if status != 0 {
            unsafe { RegCloseKey(key) };
            return Err(io::Error::from_raw_os_error(status));
        }
    }
    unsafe { RegCloseKey(key) };
    Ok(())
}

enum Value {
    Text(String),
    Number(u32),
}

/// A Start menu shortcut, through the shell's IShellLink (no PowerShell, no script).
fn make_shortcut(link: &Path, target: &Path, args: &str) -> io::Result<()> {
    type Release = unsafe extern "system" fn(*mut c_void) -> u32;
    type SetString = unsafe extern "system" fn(*mut c_void, *const u16) -> i32;
    type QueryInterface = unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32;
    type Save = unsafe extern "system" fn(*mut c_void, *const u16, i32) -> i32;
    type CoInitializeEx = unsafe extern "system" fn(*mut c_void, u32) -> i32;
    type CoCreateInstance = unsafe extern "system" fn(*const Guid, *mut c_void, u32, *const Guid, *mut *mut c_void) -> i32;
    let fail = |what: &str, hr: i32| io::Error::other(format!("{what} 失败（0x{hr:08X}）"));
    unsafe {
        // loaded here, not linked: the background client shares this exe (RULES.md R6)
        let ole = LoadLibraryExW(wide("ole32.dll").as_ptr(), ptr::null_mut(), 0x800);
        let (init, create) = (GetProcAddress(ole, b"CoInitializeEx\0".as_ptr()), GetProcAddress(ole, b"CoCreateInstance\0".as_ptr()));
        if ole.is_null() || init.is_null() || create.is_null() {
            return Err(io::Error::other("无法加载 ole32.dll"));
        }
        // SAFETY: the two exports have these signatures
        let (init, create): (CoInitializeEx, CoCreateInstance) = (std::mem::transmute(init), std::mem::transmute(create));
        init(ptr::null_mut(), COINIT_APARTMENTTHREADED);
        let mut shell = ptr::null_mut();
        let hr = create(&CLSID_SHELL_LINK, ptr::null_mut(), CLSCTX_INPROC_SERVER, &IID_SHELL_LINK_W, &mut shell);
        if hr < 0 {
            return Err(fail("创建快捷方式", hr));
        }
        // SAFETY: IShellLinkW's vtable: QueryInterface 0, Release 2, SetDescription 7,
        // SetWorkingDirectory 9, SetArguments 11, SetIconLocation 17, SetPath 20
        let vt = *(shell as *const *const usize);
        let call = |slot: usize, s: &str| {
            let f: SetString = std::mem::transmute(*vt.add(slot));
            f(shell, wide(s).as_ptr())
        };
        call(20, &target.to_string_lossy());
        call(11, args);
        call(9, &target.parent().unwrap_or(target).to_string_lossy());
        call(7, "Wisp：连接状态与配对");
        let qi: QueryInterface = std::mem::transmute(*vt.add(0));
        let mut file = ptr::null_mut();
        let hr = qi(shell, &IID_PERSIST_FILE, &mut file);
        let release: Release = std::mem::transmute(*vt.add(2));
        if hr < 0 {
            release(shell);
            return Err(fail("保存快捷方式", hr));
        }
        // IPersistFile: Release 2, Save 6
        let fvt = *(file as *const *const usize);
        let save: Save = std::mem::transmute(*fvt.add(6));
        let hr = save(file, wide(&link.to_string_lossy()).as_ptr(), 1);
        let file_release: Release = std::mem::transmute(*fvt.add(2));
        file_release(file);
        release(shell);
        if hr < 0 {
            return Err(fail("保存快捷方式", hr));
        }
    }
    Ok(())
}

fn message(text: &str, error: bool) {
    unsafe {
        MessageBoxW(ptr::null_mut(), wide(text).as_ptr(), wide("Wisp").as_ptr(), if error { MB_ICONERROR } else { MB_ICONINFORMATION });
    }
}

fn copy_self(to: &Path) -> io::Result<()> {
    let from = std::env::current_exe()?;
    let mut last = None;
    for _ in 0..20 {
        match std::fs::copy(&from, to) {
            Ok(_) => return Ok(()),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err(last.unwrap_or_else(|| io::Error::other("复制失败")))
}

/// Starts `exe arg` sharing nothing with this process: whoever ran the installer (a
/// script waiting on its output, say) is not kept waiting by the long-lived client.
fn detached(exe: &Path, arg: &str) -> io::Result<()> {
    use std::process::{Command, Stdio};
    Command::new(exe).arg(arg).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(|_| ())
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn register(exe: &Path, logon_start: bool) -> io::Result<()> {
    let quoted = format!("\"{}\"", exe.display());
    if logon_start {
        reg_set(RUN_KEY, &[("Wisp", Value::Text(format!("{quoted} client")))])?;
    }
    let size_kb = std::fs::metadata(exe).map_or(0, |m| (m.len() / 1024) as u32);
    reg_set(UNINSTALL_KEY, &[
        ("DisplayName", Value::Text("Wisp".into())),
        ("DisplayVersion", Value::Text(env!("CARGO_PKG_VERSION").into())),
        ("Publisher", Value::Text("xfun-dev".into())),
        ("DisplayIcon", Value::Text(format!("{},0", exe.display()))),
        ("InstallLocation", Value::Text(install_dir().display().to_string())),
        ("UninstallString", Value::Text(format!("{quoted} uninstall"))),
        ("QuietUninstallString", Value::Text(format!("{quoted} uninstall --quiet"))),
        ("URLInfoAbout", Value::Text("https://github.com/xfun-dev/wisp".into())),
        ("EstimatedSize", Value::Number(size_kb)),
        ("NoModify", Value::Number(1)),
        ("NoRepair", Value::Number(1)),
    ])?;
    make_shortcut(&start_menu_link(), exe, "status")
}

/// WispService has taken over (RULES.md W1): the per-user client no longer starts
/// at logon, so the two never fight over the link.
pub fn remove_run_key() {
    let mut key = 0isize;
    unsafe {
        if RegCreateKeyExW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), 0, ptr::null(), 0, KEY_WRITE, ptr::null_mut(), &mut key, ptr::null_mut()) == 0 {
            RegDeleteValueW(key, wide("Wisp").as_ptr());
            RegCloseKey(key);
        }
    }
}

/// The per-user client starts at logon again; returns its exe.
pub fn restore_run_key() -> io::Result<PathBuf> {
    let exe = install_dir().join("wisp.exe");
    reg_set(RUN_KEY, &[("Wisp", Value::Text(format!("\"{}\" client", exe.display())))])?;
    Ok(exe)
}

/// Installs (or updates) this exe and opens the status window, unless `quiet`.
/// `user_only` (developer deploys, RULES.md X1): with WispService on, update only this
/// user's copy; the service's arrives over the bridge.
pub fn install(quiet: bool, user_only: bool) -> ! {
    keep_handles_to_myself();
    let dir = install_dir();
    let exe = dir.join("wisp.exe");
    let result = (|| -> io::Result<()> {
        std::fs::create_dir_all(&dir)?;
        let current = std::env::current_exe()?;
        if crate::service_win::installed() {
            // WispService runs the client from Program Files: this user's copy (status
            // window, menus) is updated here, the service's with one UAC prompt (W5).
            // No logon start, and the service's client is not asked to quit.
            if !same_file(&current, &exe) {
                close_status_window();
                copy_self(&exe)?;
            }
            register(&exe, false)?;
            if !user_only {
                crate::service_win::elevated(&format!("service-install {}", crate::service_win::user_pair_arg()))
                    .map_err(|e| io::Error::other(format!("开着「锁屏和权限弹窗也能控制」时，更新需要管理员授权（{e}）")))?;
            }
            return Ok(());
        }
        if !same_file(&current, &exe) {
            stop_running();
            copy_self(&exe)?;
        }
        register(&exe, true)?;
        if !client_running() {
            detached(&exe, "client")?;
        }
        Ok(())
    })();
    match result {
        Ok(()) if quiet => std::process::exit(0),
        Ok(()) => {
            // the status window, from the installed copy, so it shows the pairing tray
            let _ = detached(&exe, "status");
            std::process::exit(0);
        }
        Err(e) => {
            if !quiet {
                message(&format!("Wisp 安装失败：{e}"), true);
            }
            std::process::exit(1);
        }
    }
}

/// Stops Wisp and removes everything `install` added. The folder (the exe itself is
/// still running here) is removed by a short-lived cmd after this process exits.
pub fn uninstall(quiet: bool) -> ! {
    keep_handles_to_myself();
    // WispService goes first, as administrator (one UAC prompt); without it, nothing is removed
    if crate::service_win::installed() {
        if let Err(e) = crate::service_win::elevated("service-uninstall") {
            if !quiet {
                message(&format!("Wisp 没有卸载：{e}"), true);
            }
            std::process::exit(1);
        }
    }
    stop_running();
    let mut key = 0isize;
    unsafe {
        if RegCreateKeyExW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), 0, ptr::null(), 0, KEY_WRITE, ptr::null_mut(), &mut key, ptr::null_mut()) == 0 {
            RegDeleteValueW(key, wide("Wisp").as_ptr());
            RegCloseKey(key);
        }
        RegDeleteTreeW(HKEY_CURRENT_USER, wide(UNINSTALL_KEY).as_ptr());
    }
    let _ = std::fs::remove_file(start_menu_link());
    let dir = install_dir();
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // raw: cmd does not understand the \" that normal argument quoting would produce
    let _ = std::process::Command::new("cmd")
        .raw_arg(format!("/C ping -n 3 127.0.0.1 >nul & rmdir /S /Q \"{}\"", dir.display()))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    if !quiet {
        message("Wisp 已卸载。", false);
    }
    std::process::exit(0);
}
