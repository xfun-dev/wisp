//! Control on the secure desktop (UAC prompts, the lock screen, the sign-in screen)
//! (RULES.md W1–W8). Ordinary programs cannot send input there; a process running as
//! SYSTEM in the console session can. Turned on from the tray menu (one UAC prompt),
//! `wisp service-install` copies the exe under Program Files and registers
//! WispService, which keeps `wisp.exe client --system` running as SYSTEM in whichever
//! session owns the screen; that client moves its input onto whichever desktop has it.
//! SYSTEM runs and writes only where administrators alone can change things;
//! everything the user sees or asks for (the status window, bridge commands) runs as
//! the user.

use crate::win32::wide;
use std::ffi::c_void;
use std::io;
use std::path::PathBuf;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::time::Duration;

type Handle = *mut c_void;

#[repr(C)]
struct ServiceStatus {
    kind: u32,
    state: u32,
    accepted: u32,
    exit_code: u32,
    service_exit_code: u32,
    checkpoint: u32,
    wait_hint: u32,
}

#[repr(C)]
struct TableEntry {
    name: *const u16,
    main: Option<unsafe extern "system" fn(u32, *mut *mut u16)>,
}

#[repr(C)]
struct StartupInfo {
    cb: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    xc: u32,
    yc: u32,
    fill: u32,
    flags: u32,
    show: u16,
    reserved2: u16,
    reserved3: *mut u8,
    stdin: Handle,
    stdout: Handle,
    stderr: Handle,
}

#[repr(C)]
struct ProcessInfo {
    process: Handle,
    thread: Handle,
    pid: u32,
    tid: u32,
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenSCManagerW(machine: *const u16, db: *const u16, access: u32) -> Handle;
    fn CreateServiceW(scm: Handle, name: *const u16, display: *const u16, access: u32, kind: u32, start: u32, error: u32, path: *const u16, group: *const u16, tag: *mut u32, deps: *const u16, user: *const u16, password: *const u16) -> Handle;
    fn OpenServiceW(scm: Handle, name: *const u16, access: u32) -> Handle;
    fn StartServiceW(service: Handle, argc: u32, argv: *const *const u16) -> i32;
    fn ControlService(service: Handle, control: u32, status: *mut ServiceStatus) -> i32;
    fn DeleteService(service: Handle) -> i32;
    fn CloseServiceHandle(h: Handle) -> i32;
    fn StartServiceCtrlDispatcherW(table: *const TableEntry) -> i32;
    fn RegisterServiceCtrlHandlerExW(name: *const u16, handler: unsafe extern "system" fn(u32, u32, *mut c_void, *mut c_void) -> u32, ctx: *mut c_void) -> isize;
    fn SetServiceStatus(handle: isize, status: *const ServiceStatus) -> i32;
    fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
    fn DuplicateTokenEx(token: Handle, access: u32, attrs: *mut c_void, level: u32, kind: u32, out: *mut Handle) -> i32;
    fn SetTokenInformation(token: Handle, class: u32, info: *const c_void, len: u32) -> i32;
    #[cfg(feature = "bridge")]
    fn ImpersonateLoggedOnUser(token: Handle) -> i32;
    #[cfg(feature = "bridge")]
    fn RevertToSelf() -> i32;
    fn CreateProcessAsUserW(token: Handle, app: *const u16, cmd: *mut u16, pa: *mut c_void, ta: *mut c_void, inherit: i32, flags: u32, env: *mut c_void, dir: *const u16, si: *const StartupInfo, pi: *mut ProcessInfo) -> i32;
}

// wtsapi32 and userenv are loaded for each call, from System32, and let go again:
// the client shares this exe and must not keep them (RULES.md R1, R6)
type QueryUserToken = unsafe extern "system" fn(u32, *mut Handle) -> i32;
type CreateEnv = unsafe extern "system" fn(*mut *mut c_void, Handle, i32) -> i32;
type DestroyEnv = unsafe extern "system" fn(*mut c_void) -> i32;

/// Calls `name` from System32's `dll` through `call`, then unloads the dll; 0 if missing.
unsafe fn with_system_fn(dll: &str, name: &str, call: impl FnOnce(*mut c_void) -> i32) -> i32 {
    let module = LoadLibraryExW(wide(dll).as_ptr(), ptr::null_mut(), 0x800);
    if module.is_null() {
        return 0;
    }
    let f = GetProcAddress(module, format!("{name}\0").as_ptr());
    let out = if f.is_null() { 0 } else { call(f) };
    FreeLibrary(module);
    out
}

