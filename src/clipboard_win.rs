//! Windows clipboard: plain text in, plain text out (RULES.md CB1–CB4).

use crate::logic::clipboard::{from_crlf, to_crlf, ClipSync, Outgoing};
use crate::proto::{self, Msg};
use std::ffi::c_void;
use std::ptr;
use std::time::Duration;

#[link(name = "user32")]
extern "system" {
    fn OpenClipboard(owner: *mut c_void) -> i32;
    fn CloseClipboard() -> i32;
    fn EmptyClipboard() -> i32;
    fn GetClipboardData(format: u32) -> *mut c_void;
    fn SetClipboardData(format: u32, mem: *mut c_void) -> *mut c_void;
    fn IsClipboardFormatAvailable(format: u32) -> i32;
    fn RegisterClipboardFormatW(name: *const u16) -> u32;
    fn GetClipboardSequenceNumber() -> u32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GlobalAlloc(flags: u32, bytes: usize) -> *mut c_void;
    fn GlobalLock(mem: *mut c_void) -> *mut c_void;
    fn GlobalUnlock(mem: *mut c_void) -> i32;
    fn GlobalSize(mem: *mut c_void) -> usize;
    fn GlobalFree(mem: *mut c_void) -> *mut c_void;
}

const CF_UNICODETEXT: u32 = 13;
const GMEM_MOVEABLE: u32 = 0x0002;

/// Opens the clipboard, retrying briefly while another program holds it.
fn open() -> bool {
    for _ in 0..10 {
        if unsafe { OpenClipboard(ptr::null_mut()) } != 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

enum Content {
    /// nobody copied anything since the last look
    Unchanged,
    /// a password manager marked it secret (RULES.md CB3)
    Concealed,
    /// no plain text on the clipboard, or it stayed busy
    NoText,
    Text(String),
}

/// This machine's clipboard plus what the peer already has.
pub struct Clipboard {
    /// format password managers add to keep a copy out of clipboard history and sync
    exclude: u32,
    seen: Option<u32>,
    sync: ClipSync,
}

impl Clipboard {
    pub fn new() -> Self {
        let name: Vec<u16> = "ExcludeClipboardContentFromMonitorProcessing\0".encode_utf16().collect();
        let exclude = unsafe { RegisterClipboardFormatW(name.as_ptr()) };
        Clipboard { exclude, seen: None, sync: ClipSync::default() }
    }

    fn read(&mut self) -> Content {
        let seq = unsafe { GetClipboardSequenceNumber() };
        if self.seen == Some(seq) {
            return Content::Unchanged;
        }
        self.seen = Some(seq);
        unsafe {
            if self.exclude != 0 && IsClipboardFormatAvailable(self.exclude) != 0 {
                return Content::Concealed;
            }
            if IsClipboardFormatAvailable(CF_UNICODETEXT) == 0 || !open() {
                return Content::NoText;
            }
            let mem = GetClipboardData(CF_UNICODETEXT);
            let wide = if mem.is_null() { ptr::null() } else { GlobalLock(mem) as *const u16 };
            let text = if wide.is_null() {
                None
            } else {
                let units = std::slice::from_raw_parts(wide, GlobalSize(mem) / 2);
                let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
                let text = String::from_utf16_lossy(&units[..end]);
                GlobalUnlock(mem);
                Some(text)
            };
            CloseClipboard();
            text.map_or(Content::NoText, |t| Content::Text(from_crlf(&t)))
        }
    }

    fn write(&mut self, text: &str) -> bool {
        let wide: Vec<u16> = to_crlf(text).encode_utf16().chain(Some(0)).collect();
        if !open() {
            return false;
        }
        let ok = unsafe {
            EmptyClipboard();
            let mem = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
            let dst = if mem.is_null() { ptr::null_mut() } else { GlobalLock(mem) as *mut u16 };
            if dst.is_null() {
                false
            } else {
                ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
                GlobalUnlock(mem);
                // on success the clipboard owns the memory; on failure it is still ours
                let placed = !SetClipboardData(CF_UNICODETEXT, mem).is_null();
                if !placed {
                    GlobalFree(mem);
                }
                placed
            }
        };
        unsafe { CloseClipboard() };
        // our own write must not count as a new copy
        self.seen = Some(unsafe { GetClipboardSequenceNumber() });
        ok
    }

    /// The cursor just came back to the peer: send our clipboard if it is new.
    pub fn push(&mut self, peer: &proto::Queue) {
        match self.read() {
            Content::Text(text) => match self.sync.outgoing(&text) {
                Outgoing::Send => {
                    let _ = peer.try_send(Msg::Clipboard(text));
                }
                Outgoing::TooLarge(n) => log!("wisp: 剪贴板文字 {} KB 超过 1 MB 上限，这次不同步", n / 1024),
                Outgoing::Unchanged => {}
            },
            Content::Concealed => log!("wisp: 剪贴板是密码管理器标记的敏感内容，不同步"),
            Content::Unchanged | Content::NoText => {}
        }
    }

    /// The peer sent its clipboard: put it on ours.
    pub fn pull(&mut self, text: &str) {
        if self.write(text) {
            self.sync.incoming(text);
        } else {
            log!("wisp: 写入剪贴板失败");
        }
    }
}
