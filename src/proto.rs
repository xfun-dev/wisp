//! Wire protocol: a tiny plaintext handshake that exchanges random salts,
//! then length-prefixed ChaCha20-Poly1305 frames with per-direction keys and
//! strictly increasing nonces (which also rejects replayed or reordered frames).
//! Fixed-size messages (all keyboard and mouse traffic) are built and opened in
//! place on the stack, so they never allocate; only clipboard text uses the heap.

use chacha20poly1305::aead::{AeadInPlace, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce, Tag};
use sha2::{Digest, Sha256};
use std::io::{self, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

pub const DEFAULT_PORT: u16 = 24801;
const MAGIC: &[u8; 4] = b"WISP";
/// 2: Key scancodes are by physical position (Command = Win key)
/// 3: 4-byte frame length, Clipboard message
/// 4: Name message (server -> client after Info)
const VERSION: u8 = 4;
const TAG_LEN: usize = 16;
const LEN_LEN: usize = 4;
/// longest fixed-size message (tag byte + two i32)
const MAX_SMALL: usize = 9;
/// largest clipboard text carried, in UTF-8 bytes
pub const MAX_CLIPBOARD: usize = 1 << 20;
const CLIPBOARD: u8 = 10;
const NAME: u8 = 11;
/// longest machine name sent, in UTF-8 bytes
#[cfg(any(test, target_os = "macos"))]
const MAX_NAME: usize = 128;
const MAX_FRAME: usize = 1 + MAX_CLIPBOARD + TAG_LEN;
/// Messages waiting for the writer. The queue is allocated once, up front: an
/// unbounded channel allocates a block every 31 messages on the sending thread and
/// frees it on the writer's, which measurably grew the Mac's footprint (~0.2-0.4 MB).
/// It only fills when the socket's own buffer is full too, i.e. the link is stuck.
const QUEUE_LEN: usize = 512;
pub type Queue = mpsc::SyncSender<Msg>;
/// idle interval after which the writer sends a Ping
const KEEPALIVE: Duration = Duration::from_secs(2);
/// a peer silent this long (three missed pings) is gone
const READ_TIMEOUT: Duration = Duration::from_secs(6);

#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    /// client -> server: client screen size in pixels
    Info { w: u16, h: u16 },
    /// keepalive, both directions
    Ping,
    /// server -> client: cursor now lives on the client
    Enter,
    /// server -> client: cursor went back; release everything held
    Leave,
    /// absolute cursor position in client pixels
    MouseAbs { x: i32, y: i32 },
    /// button: 1 left, 2 right, 3 middle, 4/5 extra
    Button { btn: u8, down: bool },
    /// wheel delta in Windows units (120 = one notch); positive y scrolls up, positive x scrolls right
    Wheel { dx: i32, dy: i32 },
    /// pinch zoom in whole steps; positive zooms in
    Zoom { steps: i32 },
    /// PC set-1 scancode by physical position, 0xE0xx for extended keys
    Key { scan: u16, down: bool },
    /// plain-text clipboard, both directions, at most MAX_CLIPBOARD bytes
    Clipboard(String),
    /// server -> client: the server's computer name, at most MAX_NAME bytes
    Name(String),
}

impl Msg {
    /// Writes a fixed-size message into `out` (at least MAX_SMALL bytes) and returns its length.
    fn encode_small(&self, out: &mut [u8]) -> usize {
        let mut n = 0;
        let mut put = |b: &[u8]| {
            out[n..n + b.len()].copy_from_slice(b);
            n += b.len();
        };
        match *self {
            Msg::Info { w, h } => {
                put(&[1]);
                put(&w.to_le_bytes());
                put(&h.to_le_bytes());
            }
            Msg::Ping => put(&[2]),
            Msg::Enter => put(&[3]),
            Msg::Leave => put(&[4]),
            Msg::MouseAbs { x, y } => {
                put(&[5]);
                put(&x.to_le_bytes());
                put(&y.to_le_bytes());
            }
            Msg::Button { btn, down } => put(&[6, btn, down as u8]),
            Msg::Wheel { dx, dy } => {
                put(&[7]);
                put(&dx.to_le_bytes());
                put(&dy.to_le_bytes());
            }
            Msg::Key { scan, down } => {
                put(&[8]);
                put(&scan.to_le_bytes());
                put(&[down as u8]);
            }
            Msg::Zoom { steps } => {
                put(&[9]);
                put(&steps.to_le_bytes());
            }
            Msg::Clipboard(_) | Msg::Name(_) => unreachable!("text messages are framed by Sender::send"),
        }
        n
    }

    /// The tag and text of a variable-length message; None for fixed-size ones.
    fn text(&self) -> Option<(u8, &str)> {
        match self {
            Msg::Clipboard(t) => Some((CLIPBOARD, t)),
            Msg::Name(t) => Some((NAME, t)),
            _ => None,
        }
    }

