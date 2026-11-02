// no console window on Windows: wisp runs in the background and logs to a file
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

macro_rules! log {
    ($($arg:tt)*) => { crate::write_log(format_args!($($arg)*)) };
}

#[cfg(target_os = "macos")]
mod control;
mod discovery;
#[cfg(target_os = "macos")]
mod keymap;
mod logic;
mod pair;
#[cfg(target_os = "macos")]
mod pasteboard_mac;
mod proto;

#[cfg(target_os = "macos")]
mod server_mac;
#[cfg(windows)]
mod client_win;
#[cfg(windows)]
mod clipboard_win;
#[cfg(windows)]
mod shell_win;
#[cfg(windows)]
mod gdiplus_win;
#[cfg(windows)]
mod install_win;
#[cfg(windows)]
mod status_win;
#[cfg(windows)]
mod win32;

const USAGE: &str = "\
用法:
  wisp server [--port 24801]                                  (macOS；方向、速度等设置在数据目录的 config 文件里)
  wisp ctl status | pause | resume | quit | pair | pair-cancel | unpair | set <设置>=<值>
                                                              (macOS；查看、修改或退出正在运行的 Wisp，配对)
  wisp client [--server <Mac的IP>] [--port 24801]             (Windows，不填 --server 时自动在局域网里找 Mac)
  wisp status                                                 (Windows；打开连接状态窗口)
  wisp pair <6 位配对码>                                       (Windows；和正在等待配对的 Mac 配对)
  wisp install [--quiet] | uninstall [--quiet]                 (Windows；双击 Wisp.exe 等同 install)
两台电脑的密钥由配对生成，保存在数据目录的 pair 文件里。";

static LOG_FILE: OnceLock<Mutex<File>> = OnceLock::new();

fn write_log(args: std::fmt::Arguments) {
    eprintln!("{args}");
    if let Some(f) = LOG_FILE.get() {
        let _ = writeln!(f.lock().unwrap(), "{args}");
    }
}

/// ~/.config/wisp on macOS, %LOCALAPPDATA%\Wisp on Windows.
fn data_dir() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("Wisp")
    } else {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config/wisp")
    }
}

fn fail(code: i32, msg: &str) -> ! {
    log!("{msg}");
    std::process::exit(code);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(windows)]
    match args.first().map(String::as_str) {
        Some("status") => return status_win::run(),
        // double-clicked: Wisp.exe installs itself (RULES.md I1)
        None => install_win::install(false),
        Some("install") => install_win::install(args.iter().any(|a| a == "--quiet")),
        Some("uninstall") => install_win::uninstall(args.iter().any(|a| a == "--quiet")),
        _ => {}
    }
    #[cfg(target_os = "macos")]
    if args.first().map(String::as_str) == Some("pasteboard") {
        std::process::exit(pasteboard_mac::helper(&args[1..]));
    }
    // only the background client logs to a file (launchd captures the Mac server's
    // stderr); a status window starting must not truncate the client's log
    if cfg!(windows) && args.first().map(String::as_str) == Some("client") {
        if let Ok(f) = File::create(data_dir().join("wisp.log")) {
            let _ = LOG_FILE.set(Mutex::new(f));
        }
    }

    let opt = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let port = opt("--port").and_then(|p| p.parse().ok()).unwrap_or(proto::DEFAULT_PORT);

    let result = match args.first().map(String::as_str) {
        #[cfg(target_os = "macos")]
        Some("server") => server_mac::run(server_mac::Options {
            port,
            live: std::sync::Arc::new(control::Live::load(data_dir())),
        }),
        #[cfg(target_os = "macos")]
        Some("ctl") => match control::read_token(&data_dir()).and_then(|t| control::request(&t, control::PORT, &args[1..].join(" "))) {
            Ok(reply) => {
                println!("{reply}");
                std::process::exit(if reply.starts_with("ok") { 0 } else { 1 });
            }
            Err(e) => fail(1, &format!("wisp: 连不上正在运行的 Wisp：{e}")),
        },
        #[cfg(windows)]
        Some("client") => client_win::run(opt("--server").as_deref(), port),
        #[cfg(windows)]
        Some("pair") => match args.get(1).and_then(|c| logic::pairing::typed(c)) {
            Some(code) => match client_win::pair(&code, port) {
                Ok(name) => {
                    println!("已与 {name} 配对");
                    std::process::exit(0);
                }
                Err(e) => fail(1, &format!("配对失败：{e}")),
            },
            None => fail(2, "配对码是 6 位数字"),
        },
        _ => fail(2, USAGE),
    };
    if let Err(e) = result {
        fail(1, &format!("wisp: {e}"));
    }
}
