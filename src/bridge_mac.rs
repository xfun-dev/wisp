//! Mac side of the developer bridge (RULES.md X1–X5), built only with the `bridge`
//! feature. The control port (`wisp win …`, the settings window's hidden switch) opens
//! or closes it and sends requests; the connected PC runs them and answers over the
//! same encrypted link. It stays open until switched off: the data directory's `bridge`
//! file says `on`, so a restart of the core or a new link keeps it as it was.

use crate::logic::bridge::{self, Bridge};
use crate::proto::Msg;
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::Duration;

type Send = Box<dyn Fn(Msg) -> bool + std::marker::Send + Sync>;

/// Hands a message to the connected PC; false when there is none.
static SEND: OnceLock<Send> = OnceLock::new();
static PENDING: Mutex<Option<HashMap<u32, mpsc::Sender<Bridge>>>> = Mutex::new(None);
static NEXT: AtomicU32 = AtomicU32::new(1);

fn file() -> PathBuf {
    crate::data_dir().join("bridge")
}

pub fn set_sender(send: Send) {
    let _ = SEND.set(send);
}

fn send(msg: Bridge) -> bool {
    SEND.get().is_some_and(|f| f(Msg::Bridge(msg.encode())))
}

/// Whether the bridge is open.
pub fn is_open() -> bool {
    std::fs::read_to_string(file()).is_ok_and(|t| t.trim() == "on")
}

/// Opens or closes the bridge until told otherwise, and tells the PC.
pub fn set(on: bool) -> io::Result<()> {
    std::fs::write(file(), if on { "on" } else { "off" })?;
    log!("wisp: 开发者通道{}", if on { "已开启" } else { "已关闭" });
    send(Bridge::State { open: on });
    Ok(())
}

/// A PC just connected: tell it whether the bridge is open.
pub fn on_connect() {
    send(Bridge::State { open: is_open() });
}

/// The link dropped: every request still waiting fails at once.
pub fn on_disconnect() {
    PENDING.lock().unwrap().take();
}

/// An answer from the PC.
pub fn on_message(bytes: &[u8]) {
    let Some(msg) = Bridge::decode(bytes) else { return };
    let id = match &msg {
        Bridge::Piece { id, .. } | Bridge::Done { id, .. } | Bridge::Ack { id } => *id,
        _ => return,
    };
    let mut pending = PENDING.lock().unwrap();
    let Some(map) = pending.as_mut() else { return };
    let done = matches!(msg, Bridge::Done { .. });
    if let Some(tx) = map.get(&id) {
        let _ = tx.send(msg);
    }
    if done {
        map.remove(&id);
    }
}

/// A request in flight: its id and where its answers arrive.
struct Request {
    id: u32,
    rx: mpsc::Receiver<Bridge>,
}

impl Request {
    fn new() -> io::Result<Request> {
        if !is_open() {
            return Err(io::Error::other("开发者通道没有开启（在 Wisp 设置里点两下花盆、一下茶杯）"));
        }
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        PENDING.lock().unwrap().get_or_insert_with(HashMap::new).insert(id, tx);
        Ok(Request { id, rx })
    }

    fn send(&self, msg: Bridge) -> io::Result<()> {
        if send(msg) { Ok(()) } else { Err(io::Error::other("Windows 没有连接")) }
    }

    fn next(&self, wait: Duration) -> io::Result<Bridge> {
        self.rx.recv_timeout(wait).map_err(|e| match e {
            mpsc::RecvTimeoutError::Timeout => io::Error::new(io::ErrorKind::TimedOut, "Windows 没有在时限内回答"),
            mpsc::RecvTimeoutError::Disconnected => io::Error::other("与 Windows 的连接断开了"),
        })
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        if let Some(map) = PENDING.lock().unwrap().as_mut() {
            map.remove(&self.id);
        }
    }
}

const STEP: Duration = Duration::from_secs(30);

fn failed(code: i32, data: &[u8]) -> io::Result<()> {
    match code {
        0 => Ok(()),
        _ => Err(io::Error::other(String::from_utf8_lossy(data).into_owned())),
    }
}

/// Runs `cmd` on the PC through cmd.exe; returns its exit code and (the start of) its output.
pub fn exec(cmd: &str, timeout: u32) -> io::Result<(i32, Vec<u8>)> {
    let r = Request::new()?;
    r.send(Bridge::Exec { id: r.id, timeout, cmd: cmd.to_string() })?;
    let wait = Duration::from_secs(timeout as u64 + 15);
    loop {
        if let Bridge::Done { code, data, .. } = r.next(wait)? {
            return Ok((code, data));
        }
    }
}

/// Copies the local file `from` to `to` on the PC, a piece at a time.
pub fn put(from: &str, to: &str) -> io::Result<()> {
    use std::io::Read;
    let r = Request::new()?;
    let mut file = std::fs::File::open(from)?;
    let size = file.metadata()?.len();
    let mut buf = vec![0u8; bridge::CHUNK];
    let mut offset = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        let last = offset + n as u64 >= size || n == 0;
        r.send(Bridge::Put { id: r.id, offset, last, path: to.to_string(), data: buf[..n].to_vec() })?;
        offset += n as u64;
        // wait for the PC before sending more: only one piece is ever on its way
        match r.next(STEP)? {
            Bridge::Ack { .. } if !last => {}
            Bridge::Done { code, data, .. } => return failed(code, &data),
            _ => return Err(io::Error::other("Windows 的回答不对")),
        }
    }
}

/// Copies `from` on the PC to the local file `to`, writing each piece as it comes.
pub fn get(from: &str, to: &str) -> io::Result<()> {
    use std::io::Write;
    let r = Request::new()?;
    r.send(Bridge::Get { id: r.id, path: from.to_string() })?;
    let mut file = None;
    loop {
        match r.next(STEP)? {
            Bridge::Piece { data, .. } => {
                if file.is_none() {
                    file = Some(std::fs::File::create(to)?);
                }
                file.as_mut().unwrap().write_all(&data)?;
                r.send(Bridge::Ack { id: r.id })?;
            }
            Bridge::Done { code, data, .. } => {
                if code != 0 {
                    drop(file.take());
                    let _ = std::fs::remove_file(to);
                    return failed(code, &data);
                }
                if file.is_none() {
                    std::fs::File::create(to)?;
                }
                return Ok(());
            }
            _ => {}
        }
    }
}