    fn from_text(tag: u8, text: String) -> Option<Msg> {
        match tag {
            CLIPBOARD => Some(Msg::Clipboard(text)),
            NAME => Some(Msg::Name(text)),
            _ => None,
        }
    }

    /// Decodes a message that fit a small frame: any fixed-size one, or text of a few bytes.
    fn decode_small(b: &[u8]) -> Option<Msg> {
        let u16_at = |i: usize| Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?));
        let i32_at = |i: usize| Some(i32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?));
        Some(match *b.first()? {
            1 => Msg::Info { w: u16_at(1)?, h: u16_at(3)? },
            2 => Msg::Ping,
            3 => Msg::Enter,
            4 => Msg::Leave,
            5 => Msg::MouseAbs { x: i32_at(1)?, y: i32_at(5)? },
            6 => Msg::Button { btn: *b.get(1)?, down: *b.get(2)? != 0 },
            7 => Msg::Wheel { dx: i32_at(1)?, dy: i32_at(5)? },
            8 => Msg::Key { scan: u16_at(1)?, down: *b.get(3)? != 0 },
            9 => Msg::Zoom { steps: i32_at(1)? },
            tag @ (CLIPBOARD | NAME) => Msg::from_text(tag, String::from_utf8(b[1..].to_vec()).ok()?)?,
            _ => return None,
        })
    }
}

/// Cuts a machine name to MAX_NAME bytes without splitting a character.
#[cfg(any(test, target_os = "macos"))]
pub fn short_name(name: &str) -> String {
    let mut end = name.len().min(MAX_NAME);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].trim().to_string()
}

/// A key from a password, for tests; real keys come from pairing (RULES.md P3).
#[cfg(test)]
pub fn psk(password: &str) -> [u8; 32] {
    Sha256::new_with_prefix(b"wisp-psk-v1").chain_update(password).finalize().into()
}

fn session_key(psk: &[u8; 32], dir: &[u8], client_salt: &[u8], server_salt: &[u8]) -> ChaCha20Poly1305 {
    let key = Sha256::new_with_prefix(psk)
        .chain_update(dir)
        .chain_update(client_salt)
        .chain_update(server_salt)
        .finalize();
    ChaCha20Poly1305::new(Key::from_slice(&key))
}

pub struct Sender {
    stream: TcpStream,
    cipher: ChaCha20Poly1305,
    counter: u64,
}

pub struct Receiver {
    // small buffer: a burst of mouse frames usually arrives in one TCP segment
    stream: BufReader<TcpStream>,
    cipher: ChaCha20Poly1305,
    counter: u64,
}

fn nonce(counter: u64) -> Nonce {
    let mut n = [0u8; 12];
    n[4..].copy_from_slice(&counter.to_le_bytes());
    Nonce::from(n)
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

impl Sender {
    fn send(&mut self, msg: &Msg) -> io::Result<()> {
        let mut small = [0u8; LEN_LEN + MAX_SMALL + TAG_LEN];
        let mut big;
        let frame: &mut [u8] = match msg.text() {
            Some((tag, text)) => {
                debug_assert!(text.len() <= MAX_CLIPBOARD, "oversized text must be filtered before sending");
                big = Vec::with_capacity(LEN_LEN + 1 + text.len() + TAG_LEN);
                big.extend_from_slice(&[0; LEN_LEN]);
                big.push(tag);
                big.extend_from_slice(text.as_bytes());
                big.extend_from_slice(&[0; TAG_LEN]);
                &mut big
            }
            None => {
                let n = msg.encode_small(&mut small[LEN_LEN..]);
                &mut small[..LEN_LEN + n + TAG_LEN]
            }
        };
        // frame layout: length, body (encrypted in place), tag
        let body_end = frame.len() - TAG_LEN;
        let tag = self
            .cipher
            .encrypt_in_place_detached(&nonce(self.counter), b"", &mut frame[LEN_LEN..body_end])
            .map_err(|_| bad("encrypt"))?;
        self.counter += 1;
        frame[body_end..].copy_from_slice(&tag);
        let len = (frame.len() - LEN_LEN) as u32;
        frame[..LEN_LEN].copy_from_slice(&len.to_be_bytes());
        self.stream.write_all(frame)
    }
}

impl Receiver {
    pub fn recv(&mut self) -> io::Result<Msg> {
        let mut len = [0u8; LEN_LEN];
        self.stream.read_exact(&mut len)?;
        let len = u32::from_be_bytes(len) as usize;
        if len > MAX_FRAME {
            return Err(bad("frame too large"));
        }
        let mut small = [0u8; MAX_SMALL + TAG_LEN];
        if len <= small.len() {
            let body = self.open(&mut small[..len])?;
            return Msg::decode_small(body).ok_or_else(|| bad("unknown message"));
        }
        // only text messages are larger; decode them in their own buffer, no copy
        let mut big = vec![0u8; len];
        let body_len = self.open(&mut big)?.len();
        let tag = big[0];
        big.truncate(body_len);
        big.drain(..1);
        let text = String::from_utf8(big).map_err(|_| bad("text is not UTF-8"))?;
        Msg::from_text(tag, text).ok_or_else(|| bad("unknown message"))
    }

    /// Reads a frame of `frame.len()` bytes into `frame`, authenticates and
    /// decrypts it in place, and returns the plaintext body.
    fn open<'a>(&mut self, frame: &'a mut [u8]) -> io::Result<&'a mut [u8]> {
        self.stream.read_exact(frame)?;
        let auth_failed = || bad("authentication failed (wrong password?)");
        if frame.len() < TAG_LEN {
            return Err(auth_failed());
        }
        let (body, tag) = frame.split_at_mut(frame.len() - TAG_LEN);
        self.cipher
            .decrypt_in_place_detached(&nonce(self.counter), b"", body, Tag::from_slice(tag))
            .map_err(|_| auth_failed())?;
        self.counter += 1;
        Ok(body)
    }

    /// Hands every message to `f` until the connection fails, then closes it
    /// (which also stops the writer thread) and returns why it ended.
    pub fn run(mut self, mut f: impl FnMut(Msg)) -> io::Error {
        let err = loop {
            match self.recv() {
                Ok(msg) => f(msg),
                Err(e) => break e,
            }
        };
        let _ = self.stream.get_ref().shutdown(Shutdown::Both);
        err
    }
}

