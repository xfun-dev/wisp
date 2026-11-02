//! The pairing exchange (RULES.md P2-P4). SPAKE2 turns the 6-digit code into a
//! shared secret without letting an eavesdropper test guesses offline; both sides
//! then prove they derived the same secret, bound to both machine names, before
//! anything is saved. The secret itself never travels.
//!
//! Over TCP, client (Windows) first:
//!   C: "WSPP" ver name spake_b      S: status [name spake_a]
//!   C: confirm_client               S: status [confirm_server | tries_left]

use crate::logic::pairing::Paired;
#[cfg(any(test, target_os = "macos"))]
use crate::logic::pairing::Window;
use sha2::{Digest, Sha256};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use std::io::{self, Read, Write};
use std::net::TcpStream;
#[cfg(any(test, target_os = "macos"))]
use std::sync::Mutex;
use std::time::Duration;

pub const MAGIC: &[u8; 4] = b"WSPP";
const VERSION: u8 = 1;
const SPAKE_LEN: usize = 33;
const OK: u8 = 0;
#[cfg(any(test, target_os = "macos"))]
const CLOSED: u8 = 1;
const WRONG: u8 = 2;

fn spake_ids() -> (Identity, Identity) {
    (Identity::new(b"wisp-mac"), Identity::new(b"wisp-pc"))
}

/// One of the three values derived from the SPAKE2 secret, bound to both names.
fn derive(secret: &[u8], label: &[u8], mac: &str, pc: &str) -> [u8; 32] {
    Sha256::new_with_prefix(b"wisp-pair-v1")
        .chain_update(label)
        .chain_update((mac.len() as u32).to_be_bytes())
        .chain_update(mac)
        .chain_update(pc)
        .chain_update(secret)
        .finalize()
        .into()
}

fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn write_name(out: &mut Vec<u8>, name: &str) {
    let bytes = &name.as_bytes()[..name.len().min(255)];
    out.push(bytes.len() as u8);
    out.extend_from_slice(bytes);
}

fn read_name(stream: &mut TcpStream) -> io::Result<String> {
    let mut len = [0u8; 1];
    stream.read_exact(&mut len)?;
    let mut name = vec![0u8; len[0] as usize];
    stream.read_exact(&mut name)?;
    Ok(String::from_utf8_lossy(&name).into_owned())
}

fn read_array<const N: usize>(stream: &mut TcpStream) -> io::Result<[u8; N]> {
    let mut a = [0u8; N];
    stream.read_exact(&mut a)?;
    Ok(a)
}

fn failed(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, msg)
}

/// Mac side: one attempt on a connection that starts with MAGIC. Uses the code in
/// `window` (counting a wrong one, closing it when used or out of tries) and returns
/// the new pairing on success. `now` is unix seconds.
#[cfg(any(test, target_os = "macos"))]
pub fn host(mut stream: TcpStream, window: &Mutex<Option<Window>>, mac: &str, now: u64) -> io::Result<Paired> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let head = read_array::<5>(&mut stream)?;
    if &head[..4] != MAGIC || head[4] != VERSION {
        return Err(failed("not a pairing request"));
    }
    let pc = read_name(&mut stream)?;
    let msg_b = read_array::<SPAKE_LEN>(&mut stream)?;

    let code = window.lock().unwrap().as_ref().and_then(|w| w.code(now).map(str::to_string));
    let Some(code) = code else {
        stream.write_all(&[CLOSED])?;
        return Err(failed("no open pairing code"));
    };
    let (a, b) = spake_ids();
    let (spake, msg_a) = Spake2::<Ed25519Group>::start_a(&Password::new(code.as_bytes()), &a, &b);
    let mut reply = vec![OK];
    write_name(&mut reply, mac);
    reply.extend_from_slice(&msg_a);
    stream.write_all(&reply)?;
    let secret = spake.finish(&msg_b).map_err(|_| failed("bad pairing message"))?;

    let confirm = read_array::<32>(&mut stream)?;
    if !same(&confirm, &derive(&secret, b"confirm-client", mac, &pc)) {
        let mut w = window.lock().unwrap();
        let still_open = w.as_mut().is_some_and(Window::failed);
        let tries_left = if still_open { w.as_ref().map_or(0, Window::tries_left) } else { 0 };
        if !still_open {
            *w = None;
        }
        stream.write_all(&[WRONG, tries_left])?;
        return Err(failed("wrong pairing code"));
    }
    *window.lock().unwrap() = None;
    let mut done = vec![OK];
    done.extend_from_slice(&derive(&secret, b"confirm-server", mac, &pc));
    stream.write_all(&done)?;
    Ok(Paired { name: pc.clone(), key: derive(&secret, b"key", mac, &pc) })
}

