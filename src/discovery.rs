//! Finding the server on the LAN. The client broadcasts an authenticated query and
//! the server answers with its TCP port. The client only ever sends outbound, so
//! the Windows firewall needs no inbound rule: it lets unicast replies to a
//! broadcast through by default.

use sha2::{Digest, Sha256};
use std::net::UdpSocket;
use std::time::Duration;
#[cfg(any(test, windows))]
use std::{io, net::SocketAddr, time::Instant};

/// Packets. The client broadcasts a query carrying a fresh nonce;
/// the server answers with its TCP port. Each side tags its packet with a key
/// derived from the password, so strangers neither get an answer nor can forge one.
const QUERY_MAGIC: &[u8; 4] = b"WSPQ";
const ANSWER_MAGIC: &[u8; 4] = b"WSPA";
const DISCOVERY_VERSION: u8 = 1;
/// magic, version, nonce, tag
const QUERY_LEN: usize = 4 + 1 + 16 + 16;
/// magic, version, port, tag
const ANSWER_LEN: usize = 4 + 1 + 2 + 16;

/// Truncated SHA-256 over the password key and fixed-length fields; truncation
/// rules out length extension, the fixed layout rules out ambiguity.
fn tag(psk: &[u8; 32], label: &[u8], nonce: &[u8], extra: &[u8]) -> [u8; 16] {
    let h = Sha256::new_with_prefix(psk)
        .chain_update(label)
        .chain_update(nonce)
        .chain_update(extra)
        .finalize();
    h[..16].try_into().unwrap()
}

/// Compares without an early exit, so timing does not reveal how much matched.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(any(test, windows))]
/// A query proving knowledge of the password, plus the nonce its answer must echo.
fn query(psk: &[u8; 32]) -> io::Result<([u8; QUERY_LEN], [u8; 16])> {
    let mut nonce = [0u8; 16];
    getrandom::getrandom(&mut nonce).map_err(|_| io::Error::other("no randomness"))?;
    let mut q = [0u8; QUERY_LEN];
    q[..4].copy_from_slice(QUERY_MAGIC);
    q[4] = DISCOVERY_VERSION;
    q[5..21].copy_from_slice(&nonce);
    q[21..].copy_from_slice(&tag(psk, b"query", &nonce, &[]));
    Ok((q, nonce))
}

#[cfg(any(test, target_os = "macos"))]
/// The answer to a genuine query, advertising TCP `port`; None for anything else.
fn answer(psk: &[u8; 32], query: &[u8], port: u16) -> Option<[u8; ANSWER_LEN]> {
    if query.len() != QUERY_LEN || &query[..4] != QUERY_MAGIC || query[4] != DISCOVERY_VERSION {
        return None;
    }
    let nonce = &query[5..21];
    if !same(&query[21..], &tag(psk, b"query", nonce, &[])) {
        return None;
    }
    let mut a = [0u8; ANSWER_LEN];
    a[..4].copy_from_slice(ANSWER_MAGIC);
    a[4] = DISCOVERY_VERSION;
    let port = port.to_be_bytes();
    a[5..7].copy_from_slice(&port);
    a[7..].copy_from_slice(&tag(psk, b"answer", nonce, &port));
    Some(a)
}

#[cfg(any(test, windows))]
/// The TCP port from a genuine answer to the query that used `nonce`.
fn answered_port(psk: &[u8; 32], nonce: &[u8; 16], answer: &[u8]) -> Option<u16> {
    if answer.len() != ANSWER_LEN || &answer[..4] != ANSWER_MAGIC || answer[4] != DISCOVERY_VERSION {
        return None;
    }
    let port = &answer[5..7];
    same(&answer[7..], &tag(psk, b"answer", nonce, port)).then(|| u16::from_be_bytes([port[0], port[1]]))
}

/// Asking which Mac is waiting to pair (RULES.md P2). Anyone may ask: the answer
/// carries only the Mac's name and port, the code still has to be right.
const PAIR_QUERY: &[u8; 4] = b"WSPD";
const PAIR_ANSWER: &[u8; 4] = b"WSPE";

