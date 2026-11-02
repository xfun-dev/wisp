//! Windows side: connects to the server and replays what it receives through
//! SendInput, so Windows sees it exactly like a locally attached mouse and keyboard.

use crate::clipboard_win::Clipboard;
use crate::shell_win::Shell;
use crate::discovery;
use crate::logic::replay::{mac_habit, to_absolute, Held, LEFT_CTRL};
use crate::logic::pairing::Paired;
use crate::logic::status::Status;
use crate::pair;
use crate::proto::{self, Msg};
use std::convert::Infallible;
use std::io;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[repr(C)]
#[derive(Clone, Copy)]
struct MouseInput {
    dx: i32,
    dy: i32,
    data: u32,
    flags: u32,
    time: u32,
    extra: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct KeybdInput {
    vk: u16,
    scan: u16,
    flags: u32,
    time: u32,
    extra: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
union InputData {
    mi: MouseInput,
    ki: KeybdInput,
}

#[repr(C)]
struct Input {
    kind: u32,
    data: InputData,
}

#[link(name = "user32")]
extern "system" {
    fn SendInput(n: u32, inputs: *const Input, size: i32) -> u32;
    fn GetSystemMetrics(index: i32) -> i32;
    fn SetProcessDPIAware() -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateMutexW(attrs: *mut std::ffi::c_void, owned: i32, name: *const u16) -> *mut std::ffi::c_void;
}

/// Held for the client's whole life, so the status window can tell whether it is running.
pub const RUNNING_MUTEX: &str = "Local\\WispClient";

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn status_file() -> std::path::PathBuf {
    crate::data_dir().join("status")
}

fn pair_file() -> std::path::PathBuf {
    crate::data_dir().join("pair")
}

/// Who this machine is paired with, if anyone (RULES.md P3).
fn paired() -> Option<Paired> {
    std::fs::read_to_string(pair_file()).ok().and_then(|t| Paired::parse(&t))
}

/// `wisp.exe pair <code>`: finds the Mac waiting to pair, runs the exchange with
/// `code` and saves the result; the running client picks it up within 2 s.
/// Returns the Mac's name; errors carry the words to show.
pub fn pair(code: &str, port: u16) -> io::Result<String> {
    let (addr, _) = discovery::find_pairing(&discovery::broadcast_targets(port), Duration::from_secs(2))?;
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
    let me = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Windows".to_string());
    let paired = pair::join(stream, code, &me)?;
    let tmp = pair_file().with_extension("tmp");
    std::fs::write(&tmp, paired.to_text()).and_then(|_| std::fs::rename(&tmp, pair_file()))?;
    Ok(paired.name)
}

/// The status file as the client last wrote it.
pub fn read_status() -> Option<String> {
    std::fs::read_to_string(status_file()).ok()
}

/// Replaces the status file whole (write, then rename), so the window never reads
/// half of it, then has the tray icon pick it up.
fn report(shell: &Shell, status: Status) {
    let file = status_file();
    let tmp = file.with_extension("tmp");
    let _ = std::fs::write(&tmp, status.to_text()).and_then(|_| std::fs::rename(&tmp, &file));
    shell.refresh();
}

const INPUT_MOUSE: u32 = 0;
const INPUT_KEYBOARD: u32 = 1;

const MOUSE_MOVE: u32 = 0x0001;
// each *UP flag is its *DOWN flag shifted left by one
const MOUSE_LEFTDOWN: u32 = 0x0002;
const MOUSE_RIGHTDOWN: u32 = 0x0008;
const MOUSE_MIDDLEDOWN: u32 = 0x0020;
const MOUSE_XDOWN: u32 = 0x0080;
const MOUSE_WHEEL: u32 = 0x0800;
const MOUSE_HWHEEL: u32 = 0x1000;
const MOUSE_ABSOLUTE: u32 = 0x8000;

const KEY_EXTENDED: u32 = 0x0001;
const KEY_UP: u32 = 0x0002;
const KEY_SCANCODE: u32 = 0x0008;

fn send_input(input: Input) {
    unsafe { SendInput(1, &input, std::mem::size_of::<Input>() as i32) };
}

fn send_mouse(dx: i32, dy: i32, data: u32, flags: u32) {
    send_input(Input {
        kind: INPUT_MOUSE,
        data: InputData { mi: MouseInput { dx, dy, data, flags, time: 0, extra: 0 } },
    });
}

fn send_key(scan: u16, down: bool) {
    let mut flags = KEY_SCANCODE;
    if scan & 0xFF00 == 0xE000 {
        flags |= KEY_EXTENDED;
    }
    if !down {
        flags |= KEY_UP;
    }
    send_input(Input {
        kind: INPUT_KEYBOARD,
        data: InputData { ki: KeybdInput { vk: 0, scan: scan & 0xFF, flags, time: 0, extra: 0 } },
    });
}

/// Sends a button press or release; buttons outside 1..=5 are ignored.
fn send_button(btn: u8, down: bool) {
    let (press, data) = match btn {
        1 => (MOUSE_LEFTDOWN, 0),
        2 => (MOUSE_RIGHTDOWN, 0),
        3 => (MOUSE_MIDDLEDOWN, 0),
        4 | 5 => (MOUSE_XDOWN, btn as u32 - 3),
        _ => return,
    };
    send_mouse(0, 0, data, if down { press } else { press << 1 });
}

/// Replays input, remembering what is held so nothing stays stuck.
struct Injector {
    w: i32,
    h: i32,
    held: Held,
}

impl Injector {
    fn apply(&mut self, msg: Msg) {
        match msg {
            Msg::MouseAbs { x, y } => {
                send_mouse(to_absolute(x, self.w), to_absolute(y, self.h), 0, MOUSE_MOVE | MOUSE_ABSOLUTE)
            }
            Msg::Button { btn, down } => {
                self.held.button(btn, down);
                send_button(btn, down);
            }
            Msg::Wheel { dx, dy } => {
                if dy != 0 {
                    send_mouse(0, 0, dy as u32, MOUSE_WHEEL);
                }
                if dx != 0 {
                    send_mouse(0, 0, dx as u32, MOUSE_HWHEEL);
                }
            }
            Msg::Zoom { steps } => {
                // Ctrl+wheel is the zoom gesture nearly every Windows app understands
                let ctrl_held = self.held.has_key(LEFT_CTRL);
                if !ctrl_held {
                    send_key(LEFT_CTRL, true);
                }
                send_mouse(0, 0, (steps * 120) as u32, MOUSE_WHEEL);
                if !ctrl_held {
                    send_key(LEFT_CTRL, false);
                }
            }
            Msg::Key { scan, down } => {
                let scan = mac_habit(scan);
                self.held.key(scan, down);
                send_key(scan, down);
            }
            Msg::Leave => self.release_all(),
            Msg::Enter | Msg::Ping | Msg::Info { .. } | Msg::Clipboard(_) | Msg::Name(_) | Msg::Bridge(_) => {}
        }
    }

    fn release_all(&mut self) {
        self.held.release_all(|scan| send_key(scan, false), |btn| send_button(btn, false));
    }
}

pub fn run(server: Option<&str>, port: u16) -> io::Result<()> {
    unsafe { SetProcessDPIAware() };
    let name: Vec<u16> = RUNNING_MUTEX.encode_utf16().chain(Some(0)).collect();
    unsafe { CreateMutexW(std::ptr::null_mut(), 0, name.as_ptr()) };
    let shell = Shell::spawn();
    report(&shell, Status { since: now(), ..Status::default() });
    let mut finder = Finder { server, port, psk: [0; 32], known: None };
    let mut clipboard = Clipboard::new();
    let mut last_err = String::new();
    loop {
        // re-read every round: pairing (or re-pairing) happens in another process
        let Some(p) = paired() else {
            let e = "还没有配对：在 Mac 的 Wisp 里点“配对新电脑”，再在这里输入配对码".to_string();
            if e != last_err {
                log!("wisp: {e}");
                report(&shell, Status { since: now(), problem: e.clone(), unpaired: true, ..Status::default() });
                last_err = e;
            }
            std::thread::sleep(Duration::from_secs(2));
            continue;
        };
        if finder.psk != p.key {
            finder.psk = p.key;
            finder.known = None;
        }
        let mut inj = Injector {
            w: unsafe { GetSystemMetrics(0) },
            h: unsafe { GetSystemMetrics(1) },
            held: Held::default(),
        };
        let Err(e) = session(&mut finder, &mut inj, &mut clipboard, &shell, &mut last_err);
        // while the Mac is away this repeats every 2 s; log each distinct reason once
        let e = e.to_string();
        if e != last_err {
            log!("wisp: 连接失败：{e}（每 2 秒重试）");
            report(&shell, Status { since: now(), problem: e.clone(), ..Status::default() });
            last_err = e;
        }
        inj.release_all();
        // the Mac is gone: give the pointer back to whoever uses this machine
        shell.uncover();
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// Where the server is: the `--server` override if given, else the last address
/// that worked, else whoever answers a LAN discovery query.
struct Finder<'a> {
    server: Option<&'a str>,
    port: u16,
    psk: [u8; 32],
    known: Option<SocketAddr>,
}

impl Finder<'_> {
    fn connect(&mut self) -> io::Result<TcpStream> {
        let timeout = Duration::from_secs(3);
        if let Some(server) = self.server {
            let addr = (server, self.port)
                .to_socket_addrs()?
                .next()
                .ok_or_else(|| io::Error::other("无法解析服务器地址"))?;
            return TcpStream::connect_timeout(&addr, timeout);
        }
        if let Some(addr) = self.known {
            if let Ok(stream) = TcpStream::connect_timeout(&addr, timeout) {
                return Ok(stream);
            }
        }
        let addr = discovery::find(&self.psk, &discovery::broadcast_targets(self.port), Duration::from_secs(1))?;
        if self.known != Some(addr) {
            log!("wisp: 在局域网找到 Mac：{addr}");
            self.known = Some(addr);
        }
        TcpStream::connect_timeout(&addr, timeout)
    }
}

fn session(
    finder: &mut Finder,
    inj: &mut Injector,
    clipboard: &mut Clipboard,
    shell: &Shell,
    last_err: &mut String,
) -> io::Result<Infallible> {
    let stream = finder.connect()?;
    let addr = stream.peer_addr()?;
    let (tx, rx) = proto::handshake(stream, &finder.psk, false)?;
    let queue = tx.spawn();
    queue
        .send(Msg::Info { w: inj.w as u16, h: inj.h as u16 })
        .map_err(|_| io::Error::other("连接在握手后立即断开"))?;
    log!("wisp: 已连接 {addr}，本机屏幕 {}×{}", inj.w, inj.h);
    let mut status = Status { connected: true, peer: addr.to_string(), since: now(), ..Status::default() };
    #[cfg(feature = "bridge")]
    crate::bridge_win::reset();
    report(shell, status.clone());
    last_err.clear();
    Err(rx.run(|msg| match msg {
        Msg::Clipboard(text) => clipboard.pull(&text),
        Msg::Name(name) => {
            log!("wisp: 对方是 {name}");
            status.name = name;
            report(shell, status.clone());
        }
        #[cfg(feature = "bridge")]
        Msg::Bridge(bytes) => {
            if crate::bridge_win::handle(&bytes, &queue) {
                status.bridge_until = crate::bridge_win::until();
                report(shell, status.clone());
            }
        }
        Msg::Enter => shell.uncover(),
        Msg::Leave => {
            inj.apply(Msg::Leave);
            clipboard.push(&queue);
            // control went back to the Mac: hide the pointer left behind (RULES.md H3)
            shell.cover();
        }
        other => inj.apply(other),
    }))
}
