//! Windows side of the developer bridge (RULES.md X1–X5), built only with the
//! `bridge` feature. Requests are refused unless the connected Mac has opened the
//! bridge and its 8 hours are not up; commands and file reads run on their own
//! threads so input injection never waits for them.

use crate::logic::bridge::{self, Bridge};
use crate::proto::{Msg, Queue};
use std::io::{Read, Write};
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const MAX_OUTPUT: usize = bridge::MAX_OUTPUT;
/// `wisp win put <exe> wisp:update`: the service's next client (RULES.md W5)
const UPDATE_PATH: &str = "wisp:update";

/// Files being sent to the Mac, each waiting for the Mac's Ack of its last piece.
static SENDING: Mutex<Option<HashMap<u32, mpsc::Sender<()>>>> = Mutex::new(None);

/// Unix time the bridge closes; 0 closed. Set only by the Mac, reset on every new link.
static UNTIL: AtomicU64 = AtomicU64::new(0);

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

pub fn until() -> u64 {
    UNTIL.load(Ordering::Relaxed)
}

/// A new link: closed until the Mac says otherwise.
pub fn reset() {
    UNTIL.store(0, Ordering::Relaxed);
}

fn open() -> bool {
    bridge::left(now(), until()) > 0
}

fn answer(queue: &Queue, msg: Bridge) {
    let _ = queue.send(Msg::Bridge(msg.encode()));
}

fn refuse(queue: &Queue, id: u32) {
    answer(queue, Bridge::Done { id, code: -3, data: "开发者通道没有开启".as_bytes().to_vec() });
}

/// Handles one bridge message from the Mac. Returns true when the open/closed state
/// changed, so the caller can update the status file.
pub fn handle(bytes: &[u8], queue: &Queue) -> bool {
    let Some(msg) = Bridge::decode(bytes) else { return false };
    match msg {
        Bridge::State { secs } => {
            let next = if secs == 0 { 0 } else { bridge::until(now(), secs) };
            let was = UNTIL.swap(next, Ordering::Relaxed);
            log!("wisp: 开发者通道{}", if next > 0 { "已开启" } else { "已关闭" });
            return (was > 0) != (next > 0) || was != next;
        }
        Bridge::Exec { id, .. } | Bridge::Put { id, .. } | Bridge::Get { id, .. } if !open() => refuse(queue, id),
        Bridge::Exec { id, timeout, cmd } => {
            let queue = queue.clone();
            small_thread(move || {
                let (code, out) = run(&cmd, timeout);
                answer(&queue, Bridge::Done { id, code, data: out });
            });
        }
        // a new client for WispService, from the paired Mac with the bridge open: written
        // by SYSTEM into the protected folder, whole before the service sees it (W5)
        Bridge::Put { id, offset, last, path, data } if path == UPDATE_PATH => {
            let written = (|| -> std::io::Result<()> {
                if !crate::client_win::is_system() {
                    return Err(std::io::Error::other("Windows 没有开启「锁屏和权限弹窗也能控制」"));
                }
                let dir = crate::service_win::state_dir();
                let tmp = dir.join("update.tmp");
                let mut f = if offset == 0 { std::fs::File::create(&tmp)? } else { std::fs::OpenOptions::new().append(true).open(&tmp)? };
                f.write_all(&data)?;
                drop(f);
                if last {
                    std::fs::rename(&tmp, dir.join(crate::service_win::NEXT_EXE))?;
                }
                Ok(())
            })();
            match written {
                Err(e) => answer(queue, Bridge::Done { id, code: -1, data: format!("更新失败：{e}").into_bytes() }),
                Ok(()) if last => answer(queue, Bridge::Done { id, code: 0, data: vec![] }),
                Ok(()) => answer(queue, Bridge::Ack { id }),
            }
        }
        // pieces arrive in order on this thread; writing them here keeps that order
        Bridge::Put { id, offset, last, path, data } => {
            let written = (|| -> std::io::Result<()> {
                // opened with the signed-in user's rights, not SYSTEM's (RULES.md W7)
                let mut f = crate::service_win::as_user(|| {
                    if offset == 0 {
                        std::fs::File::create(&path)
                    } else {
                        std::fs::OpenOptions::new().append(true).open(&path)
                    }
                })??;
                f.write_all(&data)
            })();
            match written {
                Err(e) => answer(queue, Bridge::Done { id, code: -1, data: format!("写入 {path} 失败：{e}").into_bytes() }),
                Ok(()) if last => answer(queue, Bridge::Done { id, code: 0, data: vec![] }),
                // the Mac sends the next piece only after this (RULES.md X3)
                Ok(()) => answer(queue, Bridge::Ack { id }),
            }
        }
        Bridge::Get { id, path } => {
            let queue = queue.clone();
            let (tx, acks) = mpsc::channel();
            SENDING.lock().unwrap().get_or_insert_with(HashMap::new).insert(id, tx);
            small_thread(move || {
                let code = match send_file(&queue, id, &path, &acks) {
                    Ok(()) => (0, vec![]),
                    Err(e) => (-1, format!("读取 {path} 失败：{e}").into_bytes()),
                };
                SENDING.lock().unwrap().as_mut().map(|m| m.remove(&id));
                answer(&queue, Bridge::Done { id, code: code.0, data: code.1 });
            });
        }
        Bridge::Ack { id } => {
            if let Some(tx) = SENDING.lock().unwrap().as_ref().and_then(|m| m.get(&id)) {
                let _ = tx.send(());
            }
        }
        Bridge::Piece { .. } | Bridge::Done { .. } => {}
    }
    false
}