/// Windows side: pairs with the Mac on `stream` using `code`; returns the pairing to save.
/// Errors carry the words to show the user.
#[cfg(any(test, windows))]
pub fn join(mut stream: TcpStream, code: &str, pc: &str) -> io::Result<Paired> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let (a, b) = spake_ids();
    let (spake, msg_b) = Spake2::<Ed25519Group>::start_b(&Password::new(code.as_bytes()), &a, &b);
    let mut hello = MAGIC.to_vec();
    hello.push(VERSION);
    write_name(&mut hello, pc);
    hello.extend_from_slice(&msg_b);
    stream.write_all(&hello)?;

    if read_array::<1>(&mut stream)?[0] != OK {
        return Err(failed("Mac 上的配对码已失效，请在 Mac 上重新生成"));
    }
    let mac = read_name(&mut stream)?;
    let msg_a = read_array::<SPAKE_LEN>(&mut stream)?;
    let secret = spake.finish(&msg_a).map_err(|_| failed("配对消息有误，请重试"))?;
    stream.write_all(&derive(&secret, b"confirm-client", &mac, pc))?;

    match read_array::<1>(&mut stream)?[0] {
        OK => {
            let confirm = read_array::<32>(&mut stream)?;
            if !same(&confirm, &derive(&secret, b"confirm-server", &mac, pc)) {
                return Err(failed("对方没能证明自己知道配对码，已停止配对"));
            }
            Ok(Paired { name: mac.clone(), key: derive(&secret, b"key", &mac, pc) })
        }
        WRONG => Err(failed(&match read_array::<1>(&mut stream)?[0] {
            0 => "配对码不对，已失效，请在 Mac 上重新生成".to_string(),
            n => format!("配对码不对，还可以再试 {n} 次"),
        })),
        _ => Err(failed("Mac 上的配对码已失效，请在 Mac 上重新生成")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::Arc;

    /// A Mac with `window` open; returns its address and what each attempt produced.
    fn mac(window: Arc<Mutex<Option<Window>>>, attempts: usize) -> (std::net::SocketAddr, std::thread::JoinHandle<Vec<io::Result<Paired>>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let t = std::thread::spawn(move || {
            (0..attempts).map(|_| host(l.accept().unwrap().0, &window, "MacBook", 1000)).collect()
        });
        (addr, t)
    }

    fn pc(addr: std::net::SocketAddr, code: &str) -> io::Result<Paired> {
        join(TcpStream::connect(addr).unwrap(), code, "Office PC")
    }

    #[test]
    fn the_right_code_gives_both_sides_the_same_key() {
        let window = Arc::new(Mutex::new(Some(Window::open("482917".into(), 1000))));
        let (addr, t) = mac(window.clone(), 1);
        let on_pc = pc(addr, "482917").unwrap();
        let on_mac = t.join().unwrap().remove(0).unwrap();
        assert_eq!(on_pc.key, on_mac.key);
        assert_eq!((on_pc.name.as_str(), on_mac.name.as_str()), ("MacBook", "Office PC"));
        assert!(window.lock().unwrap().is_none(), "a used code is gone");
    }

    #[test]
    fn three_wrong_codes_void_the_window() {
        let window = Arc::new(Mutex::new(Some(Window::open("482917".into(), 1000))));
        let (addr, t) = mac(window.clone(), 4);
        assert_eq!(pc(addr, "000000").unwrap_err().to_string(), "配对码不对，还可以再试 2 次");
        assert_eq!(pc(addr, "111111").unwrap_err().to_string(), "配对码不对，还可以再试 1 次");
        assert_eq!(pc(addr, "222222").unwrap_err().to_string(), "配对码不对，已失效，请在 Mac 上重新生成");
        assert_eq!(pc(addr, "482917").unwrap_err().to_string(), "Mac 上的配对码已失效，请在 Mac 上重新生成", "even the right code is too late");
        assert!(t.join().unwrap().iter().all(Result::is_err));
        assert!(window.lock().unwrap().is_none());
    }

    #[test]
    fn no_open_code_means_no_pairing() {
        let (addr, t) = mac(Arc::new(Mutex::new(None)), 1);
        assert!(pc(addr, "482917").is_err());
        assert!(t.join().unwrap().remove(0).is_err());

        let expired = Arc::new(Mutex::new(Some(Window::open("482917".into(), 0))));
        let (addr, t) = mac(expired, 1);
        assert!(pc(addr, "482917").is_err(), "opened at 0, tried at 1000: past LIFETIME");
        assert!(t.join().unwrap().remove(0).is_err());
    }
}
