//! Local control port for the settings window and `wisp ctl` (RULES.md U6):
//! loopback only, a local token first, then one text command per line, each
//! answered with one line starting with `ok` or `error`. A status line carries
//! tab-separated `key=value` fields, since names contain spaces.

use crate::logic::pairing::{code_from, Paired, Window};
use crate::logic::settings::Settings;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const PORT: u16 = 24802;
const TOKEN_FILE: &str = "ctl-token";

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Replaces `file` whole (write, then rename), so a crash never leaves half of it.
fn save(file: &Path, text: &str) -> io::Result<()> {
    let tmp = file.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, file)
}

/// The secret local clients present, created by the core on first start. It is
/// separate from the pairing key so the settings window works before any pairing.
pub fn ensure_token(dir: &Path) -> io::Result<String> {
    if let Ok(t) = read_token(dir) {
        return Ok(t);
    }
    let mut raw = [0u8; 32];
    getrandom::getrandom(&mut raw).map_err(|_| io::Error::other("no randomness"))?;
    let token: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    std::fs::create_dir_all(dir)?;
    save(&dir.join(TOKEN_FILE), &token)?;
    Ok(token)
}

pub fn read_token(dir: &Path) -> io::Result<String> {
    Ok(std::fs::read_to_string(dir.join(TOKEN_FILE))?.trim().to_string())
}

#[derive(Debug, PartialEq)]
pub enum Command {
    Status,
    Set(String, String),
    Pause,
    Resume,
    /// stop wisp; launchd only restarts it after a crash, so it stays stopped (RULES.md U11)
    Quit,
    /// open a pairing code (RULES.md P1)
    Pair,
    PairCancel,
    /// forget the paired machine and drop its connection (RULES.md P5)
    Unpair,
    /// test only: block the event thread for this many milliseconds (RULES.md S4)
    DebugStall(u64),
}

pub fn parse(line: &str) -> Result<Command, String> {
    let words: Vec<&str> = line.split_whitespace().collect();
    match words.as_slice() {
        ["status"] => Ok(Command::Status),
        ["pause"] => Ok(Command::Pause),
        ["resume"] => Ok(Command::Resume),
        ["quit"] => Ok(Command::Quit),
        ["pair"] => Ok(Command::Pair),
        ["pair-cancel"] => Ok(Command::PairCancel),
        ["unpair"] => Ok(Command::Unpair),
        ["debug-stall", ms] => ms.parse().map(Command::DebugStall).map_err(|_| "用法：debug-stall 毫秒".into()),
        ["set", kv] => kv.split_once('=').map(|(k, v)| Command::Set(k.into(), v.into())).ok_or("用法：set 设置=值".into()),
        _ => Err(format!("不认识的命令：{}", line.trim())),
    }
}

/// Everything the event tap, the network side and the control port share: settings
/// and pause (RULES.md U3, U4), who we are paired with, and an open pairing code.
pub struct Live {
    settings: Mutex<Settings>,
    paused: AtomicBool,
    paired: Mutex<Option<Paired>>,
    /// the open invitation, if any; `pair::host` consumes it
    pub pairing: Mutex<Option<Window>>,
    dir: PathBuf,
}

impl Live {
    /// Reads `config` and `pair` from the data directory `dir`.
    pub fn load(dir: PathBuf) -> Live {
        let read = |name: &str| std::fs::read_to_string(dir.join(name)).ok();
        Live {
            settings: Mutex::new(read("config").map(|t| Settings::parse(&t)).unwrap_or_default()),
            paused: AtomicBool::new(false),
            paired: Mutex::new(read("pair").and_then(|t| Paired::parse(&t))),
            pairing: Mutex::new(None),
            dir,
        }
    }

    pub fn settings(&self) -> Settings {
        *self.settings.lock().unwrap()
    }

    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub fn paired(&self) -> Option<Paired> {
        self.paired.lock().unwrap().clone()
    }