unsafe fn query_user_token(session: u32, token: *mut Handle) -> i32 {
    // SAFETY (all three): the export has this signature
    with_system_fn("wtsapi32.dll", "WTSQueryUserToken", |f| std::mem::transmute::<*mut c_void, QueryUserToken>(f)(session, token))
}

unsafe fn create_env_block(env: *mut *mut c_void, token: Handle, inherit: i32) -> i32 {
    with_system_fn("userenv.dll", "CreateEnvironmentBlock", |f| std::mem::transmute::<*mut c_void, CreateEnv>(f)(env, token, inherit))
}

unsafe fn destroy_env_block(env: *mut c_void) -> i32 {
    with_system_fn("userenv.dll", "DestroyEnvironmentBlock", |f| std::mem::transmute::<*mut c_void, DestroyEnv>(f)(env))
}

#[repr(C)]
struct ShellExecuteInfo {
    size: u32,
    mask: u32,
    hwnd: Handle,
    verb: *const u16,
    file: *const u16,
    params: *const u16,
    dir: *const u16,
    show: i32,
    instance: Handle,
    id_list: *mut c_void,
    class: *const u16,
    key: Handle,
    hotkey: u32,
    icon: Handle,
    process: Handle,
}

#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteExW(info: *mut ShellExecuteInfo) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    fn FreeLibrary(module: *mut c_void) -> i32;
    fn GetExitCodeProcess(h: Handle, code: *mut u32) -> i32;
    fn GetCurrentProcess() -> Handle;
    fn WTSGetActiveConsoleSessionId() -> u32;
    fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
    fn TerminateProcess(h: Handle, code: u32) -> i32;
    fn CloseHandle(h: Handle) -> i32;
    fn CreateDirectoryW(path: *const u16, attrs: *const SecurityAttributes) -> i32;
    fn LocalFree(mem: *mut c_void) -> *mut c_void;
    fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
}

#[link(name = "user32")]
extern "system" {
    fn OpenInputDesktop(flags: u32, inherit: i32, access: u32) -> Handle;
    fn SetThreadDesktop(desktop: Handle) -> i32;
    fn CloseDesktop(desktop: Handle) -> i32;
}

const NAME: &str = "WispService";
const SC_MANAGER_ALL_ACCESS: u32 = 0xF003F;
const SC_MANAGER_CONNECT: u32 = 0x1;
const SERVICE_QUERY_STATUS: u32 = 0x4;
const CREATE_UNICODE_ENVIRONMENT: u32 = 0x400;
const SEE_MASK_NOCLOSEPROCESS: u32 = 0x40;
const INFINITE: u32 = 0xFFFF_FFFF;
const SERVICE_ALL_ACCESS: u32 = 0xF01FF;
const SERVICE_WIN32_OWN_PROCESS: u32 = 0x10;
const SERVICE_AUTO_START: u32 = 2;
const SERVICE_ERROR_NORMAL: u32 = 1;
const SERVICE_RUNNING: u32 = 4;
const SERVICE_STOPPED: u32 = 1;
const SERVICE_STOP_PENDING: u32 = 3;
const SERVICE_CONTROL_STOP: u32 = 1;
const SERVICE_ACCEPT_STOP: u32 = 1;
const TOKEN_ALL_ACCESS: u32 = 0xF01FF;
const TOKEN_SESSION_ID: u32 = 12;
const SECURITY_IMPERSONATION: u32 = 2;
const TOKEN_PRIMARY: u32 = 1;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const GENERIC_ALL: u32 = 0x1000_0000;
use crate::logic::service;

fn exe() -> io::Result<PathBuf> {
    std::env::current_exe()
}

/// Where WispService and its client run from. Only administrators can change files
/// under Program Files, so SYSTEM never runs anything a user could swap (W5). Its
/// location comes from the machine's registry, which only administrators can change,
/// not from anyone's environment.
pub fn program_dir() -> PathBuf {
    const HKEY_LOCAL_MACHINE: isize = 0x8000_0002u32 as i32 as isize;
    const RRF_RT_REG_SZ: u32 = 0x2;
    let mut buf = [0u16; 260];
    let mut size = std::mem::size_of_val(&buf) as u32;
    let key = wide(r"SOFTWARE\Microsoft\Windows\CurrentVersion");
    let value = wide("ProgramFilesDir");
    let ok = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, ptr::null_mut(), buf.as_mut_ptr() as *mut c_void, &mut size) } == 0;
    let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
    let base = if ok && len > 0 { String::from_utf16_lossy(&buf[..len]) } else { r"C:\Program Files".into() };
    PathBuf::from(base).join("Wisp")
}

