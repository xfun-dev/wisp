//! The developer bridge (RULES.md X1–X5): lets the paired Mac run commands and move
//! files on the Windows PC over the encrypted link, so tests and deploys need no other
//! remote tool. Built only with the `bridge` feature (`scripts/deploy.sh`), never into
//! a release; switched on and off from the Mac, open until switched off. This module
//! is the wire format, testable anywhere.

/// Files travel in small pieces, one at a time, each acknowledged before the next:
/// the core never holds more than one, and nothing piles up in a send queue, so the
/// bridge does not move either side's memory (RULES.md X3, R1). Measured: 768 KB
/// pieces left the Mac core at 5 MB after one 680 KB transfer; 16 KB pieces (just
/// over the 16 KB the Windows low-fragmentation heap serves) grew the Windows client
/// by ~260 KB per transfer and kept growing; 12 KB pieces: +56 KB once, then flat.
pub const CHUNK: usize = 12 * 1024;
/// how much of a command's output comes back; larger results go through a file
#[cfg(any(test, windows))]
pub const MAX_OUTPUT: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum Bridge {
    /// Mac -> PC: the bridge is open or closed
    State { open: bool },
    /// Mac -> PC: run `cmd` (through cmd.exe), at most `timeout` seconds
    Exec { id: u32, timeout: u32, cmd: String },
    /// Mac -> PC: a piece of a file; `offset` 0 starts (truncates) it, `last` finishes it
    Put { id: u32, offset: u64, last: bool, path: String, data: Vec<u8> },
    /// Mac -> PC: send this file back
    Get { id: u32, path: String },
    /// PC -> Mac: a piece of a requested file
    Piece { id: u32, last: bool, data: Vec<u8> },
    /// PC -> Mac: request `id` finished with `code` (0 ok) and its output or error text
    Done { id: u32, code: i32, data: Vec<u8> },
    /// either way: the last piece of `id` arrived, send the next
    Ack { id: u32 },
}

const STATE: u8 = 1;
const EXEC: u8 = 2;
const PUT: u8 = 3;
const GET: u8 = 4;
const PIECE: u8 = 5;
const DONE: u8 = 6;
const ACK: u8 = 7;