    /// Saves (or with None, forgets) the pairing; the caller drops any old link.
    pub fn set_paired(&self, paired: Option<Paired>) -> io::Result<()> {
        let file = self.dir.join("pair");
        match &paired {
            Some(p) => save(&file, &p.to_text())?,
            None => match std::fs::remove_file(&file) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            },
        }
        *self.paired.lock().unwrap() = paired;
        Ok(())
    }

    /// The open pairing code, if any is still usable.
    pub fn pairing_code(&self) -> Option<String> {
        self.pairing.lock().unwrap().as_ref().and_then(|w| w.code(now()).map(str::to_string))
    }

    fn apply(&self, cmd: Command, drop_link: &dyn Fn()) -> Result<String, String> {
        match cmd {
            Command::Set(key, value) => {
                // the event thread reads settings on every move: never hold the lock over disk I/O
                let mut next = self.settings();
                next.set(&key, &value)?;
                save(&self.dir.join("config"), &next.to_text()).map_err(|e| format!("保存设置失败：{e}"))?;
                *self.settings.lock().unwrap() = next;
            }
            Command::Pause => self.paused.store(true, Ordering::Relaxed),
            Command::Resume => self.paused.store(false, Ordering::Relaxed),
            Command::Pair => {
                let mut raw = [0u8; 4];
                getrandom::getrandom(&mut raw).map_err(|_| "无法生成配对码".to_string())?;
                let code = code_from(u32::from_le_bytes(raw));
                *self.pairing.lock().unwrap() = Some(Window::open(code.clone(), now()));
                return Ok(format!("ok code={code}"));
            }
            Command::PairCancel => *self.pairing.lock().unwrap() = None,
            Command::Unpair => {
                self.set_paired(None).map_err(|e| format!("解除配对失败：{e}"))?;
                drop_link();
            }
            Command::Status | Command::Quit | Command::DebugStall(_) => {}
        }
        Ok("ok".into())
    }

    fn status(&self, connection: &str) -> String {
        let s = self.settings();
        let paired = self.paired().map(|p| p.name).unwrap_or_default();
        let window = self.pairing.lock().unwrap().clone();
        let t = now();
        let (code, left, tries) = match window.as_ref().and_then(|w| w.code(t).map(|c| (c.to_string(), w))) {
            Some((c, w)) => (c, w.seconds_left(t), w.tries_left()),
            None => (String::new(), 0, 0),
        };
        format!(
            "{connection}\tpaused={}\tedge={}\tspeed={}\tscroll={}\tpaired={paired}\tpairing={code}\tpair_left={left}\tpair_tries={tries}",
            self.paused(),
            s.edge.name(),
            s.speed,
            s.scroll
        )
    }
}

/// What the control port needs from the network side.
pub struct Link {
    /// tab-separated fields describing the connection, for `status`
    pub describe: Box<dyn Fn() -> String + Send>,
    /// drops the current connection, after unpairing
    pub drop: Box<dyn Fn() + Send>,
    /// asks the event thread to block for a while, for the S4 test
    pub stall: Box<dyn Fn(u64) + Send>,
}

/// Serves local clients forever, one at a time on this thread: a request takes well
/// under a millisecond, and the settings window asks once a second, so a thread and an
/// 8 KB buffer per request only churned memory (RULES.md R5). A client that stalls is
/// dropped after a short timeout so it cannot hold up the next one.
pub fn serve(listener: TcpListener, token: String, live: Arc<Live>, link: Link) {
    for stream in listener.incoming().flatten() {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let _ = session(stream, &token, &live, &link);
    }
}

fn session(stream: TcpStream, token: &str, live: &Live, link: &Link) -> io::Result<()> {
    let mut out = stream.try_clone()?;
    let mut lines = BufReader::with_capacity(256, stream).lines();
    if lines.next().transpose()?.as_deref() != Some(&format!("auth {token}")) {
        return writeln!(out, "error 认证失败");
    }
    for line in lines {
        let reply = match parse(&line?) {
            Ok(Command::Status) => format!("ok\t{}", live.status(&(link.describe)())),
            Ok(Command::Quit) => {
                writeln!(out, "ok")?;
                log!("wisp: 用户退出");
                // a clean exit: launchd (KeepAlive only on failure) leaves it stopped
                std::process::exit(0);
            }
            Ok(Command::DebugStall(ms)) => {
                (link.stall)(ms.min(10_000));
                "ok".into()
            }
            Ok(cmd) => live.apply(cmd, &*link.drop).unwrap_or_else(|e| format!("error {e}")),
            Err(e) => format!("error {e}"),
        };
        writeln!(out, "{reply}")?;
    }
    Ok(())
}