/// Where the SYSTEM client keeps its pairing, status and log, and where a developer
/// update waits: inside Program Files\Wisp, so no user can make it first or write
/// in it; users can read (W2). SYSTEM reads and writes nothing of the user's.
pub fn state_dir() -> PathBuf {
    program_dir().join("state")
}

/// Where the SYSTEM client's pairing key lives: only SYSTEM and administrators may
/// even read it, or another account on this PC could pose as the Mac (W2).
pub fn key_dir() -> PathBuf {
    state_dir().join("key")
}

/// key_dir, made with that protection if it is not there yet (only an administrator
/// can create anything under Program Files, so whatever is there was made here).
fn secret_dir() -> io::Result<PathBuf> {
    let dir = key_dir();
    std::fs::create_dir_all(state_dir())?;
    if dir.exists() {
        return Ok(dir);
    }
    let sddl = wide("D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)");
    unsafe {
        let mut sd = ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut sd, ptr::null_mut()) == 0 {
            return Err(io::Error::last_os_error());
        }
        let sa = SecurityAttributes { len: std::mem::size_of::<SecurityAttributes>() as u32, descriptor: sd, inherit: 0 };
        let ok = CreateDirectoryW(wide(&dir.to_string_lossy()).as_ptr(), &sa) != 0;
        let e = io::Error::last_os_error();
        LocalFree(sd);
        if !ok {
            return Err(e);
        }
    }
    Ok(dir)
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