/// Runs the salt exchange and returns the encrypted halves of the connection.
/// The peer is only proven to know the password once a frame from it decrypts.
pub fn handshake(stream: TcpStream, psk: &[u8; 32], is_server: bool) -> io::Result<(Sender, Receiver)> {
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    let mut hello = [0u8; 21];
    hello[..4].copy_from_slice(MAGIC);
    hello[4] = VERSION;
    getrandom::getrandom(&mut hello[5..]).map_err(|_| bad("no randomness"))?;
    (&stream).write_all(&hello)?;

    let mut theirs = [0u8; 21];
    (&stream).read_exact(&mut theirs)?;
    if &theirs[..4] != MAGIC {
        return Err(bad("not a wisp peer"));
    }
    if theirs[4] != VERSION {
        return Err(bad("protocol version mismatch"));
    }

    let (mine, theirs) = (&hello[5..], &theirs[5..]);
    let (cs, ss) = if is_server { (theirs, mine) } else { (mine, theirs) };
    let c2s = session_key(psk, b"c2s", cs, ss);
    let s2c = session_key(psk, b"s2c", cs, ss);
    let (tx, rx) = if is_server { (s2c, c2s) } else { (c2s, s2c) };

    let sender = Sender { stream: stream.try_clone()?, cipher: tx, counter: 0 };
    let receiver = Receiver { stream: BufReader::with_capacity(256, stream), cipher: rx, counter: 0 };
    Ok((sender, receiver))
}