#[cfg(any(test, target_os = "macos"))]
/// What the Mac says to a datagram: a genuine query is answered while paired
/// (`psk`), a pairing query while a code is open (`pairing` = this Mac's name).
pub fn respond(query: &[u8], psk: Option<&[u8; 32]>, pairing: Option<&str>, port: u16) -> Option<Vec<u8>> {
    if query.len() == 5 && &query[..4] == PAIR_QUERY && query[4] == DISCOVERY_VERSION {
        let name = pairing?.as_bytes();
        let name = &name[..name.len().min(255)];
        let mut a = PAIR_ANSWER.to_vec();
        a.push(DISCOVERY_VERSION);
        a.extend_from_slice(&port.to_be_bytes());
        a.extend_from_slice(name);
        return Some(a);
    }
    answer(psk?, query, port).map(|a| a.to_vec())
}

#[cfg(any(test, windows))]
fn pairing_answered(answer: &[u8]) -> Option<(u16, String)> {
    if answer.len() < 7 || &answer[..4] != PAIR_ANSWER || answer[4] != DISCOVERY_VERSION {
        return None;
    }
    Some((u16::from_be_bytes([answer[5], answer[6]]), String::from_utf8_lossy(&answer[7..]).into_owned()))
}

/// Answers datagrams on `socket` with whatever `respond` says, forever.
#[cfg(any(test, target_os = "macos"))]
pub fn serve(socket: UdpSocket, respond: impl Fn(&[u8]) -> Option<Vec<u8>>) {
    let mut buf = [0u8; QUERY_LEN + 1];
    loop {
        match socket.recv_from(&mut buf) {
            Ok((n, from)) => {
                if let Some(answer) = respond(&buf[..n]) {
                    let _ = socket.send_to(&answer, from);
                }
            }
            // e.g. ICMP port-unreachable bounced back from an earlier reply; don't spin
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// Sends `query` to each of `targets` and returns the first answer `accept` takes
/// within `wait`, with the address it came from.
#[cfg(any(test, windows))]
fn ask<T>(query: &[u8], targets: &[SocketAddr], wait: Duration, missing: &str, accept: impl Fn(&[u8]) -> Option<T>) -> io::Result<(std::net::IpAddr, T)> {
    let socket = UdpSocket::bind(("0.0.0.0", 0))?;
    socket.set_broadcast(true)?;
    for to in targets {
        // one unreachable target (e.g. no route for a broadcast) must not stop the others
        let _ = socket.send_to(query, to);
    }
    let deadline = Instant::now() + wait;
    let mut buf = [0u8; 512];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, missing.to_string()));
        }
        socket.set_read_timeout(Some(left))?;
        match socket.recv_from(&mut buf) {
            Ok((n, from)) => {
                if let Some(found) = accept(&buf[..n]) {
                    return Ok((from.ip(), found));
                }
            }
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
            Err(e) => return Err(e),
        }
    }
}

/// The paired Mac that answers a genuine query within `wait`, as the address to connect to.
#[cfg(any(test, windows))]
pub fn find(psk: &[u8; 32], targets: &[SocketAddr], wait: Duration) -> io::Result<SocketAddr> {
    let (q, nonce) = query(psk)?;
    let (ip, port) = ask(&q, targets, wait, "局域网里没有找到 Mac", |a| answered_port(psk, &nonce, a))?;
    Ok(SocketAddr::new(ip, port))
}

/// A Mac waiting to pair, with its name.
#[cfg(any(test, windows))]
pub fn find_pairing(targets: &[SocketAddr], wait: Duration) -> io::Result<(SocketAddr, String)> {
    let mut q = PAIR_QUERY.to_vec();
    q.push(DISCOVERY_VERSION);
    let missing = "局域网里没有正在等待配对的 Mac：请先在 Mac 的 Wisp 里点“配对新电脑”";
    let (ip, (port, name)) = ask(&q, targets, wait, missing, pairing_answered)?;
    Ok((SocketAddr::new(ip, port), name))
}

/// Where to broadcast queries: the limited broadcast, plus the /24 broadcast of
/// every private LAN address in `ips`. Directed broadcasts follow the LAN route,
/// so they still reach the LAN when a VPN or proxy TUN adapter owns the default
/// route and swallows 255.255.255.255.
#[cfg(any(test, windows))]
fn lan_broadcasts(ips: impl IntoIterator<Item = std::net::IpAddr>, port: u16) -> Vec<SocketAddr> {
    use std::net::{IpAddr, Ipv4Addr};
    let mut targets = vec![SocketAddr::from((Ipv4Addr::BROADCAST, port))];
    for ip in ips {
        let IpAddr::V4(ip) = ip else { continue };
        if !ip.is_private() {
            continue;
        }
        let [a, b, c, _] = ip.octets();
        let target = SocketAddr::from((Ipv4Addr::new(a, b, c, 255), port));
        if !targets.contains(&target) {
            targets.push(target);
        }
    }
    targets
}