impl Bridge {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::new();
        let text = |b: &mut Vec<u8>, s: &str| {
            b.extend_from_slice(&(s.len() as u32).to_le_bytes());
            b.extend_from_slice(s.as_bytes());
        };
        match self {
            // eight bytes, as when this carried seconds left: any non-zero value is open,
            // and an older PC reads the largest one as its own longest time
            Bridge::State { open } => {
                b.push(STATE);
                b.extend_from_slice(&if *open { u64::MAX } else { 0 }.to_le_bytes());
            }
            Bridge::Exec { id, timeout, cmd } => {
                b.push(EXEC);
                b.extend_from_slice(&id.to_le_bytes());
                b.extend_from_slice(&timeout.to_le_bytes());
                text(&mut b, cmd);
            }
            Bridge::Put { id, offset, last, path, data } => {
                b.push(PUT);
                b.extend_from_slice(&id.to_le_bytes());
                b.extend_from_slice(&offset.to_le_bytes());
                b.push(*last as u8);
                text(&mut b, path);
                b.extend_from_slice(data);
            }
            Bridge::Get { id, path } => {
                b.push(GET);
                b.extend_from_slice(&id.to_le_bytes());
                text(&mut b, path);
            }
            Bridge::Piece { id, last, data } => {
                b.push(PIECE);
                b.extend_from_slice(&id.to_le_bytes());
                b.push(*last as u8);
                b.extend_from_slice(data);
            }
            Bridge::Done { id, code, data } => {
                b.push(DONE);
                b.extend_from_slice(&id.to_le_bytes());
                b.extend_from_slice(&code.to_le_bytes());
                b.extend_from_slice(data);
            }
            Bridge::Ack { id } => {
                b.push(ACK);
                b.extend_from_slice(&id.to_le_bytes());
            }
        }
        b
    }

    pub fn decode(b: &[u8]) -> Option<Bridge> {
        let mut at = 1usize;
        let mut take = |n: usize| -> Option<&[u8]> {
            let s = b.get(at..at + n)?;
            at += n;
            Some(s)
        };
        let u32_ = |s: &[u8]| u32::from_le_bytes(s.try_into().unwrap());
        let u64_ = |s: &[u8]| u64::from_le_bytes(s.try_into().unwrap());
        Some(match *b.first()? {
            STATE => Bridge::State { open: u64_(take(8)?) != 0 },
            EXEC => {
                let id = u32_(take(4)?);
                let timeout = u32_(take(4)?);
                let len = u32_(take(4)?) as usize;
                let cmd = String::from_utf8(take(len)?.to_vec()).ok()?;
                Bridge::Exec { id, timeout, cmd }
            }
            PUT => {
                let id = u32_(take(4)?);
                let offset = u64_(take(8)?);
                let last = take(1)?[0] != 0;
                let len = u32_(take(4)?) as usize;
                let path = String::from_utf8(take(len)?.to_vec()).ok()?;
                Bridge::Put { id, offset, last, path, data: b[at..].to_vec() }
            }
            GET => {
                let id = u32_(take(4)?);
                let len = u32_(take(4)?) as usize;
                Bridge::Get { id, path: String::from_utf8(take(len)?.to_vec()).ok()? }
            }
            PIECE => {
                let id = u32_(take(4)?);
                let last = take(1)?[0] != 0;
                Bridge::Piece { id, last, data: b[at..].to_vec() }
            }
            DONE => {
                let id = u32_(take(4)?);
                let code = i32::from_le_bytes(take(4)?.try_into().ok()?);
                Bridge::Done { id, code, data: b[at..].to_vec() }
            }
            ACK => Bridge::Ack { id: u32_(take(4)?) },
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bridge_message_round_trips() {
        let all = [
            Bridge::State { open: true },
            Bridge::State { open: false },
            Bridge::Exec { id: 7, timeout: 60, cmd: "powershell -Command \"Get-Process wisp\" 中文".into() },
            Bridge::Put { id: 8, offset: 786_432, last: true, path: r"C:\Users\a\Wisp\wisp.new.exe".into(), data: vec![0, 1, 255] },
            Bridge::Get { id: 9, path: r"C:\shot.png".into() },
            Bridge::Piece { id: 9, last: false, data: vec![9; 10] },
            Bridge::Done { id: 7, code: -2, data: "超时".as_bytes().to_vec() },
            Bridge::Ack { id: 8 },
        ];
        for m in all {
            assert_eq!(Bridge::decode(&m.encode()), Some(m.clone()), "{m:?}");
        }
        assert_eq!(Bridge::decode(&[]), None);
        assert_eq!(Bridge::decode(&[2, 1, 0]), None, "cut short");
        assert_eq!(Bridge::decode(&[99]), None, "unknown kind");
    }

    #[test]
    fn the_bridge_stays_as_the_mac_last_set_it() {
        // an older Mac sent seconds left: any of them still means open
        let mut old = vec![STATE];
        old.extend_from_slice(&28_800u64.to_le_bytes());
        assert_eq!(Bridge::decode(&old), Some(Bridge::State { open: true }));
        old[1..].fill(0);
        assert_eq!(Bridge::decode(&old), Some(Bridge::State { open: false }));
    }

    #[test]
    fn a_piece_fits_well_inside_a_frame() {
        let piece = Bridge::Put { id: 1, offset: 0, last: false, path: "C:\\a\\b".into(), data: vec![0; CHUNK] };
        assert!(piece.encode().len() + 16 < 16 * 1024, "every frame fits the Windows low-fragmentation heap");
        assert!(MAX_OUTPUT + 16 < crate::proto::MAX_CLIPBOARD);
    }
}