/// `wisp ctl <command…>`: sends one command to the running core and returns its reply.
pub fn request(token: &str, port: u16, command: &str) -> io::Result<String> {
    let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))?;
    writeln!(&stream, "auth {token}\n{command}")?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
    Ok(reply.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn control_commands_parse_and_reject_unknown() {
        assert_eq!(parse("status"), Ok(Command::Status));
        assert_eq!(parse("  pause "), Ok(Command::Pause));
        assert_eq!(parse("quit"), Ok(Command::Quit));
        assert_eq!(parse("pair"), Ok(Command::Pair));
        assert_eq!(parse("pair-cancel"), Ok(Command::PairCancel));
        assert_eq!(parse("unpair"), Ok(Command::Unpair));
        assert_eq!(parse("debug-stall 3000"), Ok(Command::DebugStall(3000)));
        assert!(parse("debug-stall soon").is_err());
        assert_eq!(parse("set speed=1.5"), Ok(Command::Set("speed".into(), "1.5".into())));
        assert!(parse("set speed").is_err());
        assert!(parse("set a=1 b=2").is_err());
        assert!(parse("reboot").is_err());
        assert!(parse("").is_err());
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wisp-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A control port on a free loopback port; counts how often it dropped the link.
    fn start(live: Arc<Live>, token: &str) -> (u16, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let drops = Arc::new(AtomicUsize::new(0));
        let counter = drops.clone();
        let link = Link {
            describe: Box::new(|| "state=waiting".into()),
            drop: Box::new(move || {
                counter.fetch_add(1, Ordering::Relaxed);
            }),
            stall: Box::new(|_| {}),
        };
        let token = token.to_string();
        std::thread::spawn(move || serve(listener, token, live, link));
        (port, drops)
    }

    #[test]
    fn control_port_needs_the_token_and_persists_only_settings() {
        let dir = temp_dir("settings");
        let live = Arc::new(Live::load(dir.clone()));
        let (port, _) = start(live.clone(), "secret");

        assert_eq!(request("wrong", port, "pause").unwrap(), "error 认证失败");
        assert!(!live.paused(), "an unauthenticated client changes nothing");

        assert_eq!(request("secret", port, "set edge=left").unwrap(), "ok");
        assert_eq!(request("secret", port, "pause").unwrap(), "ok");
        assert!(request("secret", port, "set speed=99").unwrap().starts_with("error 鼠标速度"));
        assert_eq!(
            request("secret", port, "status").unwrap(),
            "ok\tstate=waiting\tpaused=true\tedge=left\tspeed=1\tscroll=3\tpaired=\tpairing=\tpair_left=0\tpair_tries=0"
        );

        let reloaded = Live::load(dir.clone());
        assert_eq!(reloaded.settings().edge.name(), "left", "settings survive a restart");
        assert!(!reloaded.paused(), "pause does not (RULES.md U4)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pairing_commands_open_a_code_and_unpair_forgets() {
        let dir = temp_dir("pairing");
        let live = Arc::new(Live::load(dir.clone()));
        live.set_paired(Some(Paired { name: "Office PC".into(), key: [1; 32] })).unwrap();
        let (port, drops) = start(live.clone(), "secret");

        let reply = request("secret", port, "pair").unwrap();
        let code = reply.strip_prefix("ok code=").expect("a code").to_string();
        assert_eq!(code.len(), 6);
        let status = request("secret", port, "status").unwrap();
        assert!(status.contains("\tpaired=Office PC\t"), "names with spaces survive: {status}");
        assert!(status.contains(&format!("\tpairing={code}\tpair_left=120\tpair_tries=3")), "{status}");

        assert_eq!(request("secret", port, "pair-cancel").unwrap(), "ok");
        assert!(live.pairing_code().is_none());

        assert_eq!(request("secret", port, "unpair").unwrap(), "ok");
        assert!(live.paired().is_none());
        assert!(!dir.join("pair").exists(), "the key file is gone");
        assert_eq!(drops.load(Ordering::Relaxed), 1, "the old link is dropped");
        assert!(Live::load(dir.clone()).paired().is_none(), "still unpaired after a restart");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_local_token_is_made_once_and_kept() {
        let dir = temp_dir("token");
        let first = ensure_token(&dir).unwrap();
        assert_eq!(first.len(), 64);
        assert_eq!(ensure_token(&dir).unwrap(), first);
        assert_eq!(read_token(&dir).unwrap(), first);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