impl Sender {
    /// Moves sending to a writer thread that forwards queued messages and pings
    /// when idle. The keepalive is tied to the socket, not to the queue: pings
    /// continue after every queue handle is dropped, until the connection closes.
    pub fn spawn(mut self) -> Queue {
        let (queue, pending) = mpsc::sync_channel(QUEUE_LEN);
        std::thread::spawn(move || {
            let mut queue_open = true;
            loop {
                let msg = if queue_open {
                    match pending.recv_timeout(KEEPALIVE) {
                        Ok(m) => m,
                        Err(RecvTimeoutError::Timeout) => Msg::Ping,
                        Err(RecvTimeoutError::Disconnected) => {
                            queue_open = false;
                            continue;
                        }
                    }
                } else {
                    std::thread::sleep(KEEPALIVE);
                    Msg::Ping
                };
                if self.send(&msg).is_err() {
                    return;
                }
            }
        });
        queue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn every_message_roundtrips() {
        let all = [
            Msg::Info { w: 2560, h: 1440 },
            Msg::Ping,
            Msg::Enter,
            Msg::Leave,
            Msg::MouseAbs { x: -1, y: i32::MAX },
            Msg::Button { btn: 5, down: true },
            Msg::Wheel { dx: -120, dy: 7 },
            Msg::Key { scan: 0xE05B, down: false },
            Msg::Zoom { steps: -3 },
        ];
        for m in all {
            let mut buf = [0u8; MAX_SMALL];
            let n = m.encode_small(&mut buf);
            assert_eq!(Msg::decode_small(&buf[..n]), Some(m));
        }
    }

    #[test]
    fn names_are_cut_on_a_character_boundary() {
        assert_eq!(short_name("  Fang's Mac  "), "Fang's Mac");
        let long = "界".repeat(100); // 300 bytes
        let cut = short_name(&long);
        assert_eq!(cut.len(), 126, "42 whole characters, not 128 bytes cut mid-character");
        assert!(cut.chars().all(|c| c == '界'));
    }

    #[test]
    fn roundtrip_over_tcp() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let key = psk("hunter2");
        let t = std::thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            let (tx, mut rx) = handshake(s, &key, true).unwrap();
            assert_eq!(rx.recv().unwrap(), Msg::Info { w: 2560, h: 1440 });
            let tx = tx.spawn();
            tx.send(Msg::Key { scan: 0xE04B, down: true }).unwrap();
            tx.send(Msg::Wheel { dx: -3, dy: 240 }).unwrap();
            tx.send(Msg::Zoom { steps: -2 }).unwrap();
            tx.send(Msg::Clipboard("复制的文字 ✓".into())).unwrap();
            tx.send(Msg::Clipboard("界".repeat(MAX_CLIPBOARD / 3))).unwrap();
            tx.send(Msg::Clipboard(String::new())).unwrap();
            tx.send(Msg::Name("小林的 MacBook Pro".into())).unwrap();
            tx.send(Msg::Name("Mac".into())).unwrap();
            tx.send(Msg::Ping).unwrap();
            // idle writer keeps the link alive
            assert_eq!(rx.recv().unwrap(), Msg::Ping);
        });
        let (tx, mut rx) = handshake(TcpStream::connect(addr).unwrap(), &key, false).unwrap();
        tx.spawn().send(Msg::Info { w: 2560, h: 1440 }).unwrap();
        assert_eq!(rx.recv().unwrap(), Msg::Key { scan: 0xE04B, down: true });
        assert_eq!(rx.recv().unwrap(), Msg::Wheel { dx: -3, dy: 240 });
        assert_eq!(rx.recv().unwrap(), Msg::Zoom { steps: -2 });
        assert_eq!(rx.recv().unwrap(), Msg::Clipboard("复制的文字 ✓".into()));
        assert_eq!(rx.recv().unwrap(), Msg::Clipboard("界".repeat(MAX_CLIPBOARD / 3)), "1 MB clipboard");
        assert_eq!(rx.recv().unwrap(), Msg::Clipboard(String::new()), "short text rides a small frame");
        assert_eq!(rx.recv().unwrap(), Msg::Name("小林的 MacBook Pro".into()));
        assert_eq!(rx.recv().unwrap(), Msg::Name("Mac".into()), "a short name rides a small frame");
        assert_eq!(rx.recv().unwrap(), Msg::Ping, "small frames still work after a large one");
        t.join().unwrap();
    }

    #[test]
    fn a_stuck_link_fills_the_queue_instead_of_blocking_the_sender() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let key = psk("hunter2");
        // the peer handshakes, then never reads again
        let t = std::thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            let link = handshake(s, &key, true).unwrap();
            std::thread::sleep(Duration::from_secs(3));
            drop(link);
        });
        let (tx, _rx) = handshake(TcpStream::connect(addr).unwrap(), &key, false).unwrap();
        let queue = tx.spawn();
        let started = std::time::Instant::now();
        let mut sent = 0;
        let full = loop {
            match queue.try_send(Msg::MouseAbs { x: sent, y: 0 }) {
                Ok(()) => sent += 1,
                Err(e) => break e,
            }
            assert!(sent < 10_000_000, "never filled");
        };
        assert!(matches!(full, mpsc::TrySendError::Full(_)), "stuck link reports Full, not Disconnected");
        assert!(started.elapsed() < Duration::from_secs(2), "filling never waited on the network");
        t.join().unwrap();
    }

    #[test]
    fn wrong_password_is_rejected() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let t = std::thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            let (_, mut rx) = handshake(s, &psk("right"), true).unwrap();
            assert!(rx.recv().is_err());
        });
        let (mut tx, _rx) = handshake(TcpStream::connect(addr).unwrap(), &psk("wrong"), false).unwrap();
        tx.send(&Msg::Ping).unwrap();
        t.join().unwrap();
    }

    #[test]
    fn oversized_frames_are_rejected_before_reading() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let key = psk("hunter2");
        let t = std::thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            let (_, mut rx) = handshake(s, &key, true).unwrap();
            let err = rx.recv().unwrap_err();
            assert_eq!(err.to_string(), "frame too large");
        });
        let (mut tx, _rx) = handshake(TcpStream::connect(addr).unwrap(), &key, false).unwrap();
        // only the length prefix is sent: the receiver must refuse without waiting for a body
        tx.stream.write_all(&((MAX_FRAME + 1) as u32).to_be_bytes()).unwrap();
        t.join().unwrap();
    }
}