/// `wisp service-pair <hex>`, as administrator: the pairing the user made, handed
/// over by the unelevated caller, becomes the SYSTEM client's (W2).
pub fn set_pair(hex: &str) -> io::Result<()> {
    let text = from_hex(hex).and_then(|b| String::from_utf8(b).ok()).filter(|t| crate::logic::pairing::Paired::parse(t).is_some());
    let text = text.ok_or_else(|| io::Error::other("配对信息无效"))?;
    let dir = secret_dir()?;
    let tmp = dir.join("pair.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, dir.join("pair"))
}

/// The user's pairing as `service-install`/`service-pair` take it; "-" for none.
pub fn user_pair_arg() -> String {
    match std::fs::read(crate::data_dir().join("pair")) {
        Ok(b) if !b.is_empty() => to_hex(&b),
        _ => "-".into(),
    }
}

/// After pairing in the status window: with WispService on, its client gets the new
/// pairing too (one UAC prompt).
pub fn sync_pair() -> io::Result<()> {
    elevated(&format!("service-pair {}", user_pair_arg()))
}

/// `wisp service-install <pairing hex|->`, as administrator: copies this exe under
/// Program Files, registers WispService (or repoints the existing one) and starts it.
/// The pairing comes from the unelevated caller as an argument: the elevated process
/// reads nothing in the user's folder, which may be another account's anyway (W1).
pub fn install(pair: &str) -> io::Result<()> {
    let exe = exe()?;
    let bin = program_dir();
    let copy = bin.join(SERVICE_EXE);
    let path = format!("\"{}\" service", copy.display());
    unsafe {
        let scm = OpenSCManagerW(ptr::null(), ptr::null(), SC_MANAGER_ALL_ACCESS);
        if scm.is_null() {
            return Err(io::Error::other("需要管理员权限"));
        }
        let mut svc = OpenServiceW(scm, wide(NAME).as_ptr(), SERVICE_ALL_ACCESS);
        if !svc.is_null() {
            // an update: stop it, so its files can be replaced
            stop_and_wait(svc);
        }
        let placed = (|| -> io::Result<()> {
            std::fs::create_dir_all(&bin)?;
            copy_retrying(&exe, &copy)?;
            copy_retrying(&exe, &bin.join(service::CLIENT_EXE))?;
            secret_dir()?;
            if pair != "-" {
                set_pair(pair)?;
            }
            Ok(())
        })();
        if let Err(e) = placed {
            if !svc.is_null() {
                // leave the old one running
                StartServiceW(svc, 0, ptr::null());
                CloseServiceHandle(svc);
            }
            CloseServiceHandle(scm);
            return Err(e);
        }
        if !svc.is_null() {
            // the files are in place: now point it at them (an earlier build ran elsewhere)
            const NO_CHANGE: u32 = 0xFFFF_FFFF;
            ChangeServiceConfigW(svc, NO_CHANGE, SERVICE_AUTO_START, NO_CHANGE, wide(&path).as_ptr(), ptr::null(), ptr::null_mut(), ptr::null(), ptr::null(), ptr::null(), ptr::null());
        } else {
            svc = CreateServiceW(scm, wide(NAME).as_ptr(), wide("Wisp").as_ptr(), SERVICE_ALL_ACCESS, SERVICE_WIN32_OWN_PROCESS, SERVICE_AUTO_START, SERVICE_ERROR_NORMAL, wide(&path).as_ptr(), ptr::null(), ptr::null_mut(), ptr::null(), ptr::null(), ptr::null());
        }
        if svc.is_null() {
            let e = io::Error::last_os_error();
            CloseServiceHandle(scm);
            return Err(e);
        }
        const ALREADY_RUNNING: i32 = 1056;
        let started = StartServiceW(svc, 0, ptr::null()) != 0;
        let e = io::Error::last_os_error();
        CloseServiceHandle(svc);
        CloseServiceHandle(scm);
        if !started && e.raw_os_error() != Some(ALREADY_RUNNING) {
            return Err(e);
        }
    }
    Ok(())
}

/// Stops and removes WispService and brings the per-user client back.
pub fn uninstall() -> io::Result<()> {
    unsafe {
        let scm = OpenSCManagerW(ptr::null(), ptr::null(), SC_MANAGER_ALL_ACCESS);
        if scm.is_null() {
            return Err(io::Error::other("需要管理员权限"));
        }
        let svc = OpenServiceW(scm, wide(NAME).as_ptr(), SERVICE_ALL_ACCESS);
        if !svc.is_null() {
            stop_and_wait(svc);
            DeleteService(svc);
            CloseServiceHandle(svc);
        }
        CloseServiceHandle(scm);
    }
    // the service process may take a moment to let go of its files
    let dir = program_dir();
    for _ in 0..20 {
        if std::fs::remove_dir_all(&dir).is_ok() || std::fs::symlink_metadata(&dir).is_err() {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    // still in use (an open status window, the very exe asking): gone at the next restart
    if dir.exists() {
        delete_at_restart(&dir);
    }
    // the unelevated caller brings the logon start and the per-user client back
    Ok(())
}

const SERVICE_EXE: &str = "wisp-service.exe";
/// a developer update, left in state_dir by the SYSTEM client (W5)
pub use crate::logic::service::NEXT_EXE;
/// the SYSTEM client's exit code when the user quits (W6)
pub use crate::logic::service::QUIT_EXIT;
/// sent by the user's `wisp.exe client` after a quit: start the client again (W6).
/// Interactive users may send a service its own control codes by default.
const CONTROL_WAKE: u32 = 128;
static WAKE: AtomicBool = AtomicBool::new(false);

/// The service has no console; a line in service.log says what it did.
fn log_line(what: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(state_dir().join("service.log")) {
        let _ = writeln!(f, "{} {what}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()));
    }
}

/// Opens the service with `access`; None when it is not installed.
/// Has Windows delete `path` (files first, then folders) when it next starts.
fn delete_at_restart(path: &std::path::Path) {
    const MOVEFILE_DELAY_UNTIL_REBOOT: u32 = 0x4;
    if let Ok(entries) = std::fs::read_dir(path) {
        for e in entries.flatten() {
            let p = e.path();
            // a link is removed itself, never followed
            if e.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink()) {
                delete_at_restart(&p);
            } else {
                unsafe { MoveFileExW(wide(&p.to_string_lossy()).as_ptr(), ptr::null(), MOVEFILE_DELAY_UNTIL_REBOOT) };
            }
        }
    }
    unsafe { MoveFileExW(wide(&path.to_string_lossy()).as_ptr(), ptr::null(), MOVEFILE_DELAY_UNTIL_REBOOT) };
}

unsafe fn open_service(access: u32) -> Option<(Handle, Handle)> {
    let scm = OpenSCManagerW(ptr::null(), ptr::null(), SC_MANAGER_CONNECT);
    if scm.is_null() {
        return None;
    }
    let svc = OpenServiceW(scm, wide(NAME).as_ptr(), access);
    if svc.is_null() {
        CloseServiceHandle(scm);
        return None;
    }
    Some((scm, svc))
}

/// Whether WispService is installed; any user may ask.
pub fn installed() -> bool {
    unsafe { open_service(SERVICE_QUERY_STATUS).map(|(scm, svc)| (CloseServiceHandle(svc), CloseServiceHandle(scm))).is_some() }
}

/// Asks a running WispService to start its client again after a quit. False when it
/// is not installed or not running: the caller then runs the per-user client, so the
/// user is never left with none (W6).
pub fn wake() -> bool {
    const SERVICE_USER_DEFINED_CONTROL: u32 = 0x100;
    unsafe {
        let Some((scm, svc)) = open_service(SERVICE_QUERY_STATUS | SERVICE_USER_DEFINED_CONTROL) else { return false };
        let mut st = std::mem::zeroed::<ServiceStatus>();
        let ok = QueryServiceStatus(svc, &mut st) != 0 && st.state == SERVICE_RUNNING && ControlService(svc, CONTROL_WAKE, &mut st) != 0;
        CloseServiceHandle(svc);
        CloseServiceHandle(scm);
        ok
    }
}

/// `wisp service-switch on|off`, run as the user (from the tray menu): one UAC
/// prompt. The logon start and the per-user client are this user's, so they are
/// handled here, unelevated; if the switch fails, things stay as they were.
pub fn switch(on: bool) -> io::Result<()> {
    if on {
        // the per-user client keeps the Mac in control until the switch succeeds; the
        // UAC prompt itself is on the secure desktop, out of its reach either way
        match elevated(&format!("service-install {}", user_pair_arg())) {
            Ok(()) => {
                crate::install_win::stop_user_client();
                crate::install_win::remove_run_key();
                // an earlier build ran the service from the user's folder; removed as the user
                let _ = std::fs::remove_file(crate::data_dir().join(SERVICE_EXE));
            }
            Err(e) => return Err(e),
        }
    } else {
        elevated("service-uninstall")?;
        // the user's own copy, not the Program Files one this may be running from
        let user_exe = crate::install_win::restore_run_key()?;
        std::process::Command::new(user_exe).arg("client").spawn()?;
    }
    Ok(())
}

fn copy_retrying(from: &std::path::Path, to: &std::path::Path) -> io::Result<()> {
    let mut last = None;
    for _ in 0..20 {
        match std::fs::copy(from, to) {
            Ok(_) => return Ok(()),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(last.unwrap_or_else(|| io::Error::other("复制失败")))
}

/// Runs `wisp.exe <params>` as administrator (one UAC prompt) and waits for it.
pub fn elevated(params: &str) -> io::Result<()> {
    let exe = exe()?;
    let verb = wide("runas");
    let file = wide(&exe.to_string_lossy());
    let params = wide(params);
    let mut info = ShellExecuteInfo {
        size: std::mem::size_of::<ShellExecuteInfo>() as u32,
        mask: SEE_MASK_NOCLOSEPROCESS,
        verb: verb.as_ptr(),
        file: file.as_ptr(),
        params: params.as_ptr(),
        ..unsafe { std::mem::zeroed() }
    };
    unsafe {
        if ShellExecuteExW(&mut info) == 0 || info.process.is_null() {
            return Err(io::Error::other("没有得到管理员授权"));
        }
        WaitForSingleObject(info.process, INFINITE);
        let mut code = 1;
        GetExitCodeProcess(info.process, &mut code);
        CloseHandle(info.process);
        if code != 0 {
            return Err(io::Error::other("设置没有完成"));
        }
    }
    Ok(())
}

static STOP: AtomicBool = AtomicBool::new(false);
static STATUS: AtomicIsize = AtomicIsize::new(0);

fn report(state: u32) {
    let st = ServiceStatus {
        kind: SERVICE_WIN32_OWN_PROCESS,
        state,
        // stop, and session changes (0x80)
        accepted: if state == SERVICE_RUNNING { SERVICE_ACCEPT_STOP | 0x80 } else { 0 },
        exit_code: 0,
        service_exit_code: 0,
        checkpoint: 0,
        wait_hint: 3000,
    };
    unsafe { SetServiceStatus(STATUS.load(Ordering::SeqCst), &st) };
}

unsafe extern "system" fn control(code: u32, event: u32, _: *mut c_void, _: *mut c_void) -> u32 {
    const SERVICE_CONTROL_SESSIONCHANGE: u32 = 0xE;
    const WTS_SESSION_LOGON: u32 = 5;
    match code {
        SERVICE_CONTROL_STOP => {
            STOP.store(true, Ordering::SeqCst);
            report(SERVICE_STOP_PENDING);
        }
        CONTROL_WAKE => WAKE.store(true, Ordering::SeqCst),
        // someone signed in: a quit from an earlier logon is over (W6)
        SERVICE_CONTROL_SESSIONCHANGE if event == WTS_SESSION_LOGON => WAKE.store(true, Ordering::SeqCst),
        SERVICE_CONTROL_SESSIONCHANGE => {}
        _ => return 120, // ERROR_CALL_NOT_IMPLEMENTED
    }
    0
}

/// `wisp service`, started by the service manager.
pub fn run() {
    let name = wide(NAME);
    let table = [TableEntry { name: name.as_ptr(), main: Some(service_main) }, TableEntry { name: ptr::null(), main: None }];
    unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) };
}

/// Stops the client if it still runs, and lets go of it.
unsafe fn end(child: &mut Option<(Handle, u32)>) {
    if let Some((h, _)) = child.take() {
        if WaitForSingleObject(h, 0) != 0 {
            TerminateProcess(h, 0);
            WaitForSingleObject(h, 3000);
        }
        CloseHandle(h);
    }
}

unsafe extern "system" fn service_main(_: u32, _: *mut *mut u16) {
    STATUS.store(RegisterServiceCtrlHandlerExW(wide(NAME).as_ptr(), control, ptr::null_mut()), Ordering::SeqCst);
    report(SERVICE_RUNNING);
    let bin = program_dir();
    let state = state_dir();
    // a fresh start of the service (boot) starts the client, as logging on would
    let mut quit = false;
    let mut child: Option<(Handle, u32)> = None;
    while !STOP.load(Ordering::SeqCst) {
        // a developer update waiting in the protected folder: stop the client, put it in place (W5)
        let next = state.join(NEXT_EXE);
        if next.exists() {
            end(&mut child);
            let ok = service::put_in_place(&next, &bin.join(service::CLIENT_EXE), &bin.join(service::OLD_EXE));
            log_line(if ok { "已换上新版本" } else { "新版本没能换上，继续用旧版本" });
        }
        let session = WTSGetActiveConsoleSessionId();
        let now = child.map(|(h, s)| {
            let alive = WaitForSingleObject(h, 0) != 0;
            if !alive {
                let mut code = 0u32;
                GetExitCodeProcess(h, &mut code);
                quit |= service::quit_by_user(code);
            }
            service::Child { alive, session: s }
        });
        // after the exit code, so a start asked for just as it quit is not lost
        if WAKE.swap(false, Ordering::SeqCst) {
            quit = false;
        }
        let step = service::step(now, quit, session);
        if step.stop {
            end(&mut child);
        }
        if step.start {
            child = launch(session, &bin, &state).map(|h| (h, session));
            if child.is_none() {
                log_line("客户端没能启动");
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    end(&mut child);
    report(SERVICE_STOPPED);
}

/// Starts `<bin>\wisp.exe client --system` as SYSTEM in `session`, on its default
/// desktop. Everything it reads and writes is in `state`.
unsafe fn launch(session: u32, bin: &std::path::Path, state: &std::path::Path) -> Option<Handle> {
    let exe = bin.join(service::CLIENT_EXE);
    // the pairing from the protected key folder; status and log in state
    std::env::set_var("WISP_DATA", key_dir());
    std::env::set_var("WISP_STATE", state);
    let mut own = ptr::null_mut();
    let mut token = ptr::null_mut();
    if OpenProcessToken(GetCurrentProcess(), TOKEN_ALL_ACCESS, &mut own) == 0 {
        return None;
    }
    let ok = DuplicateTokenEx(own, TOKEN_ALL_ACCESS, ptr::null_mut(), SECURITY_IMPERSONATION, TOKEN_PRIMARY, &mut token) != 0
        && SetTokenInformation(token, TOKEN_SESSION_ID, &session as *const u32 as *const c_void, 4) != 0;
    CloseHandle(own);
    if !ok {
        CloseHandle(token);
        return None;
    }
    let mut desktop = wide("winsta0\\default");
    let mut cmd = wide(&format!("\"{}\" client --system", exe.display()));
    let si = StartupInfo { cb: std::mem::size_of::<StartupInfo>() as u32, desktop: desktop.as_mut_ptr(), ..std::mem::zeroed() };
    let mut pi = std::mem::zeroed::<ProcessInfo>();
    let dir = wide(&bin.to_string_lossy());
    let ok = CreateProcessAsUserW(token, ptr::null(), cmd.as_mut_ptr(), ptr::null_mut(), ptr::null_mut(), 0, CREATE_NO_WINDOW, ptr::null_mut(), dir.as_ptr(), &si, &mut pi) != 0;
    CloseHandle(token);
    if !ok {
        return None;
    }
    CloseHandle(pi.thread);
    Some(pi.process)
}

static DESKTOP: AtomicIsize = AtomicIsize::new(0);

/// Moves the calling thread onto whichever desktop has the input now (the secure
/// desktop during a UAC prompt or at the lock screen). Only a SYSTEM process may.
/// Returns whether it switched.
pub fn follow_input_desktop() -> bool {
    unsafe {
        let d = OpenInputDesktop(0, 0, GENERIC_ALL);
        if d.is_null() {
            return false;
        }
        if SetThreadDesktop(d) == 0 {
            CloseDesktop(d);
            return false;
        }
        let old = DESKTOP.swap(d as isize, Ordering::SeqCst);
        if old != 0 {
            CloseDesktop(old as Handle);
        }
        true
    }
}

/// The signed-in user's token for the console session; None at the sign-in screen.
/// Only a SYSTEM process can ask.
fn user_token() -> Option<Handle> {
    let mut token = ptr::null_mut();
    let ok = unsafe { query_user_token(WTSGetActiveConsoleSessionId(), &mut token) } != 0;
    ok.then_some(token)
}

/// Starts `exe args` as the signed-in user (the status window from the tray, the
/// service switch); from an ordinary user process it simply starts it.
pub fn run_as_user(exe: &std::path::Path, args: &str) -> io::Result<()> {
    if !crate::client_win::is_system() {
        return std::process::Command::new(exe).args(args.split_whitespace()).spawn().map(|_| ());
    }
    let token = user_token().ok_or_else(|| io::Error::other("没有用户登录"))?;
    unsafe {
        let mut env = ptr::null_mut();
        // without the user's own environment it would get SYSTEM's: refuse instead
        if create_env_block(&mut env, token, 0) == 0 {
            CloseHandle(token);
            return Err(io::Error::other("无法取得用户的环境"));
        }
        let mut desktop = wide("winsta0\\default");
        let mut cmd = wide(&format!("\"{}\" {args}", exe.display()));
        let si = StartupInfo { cb: std::mem::size_of::<StartupInfo>() as u32, desktop: desktop.as_mut_ptr(), ..std::mem::zeroed() };
        let mut pi = std::mem::zeroed::<ProcessInfo>();
        let dir = wide(&exe.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default());
        let ok = CreateProcessAsUserW(token, ptr::null(), cmd.as_mut_ptr(), ptr::null_mut(), ptr::null_mut(), 0, CREATE_UNICODE_ENVIRONMENT, env, dir.as_ptr(), &si, &mut pi) != 0;
        if !env.is_null() {
            destroy_env_block(env);
        }
        CloseHandle(token);
        if !ok {
            return Err(io::Error::last_os_error());
        }
        CloseHandle(pi.thread);
        CloseHandle(pi.process);
    }
    Ok(())
}

#[cfg(feature = "bridge")]
/// Runs `f` on this thread as the signed-in user, so files a bridge request touches
/// get the user's rights, not SYSTEM's (W7). Without a signed-in user: refused.
pub fn as_user<T>(f: impl FnOnce() -> T) -> io::Result<T> {
    if !crate::client_win::is_system() {
        return Ok(f());
    }
    let token = user_token().ok_or_else(|| io::Error::other("没有用户登录，开发者通道不可用"))?;
    unsafe {
        if ImpersonateLoggedOnUser(token) == 0 {
            CloseHandle(token);
            return Err(io::Error::last_os_error());
        }
        let out = f();
        RevertToSelf();
        CloseHandle(token);
        Ok(out)
    }
}

#[cfg(feature = "bridge")]
/// The signed-in user's token and environment, for starting a bridge command as them.
pub struct UserLaunch {
    pub token: Handle,
    pub env: *mut c_void,
}

#[cfg(feature = "bridge")]
impl UserLaunch {
    pub fn get() -> io::Result<Option<UserLaunch>> {
        if !crate::client_win::is_system() {
            return Ok(None);
        }
        let token = user_token().ok_or_else(|| io::Error::other("没有用户登录，开发者通道不可用"))?;
        let mut env = ptr::null_mut();
        if unsafe { create_env_block(&mut env, token, 0) } == 0 {
            unsafe { CloseHandle(token) };
            return Err(io::Error::other("无法取得用户的环境"));
        }
        Ok(Some(UserLaunch { token, env }))
    }
}

#[cfg(feature = "bridge")]
impl Drop for UserLaunch {
    fn drop(&mut self) {
        unsafe {
            if !self.env.is_null() {
                destroy_env_block(self.env);
            }
            CloseHandle(self.token);
        }
    }
}

#[cfg(feature = "bridge")]
#[link(name = "kernel32")]
extern "system" {
    fn CreatePipe(read: *mut Handle, write: *mut Handle, attrs: *const SecurityAttributes, size: u32) -> i32;
}

#[repr(C)]
struct SecurityAttributes {
    len: u32,
    descriptor: *mut c_void,
    inherit: i32,
}

#[cfg(feature = "bridge")]
/// A command started as the signed-in user.
pub struct UserProcess {
    process: Handle,
}

#[cfg(feature = "bridge")]
/// Starts `cmdline` (through cmd.exe) as the signed-in user, output and errors on one
/// pipe, no window: a bridge command from WispService's client keeps the user's rights (W7).
pub fn spawn_as_user(user: &UserLaunch, cmdline: &str) -> io::Result<(UserProcess, std::fs::File)> {
    use std::os::windows::io::FromRawHandle;
    const STARTF_USESTDHANDLES: u32 = 0x100;
    // one at a time: a command started meanwhile would inherit this pipe's write end,
    // and this command's output would not end until that one did
    static SPAWNING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = SPAWNING.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let sa = SecurityAttributes { len: std::mem::size_of::<SecurityAttributes>() as u32, descriptor: ptr::null_mut(), inherit: 1 };
        let (mut read, mut write) = (ptr::null_mut(), ptr::null_mut());
        if CreatePipe(&mut read, &mut write, &sa, 0) == 0 {
            return Err(io::Error::last_os_error());
        }
        // only the write end goes to the child
        const HANDLE_FLAG_INHERIT: u32 = 1;
        #[link(name = "kernel32")]
        extern "system" {
            fn SetHandleInformation(h: Handle, mask: u32, flags: u32) -> i32;
        }
        SetHandleInformation(read, HANDLE_FLAG_INHERIT, 0);
        let mut desktop = wide("winsta0\\default");
        let mut cmd = wide(&format!("cmd /C ({cmdline}) 2>&1"));
        let si = StartupInfo {
            cb: std::mem::size_of::<StartupInfo>() as u32,
            desktop: desktop.as_mut_ptr(),
            flags: STARTF_USESTDHANDLES,
            stdout: write,
            stderr: write,
            ..std::mem::zeroed()
        };
        let mut pi = std::mem::zeroed::<ProcessInfo>();
        let ok = CreateProcessAsUserW(user.token, ptr::null(), cmd.as_mut_ptr(), ptr::null_mut(), ptr::null_mut(), 1, CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT, user.env, ptr::null(), &si, &mut pi) != 0;
        CloseHandle(write);
        if !ok {
            CloseHandle(read);
            return Err(io::Error::last_os_error());
        }
        CloseHandle(pi.thread);
        Ok((UserProcess { process: pi.process }, std::fs::File::from_raw_handle(read)))
    }
}

#[cfg(feature = "bridge")]
/// Waits up to `ms` for `child`; Some(exit code) once it ended.
pub fn wait(child: &UserProcess, ms: u32) -> Option<i32> {
    unsafe {
        if WaitForSingleObject(child.process, ms) != 0 {
            return None;
        }
        let mut code = 0u32;
        GetExitCodeProcess(child.process, &mut code);
        Some(code as i32)
    }
}

#[cfg(feature = "bridge")]
pub fn kill(child: &UserProcess) {
    unsafe { TerminateProcess(child.process, 1) };
}

#[cfg(feature = "bridge")]
impl Drop for UserProcess {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.process) };
    }
}

#[link(name = "advapi32")]
extern "system" {
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl: *const u16, revision: u32, sd: *mut *mut c_void, size: *mut u32) -> i32;
    fn RegGetValueW(key: isize, sub: *const u16, value: *const u16, flags: u32, kind: *mut u32, data: *mut c_void, size: *mut u32) -> i32;
    fn ChangeServiceConfigW(svc: Handle, kind: u32, start: u32, error: u32, path: *const u16, group: *const u16, tag: *mut u32, deps: *const u16, user: *const u16, password: *const u16, display: *const u16) -> i32;
    fn QueryServiceStatus(svc: Handle, status: *mut ServiceStatus) -> i32;
}

/// Security attributes letting the signed-in user wait on (see) an object the SYSTEM
/// client creates; kept for the life of the process.
pub fn open_to_user() -> *mut c_void {
    // SYSTEM: all; Everyone: SYNCHRONIZE
    let sddl = wide("D:(A;;GA;;;SY)(A;;0x00100000;;;WD)");
    let mut sd = ptr::null_mut();
    unsafe {
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut sd, ptr::null_mut()) == 0 {
            return ptr::null_mut();
        }
    }
    let attrs = Box::new(SecurityAttributes { len: std::mem::size_of::<SecurityAttributes>() as u32, descriptor: sd, inherit: 0 });
    Box::into_raw(attrs) as *mut c_void
}

/// Stops the service if it runs and waits (up to 10 s) until it has.
unsafe fn stop_and_wait(svc: Handle) {
    let mut st = std::mem::zeroed::<ServiceStatus>();
    ControlService(svc, SERVICE_CONTROL_STOP, &mut st);
    for _ in 0..50 {
        if QueryServiceStatus(svc, &mut st) == 0 || st.state == SERVICE_STOPPED {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}
