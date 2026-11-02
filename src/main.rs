// no console window on Windows: wisp runs in the background and logs to a file
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

macro_rules! log {
    ($($arg:tt)*) => { crate::write_log(format_args!($($arg)*)) };
}

#[cfg(all(target_os = "macos", feature = "bridge"))]
mod bridge_mac;
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
#[cfg(all(windows, feature = "bridge"))]
mod bridge_win;
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

/// `wisp win put <local> <remote> | get <remote> <local> | exec "<command>" [--timeout N]`:
/// the developer bridge from the command line (RULES.md X4), through the running core.
#[cfg(all(target_os = "macos", feature = "bridge"))]
fn win(args: &[String]) -> i32 {
    use control::hex;
    // the core reads and writes the local file, from its own working directory: make it absolute here
    let local = |p: &str| std::env::current_dir().map(|d| d.join(p).to_string_lossy().into_owned()).unwrap_or_else(|_| p.to_string());
    let line = match args {
        [op, a, b] if op == "put" => format!("bridge-put {} {}", hex(local(a).as_bytes()), hex(b.as_bytes())),
        [op, a, b] if op == "get" => format!("bridge-get {} {}", hex(a.as_bytes()), hex(local(b).as_bytes())),
        [op, cmd, rest @ ..] if op == "exec" => {
            let timeout = rest.iter().position(|a| a == "--timeout").and_then(|i| rest.get(i + 1)).and_then(|t| t.parse().ok()).unwrap_or(60u32);
            format!("bridge-exec {timeout} {}", hex(cmd.as_bytes()))
        }
        [on] if on == "on" || on == "off" => format!("bridge {on}"),
        _ => {
            eprintln!("用法: wisp win on | off | put <本地文件> <远程路径> | get <远程路径> <本地文件> | exec \"<命令>\" [--timeout 秒]");
            return 2;
        }
    };
    let reply = match control::read_token(&data_dir()).and_then(|t| control::request(&t, control::PORT, &line)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("wisp: 连不上正在运行的 Wisp：{e}");
            return 1;
        }
    };
    let mut words = reply.splitn(3, ' ');
    match (words.next(), words.next(), words.next()) {
        // "ok <code> <hex output>"; with no output the line ends after the code
        (Some("ok"), Some(code), out) if code.parse::<i32>().is_ok() => {
            let out = control::unhex(out.unwrap_or("")).unwrap_or_default();
            let _ = std::io::stdout().write_all(&out);
            code.parse().unwrap_or(1)
        }
        (Some("ok"), _, _) => 0,
        _ => {
            eprintln!("{}", reply.trim_start_matches("error "));
            1
        }
    }
}

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
        #[cfg(all(target_os = "macos", feature = "bridge"))]
        Some("win") => std::process::exit(win(&args[1..])),
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