/// Broadcast targets for this machine's connected adapters.
#[cfg(windows)]
pub fn broadcast_targets(port: u16) -> Vec<SocketAddr> {
    use std::net::ToSocketAddrs;
    // resolving our own computer name lists the addresses of every connected adapter
    let name = std::env::var("COMPUTERNAME").unwrap_or_default();
    let ips = (name.as_str(), 0).to_socket_addrs().into_iter().flatten().map(|a| a.ip());
    lan_broadcasts(ips, port)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto;

    fn server(psk: [u8; 32]) -> SocketAddr {
        mac(Some(psk), None)
    }

    fn mac(psk: Option<[u8; 32]>, pairing: Option<&'static str>) -> SocketAddr {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = socket.local_addr().unwrap();
        std::thread::spawn(move || serve(socket, |q| respond(q, psk.as_ref(), pairing, 24801)));
        addr
    }

    #[test]
    fn a_mac_waiting_to_pair_says_its_name_and_only_then() {
        let addr = mac(None, Some("Alex’s MacBook Pro"));
        let (found, name) = find_pairing(&[addr], Duration::from_secs(1)).unwrap();
        assert_eq!((found, name.as_str()), (SocketAddr::from(([127, 0, 0, 1], 24801)), "Alex’s MacBook Pro"));

        let addr = mac(Some(proto::psk("k")), None);
        assert!(find_pairing(&[addr], Duration::from_millis(300)).is_err(), "no code open: silent");
        let addr = mac(None, Some("Mac"));
        assert!(find(&proto::psk("k"), &[addr], Duration::from_millis(300)).is_err(), "unpaired: no answer to a keyed query");
    }

    #[test]
    fn finds_the_server_and_its_tcp_port() {
        let key = proto::psk("hunter2");
        let addr = server(key);
        let found = find(&key, &[addr], Duration::from_secs(1)).unwrap();
        assert_eq!(found, SocketAddr::from(([127, 0, 0, 1], 24801)));
    }

    #[test]
    fn a_server_with_another_password_stays_silent() {
        let addr = server(proto::psk("right"));
        let err = find(&proto::psk("wrong"), &[addr], Duration::from_millis(300)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn discovery_answers_only_the_password_holder() {
        let key = proto::psk("hunter2");
        let (q, nonce) = query(&key).unwrap();
        let a = answer(&key, &q, 24801).expect("genuine query is answered");
        assert_eq!(answered_port(&key, &nonce, &a), Some(24801));

        assert_eq!(answer(&proto::psk("other"), &q, 24801), None, "stranger's key");
        let mut forged = q;
        forged[10] ^= 1;
        assert_eq!(answer(&key, &forged, 24801), None, "tampered nonce");
        assert_eq!(answer(&key, &q[..20], 24801), None, "truncated");
    }

    #[test]
    fn discovery_rejects_forged_or_replayed_answers() {
        let key = proto::psk("hunter2");
        let (q, nonce) = query(&key).unwrap();
        let mut a = answer(&key, &q, 24801).unwrap();
        let (_, other_nonce) = query(&key).unwrap();
        assert_eq!(answered_port(&key, &other_nonce, &a), None, "answer to an older query");
        assert_eq!(answered_port(&proto::psk("other"), &nonce, &a), None, "answer from a stranger");
        a[6] ^= 1;
        assert_eq!(answered_port(&key, &nonce, &a), None, "port rewritten in transit");
    }

    #[test]
    fn broadcasts_reach_every_private_lan_but_skip_tunnels_and_link_local() {
        let ips = ["192.168.1.12", "198.18.0.1", "169.254.69.129", "10.0.0.5", "192.168.1.40", "8.8.8.8"];
        let targets = lan_broadcasts(ips.iter().map(|ip| ip.parse().unwrap()), 24801);
        let expected: Vec<SocketAddr> =
            ["255.255.255.255:24801", "192.168.1.255:24801", "10.0.0.255:24801"].iter().map(|a| a.parse().unwrap()).collect();
        assert_eq!(targets, expected);
    }
}