/// Sends a file a piece at a time, each after the Mac acknowledged the one before:
/// only one piece is ever in memory or in the queue (RULES.md X3).
fn send_file(queue: &Queue, id: u32, path: &str, acks: &mpsc::Receiver<()>) -> std::io::Result<()> {
    let mut file = crate::service_win::as_user(|| std::fs::File::open(path))??;
    let size = file.metadata()?.len();
    let mut buf = vec![0u8; bridge::CHUNK];
    let mut sent = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        sent += n as u64;
        let last = n == 0 || sent >= size;
        answer(queue, Bridge::Piece { id, last, data: buf[..n].to_vec() });
        acks.recv_timeout(Duration::from_secs(30)).map_err(|_| std::io::Error::other("Mac 没有确认"))?;
        if last {
            return Ok(());
        }
    }
}

/// The bridge's threads need little stack; a small one keeps the client in its 2 MB (R1).
fn small_thread(f: impl FnOnce() + Send + 'static) {
    let _ = std::thread::Builder::new().stack_size(64 * 1024).spawn(f);
}

/// A running bridge command: an ordinary child, or one WispService's SYSTEM client
/// started as the signed-in user (RULES.md W7).
enum Running {
    Own(std::process::Child),
    User(crate::service_win::UserProcess),
}

impl Running {
    /// Some(exit code) once it ended
    fn ended(&mut self) -> Option<i32> {
        match self {
            Running::Own(c) => match c.try_wait() {
                Ok(Some(status)) => Some(status.code().unwrap_or(-1)),
                Ok(None) => None,
                Err(_) => Some(-1),
            },
            Running::User(p) => crate::service_win::wait(p, 0),
        }
    }

    fn kill(&mut self) {
        match self {
            Running::Own(c) => {
                let _ = c.kill();
                let _ = c.wait();
            }
            Running::User(p) => {
                crate::service_win::kill(p);
                crate::service_win::wait(p, 2000);
            }
        }
    }
}

/// Starts `cmd` through cmd.exe, errors folded into the output. As SYSTEM it runs as
/// the signed-in user, and is refused when nobody is signed in.
fn start(cmd: &str) -> Result<(Running, Box<dyn Read + Send>), (i32, String)> {
    match crate::service_win::UserLaunch::get() {
        Err(e) => Err((-3, e.to_string())),
        Ok(Some(user)) => match crate::service_win::spawn_as_user(&user, cmd) {
            Ok((p, out)) => Ok((Running::User(p), Box::new(out))),
            Err(e) => Err((-1, format!("启动失败：{e}"))),
        },
        Ok(None) => {
            let child = Command::new("cmd")
                .raw_arg(format!("/C ({cmd}) 2>&1"))
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();
            match child {
                Ok(mut c) => {
                    let out = c.stdout.take().unwrap();
                    Ok((Running::Own(c), Box::new(out)))
                }
                Err(e) => Err((-1, format!("启动失败：{e}"))),
            }
        }
    }
}

/// Runs `cmd` through cmd.exe, as typed, errors folded into the output; at most
/// MAX_OUTPUT bytes come back; -2 if it ran past `timeout` seconds and was stopped.
fn run(cmd: &str, timeout: u32) -> (i32, Vec<u8>) {
    let (mut child, mut out) = match start(cmd) {
        Ok(started) => started,
        Err((code, e)) => return (code, e.into_bytes()),
    };
    let (tx, rx) = mpsc::channel();
    small_thread(move || {
        let mut buf = Vec::new();
        let _ = out.by_ref().take(MAX_OUTPUT as u64).read_to_end(&mut buf);
        // read on to the end so the command never blocks on a full pipe
        let _ = std::io::copy(&mut out, &mut std::io::sink());
        let _ = tx.send(buf);
    });
    let deadline = Instant::now() + Duration::from_secs(timeout.max(1) as u64);
    let code = loop {
        match child.ended() {
            Some(code) => break code,
            None if Instant::now() >= deadline => {
                child.kill();
                break -2;
            }
            None => std::thread::sleep(Duration::from_millis(30)),
        }
    };
    let mut bytes = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    if code == -2 {
        bytes.extend_from_slice(format!("\n（超过 {timeout} 秒，已停止）").as_bytes());
    }
    (code, to_utf8(bytes))
}

/// Console programs write in the OEM code page (GBK on a Chinese system), not UTF-8.
fn to_utf8(bytes: Vec<u8>) -> Vec<u8> {
    if std::str::from_utf8(&bytes).is_ok() {
        return bytes;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn MultiByteToWideChar(page: u32, flags: u32, s: *const u8, n: i32, w: *mut u16, wn: i32) -> i32;
    }
    const CP_OEMCP: u32 = 1;
    unsafe {
        let n = MultiByteToWideChar(CP_OEMCP, 0, bytes.as_ptr(), bytes.len() as i32, std::ptr::null_mut(), 0);
        if n <= 0 {
            return bytes;
        }
        let mut wide = vec![0u16; n as usize];
        MultiByteToWideChar(CP_OEMCP, 0, bytes.as_ptr(), bytes.len() as i32, wide.as_mut_ptr(), n);
        String::from_utf16_lossy(&wide).into_bytes()
    }
}
