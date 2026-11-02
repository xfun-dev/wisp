//! macOS clipboard (RULES.md CB1-CB6). Touching the pasteboard costs a process about
//! 1 MB for good (measured: the C Pasteboard Manager warms up and keeps it), so the
//! core never does: each read or write runs in a short-lived `wisp pasteboard`
//! helper that exits right after. The core keeps only the sync decisions (ClipSync)
//! on a worker thread, off the event tap.

use crate::logic::clipboard::{ClipSync, Outgoing};
use crate::proto::{self, Msg, MAX_CLIPBOARD};
use std::ffi::c_void;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::ptr;
use std::sync::mpsc;

type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFArrayRef = *const c_void;
type CFDataRef = *const c_void;
type PasteboardRef = *mut c_void;
type PasteboardItemID = *mut c_void;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn PasteboardCreate(name: CFStringRef, out: *mut PasteboardRef) -> i32;
    fn PasteboardSynchronize(pb: PasteboardRef) -> u32;
    fn PasteboardGetItemCount(pb: PasteboardRef, out: *mut usize) -> i32;
    fn PasteboardGetItemIdentifier(pb: PasteboardRef, index: isize, out: *mut PasteboardItemID) -> i32;
    fn PasteboardCopyItemFlavors(pb: PasteboardRef, item: PasteboardItemID, out: *mut CFArrayRef) -> i32;
    fn PasteboardCopyItemFlavorData(pb: PasteboardRef, item: PasteboardItemID, flavor: CFStringRef, out: *mut CFDataRef) -> i32;
    fn PasteboardClear(pb: PasteboardRef) -> i32;
    fn PasteboardPutItemFlavor(pb: PasteboardRef, item: PasteboardItemID, flavor: CFStringRef, data: CFDataRef, flags: u32) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFStringCreateWithCString(alloc: CFTypeRef, s: *const u8, encoding: u32) -> CFStringRef;
    fn CFArrayGetCount(array: CFArrayRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CFArrayRef, index: isize) -> CFTypeRef;
    fn CFEqual(a: CFTypeRef, b: CFTypeRef) -> u8;
    fn CFDataCreate(alloc: CFTypeRef, bytes: *const u8, len: isize) -> CFDataRef;
    fn CFDataGetLength(data: CFDataRef) -> isize;
    fn CFDataGetBytePtr(data: CFDataRef) -> *const u8;
    fn CFRelease(cf: CFTypeRef);
}

const UTF8: u32 = 0x0800_0100;

fn cfstr(s: &[u8]) -> CFStringRef {
    unsafe { CFStringCreateWithCString(ptr::null(), s.as_ptr(), UTF8) }
}

/// What the helper found, and how it tells the core on stdout.
#[derive(Debug, PartialEq)]
enum Content {
    Text(String),
    /// a password manager marked it secret (RULES.md CB3)
    Concealed,
    /// over the limit by this many bytes; not even copied out (RULES.md CB2)
    TooLarge(usize),
    /// no plain text on the clipboard
    NoText,
}

impl Content {
    fn encode(&self) -> Vec<u8> {
        match self {
            Content::Text(t) => [b"text\n", t.as_bytes()].concat(),
            Content::Concealed => b"concealed\n".to_vec(),
            Content::TooLarge(n) => format!("toolarge {n}\n").into_bytes(),
            Content::NoText => b"none\n".to_vec(),
        }
    }

    fn decode(out: &[u8]) -> Content {
        let (first, rest) = match out.iter().position(|&b| b == b'\n') {
            Some(i) => (&out[..i], &out[i + 1..]),
            None => (out, &[][..]),
        };
        match std::str::from_utf8(first).unwrap_or_default().split_once(' ') {
            None if first == b"text" => String::from_utf8(rest.to_vec()).map_or(Content::NoText, Content::Text),
            None if first == b"concealed" => Content::Concealed,
            Some(("toolarge", n)) => n.parse().map_or(Content::NoText, Content::TooLarge),
            _ => Content::NoText,
        }
    }
}

struct Pasteboard {
    pb: PasteboardRef,
    plain_text: CFStringRef,
    concealed: CFStringRef,
}

impl Pasteboard {
    fn open() -> Option<Self> {
        let mut pb = ptr::null_mut();
        let name = cfstr(b"com.apple.pasteboard.clipboard\0");
        let status = unsafe { PasteboardCreate(name, &mut pb) };
        unsafe { CFRelease(name) };
        (status == 0 && !pb.is_null()).then(|| Pasteboard {
            pb,
            plain_text: cfstr(b"public.utf8-plain-text\0"),
            concealed: cfstr(b"org.nspasteboard.ConcealedType\0"),
        })
    }

    fn read(&mut self) -> Content {
        unsafe {
            PasteboardSynchronize(self.pb);
            let mut count = 0;
            let mut item = ptr::null_mut();
            if PasteboardGetItemCount(self.pb, &mut count) != 0
                || count == 0
                || PasteboardGetItemIdentifier(self.pb, 1, &mut item) != 0
            {
                return Content::NoText;
            }
            let mut flavors = ptr::null();
            if PasteboardCopyItemFlavors(self.pb, item, &mut flavors) == 0 {
                let concealed = (0..CFArrayGetCount(flavors))
                    .any(|i| CFEqual(CFArrayGetValueAtIndex(flavors, i), self.concealed) != 0);
                CFRelease(flavors);
                if concealed {
                    return Content::Concealed;
                }
            }
            let mut data = ptr::null();
            if PasteboardCopyItemFlavorData(self.pb, item, self.plain_text, &mut data) != 0 {
                return Content::NoText;
            }
            let len = CFDataGetLength(data) as usize;
            let content = if len > MAX_CLIPBOARD {
                Content::TooLarge(len)
            } else {
                let bytes = std::slice::from_raw_parts(CFDataGetBytePtr(data), len);
                String::from_utf8(bytes.to_vec()).map_or(Content::NoText, Content::Text)
            };
            CFRelease(data);
            content
        }
    }

    fn write(&mut self, text: &str) {
        unsafe {
            PasteboardClear(self.pb);
            let data = CFDataCreate(ptr::null(), text.as_ptr(), text.len() as isize);
            PasteboardPutItemFlavor(self.pb, 1 as PasteboardItemID, self.plain_text, data, 0);
            CFRelease(data);
            PasteboardSynchronize(self.pb);
        }
    }
}

/// `wisp pasteboard read|write`, run by the core: reads the clipboard to stdout,
/// or writes stdin to it. Returns the exit code.
pub fn helper(args: &[String]) -> i32 {
    let Some(mut board) = Pasteboard::open() else { return 1 };
    match args.first().map(String::as_str) {
        Some("read") => {
            let _ = std::io::stdout().write_all(&board.read().encode());
            0
        }
        Some("write") => {
            let mut text = String::new();
            if std::io::stdin().take(MAX_CLIPBOARD as u64 + 1).read_to_string(&mut text).is_err() {
                return 1;
            }
            board.write(&text);
            0
        }
        _ => 2,
    }
}

/// Runs the helper once; its stdout if it succeeded.
fn run_helper(action: &str, input: Option<&str>) -> Option<Vec<u8>> {
    let exe = std::env::current_exe().ok()?;
    let mut child = Command::new(exe)
        .args(["pasteboard", action])
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin.write_all(text.as_bytes()).ok()?;
    }
    let out = child.wait_with_output().ok()?;
    out.status.success().then_some(out.stdout)
}

pub enum Job {
    /// the cursor just entered the peer behind this queue: send our clipboard if it is new
    Push(proto::Queue),
    /// the peer sent its clipboard: put it on ours
    Pull(String),
}

/// Starts the clipboard worker; returns the queue that feeds it.
pub fn spawn() -> mpsc::Sender<Job> {
    let (jobs, pending) = mpsc::channel();
    std::thread::spawn(move || {
        let mut sync = ClipSync::default();
        for job in pending {
            match job {
                Job::Push(peer) => match run_helper("read", None).map(|out| Content::decode(&out)) {
                    Some(Content::Text(text)) => match sync.outgoing(&text) {
                        Outgoing::Send => {
                            let _ = peer.try_send(Msg::Clipboard(text));
                        }
                        Outgoing::TooLarge(n) => log!("wisp: 剪贴板文字 {} KB 超过 1 MB 上限，这次不同步", n / 1024),
                        Outgoing::Unchanged => {}
                    },
                    Some(Content::TooLarge(n)) => log!("wisp: 剪贴板文字 {} KB 超过 1 MB 上限，这次不同步", n / 1024),
                    Some(Content::Concealed) => log!("wisp: 剪贴板是密码管理器标记的敏感内容，不同步"),
                    Some(Content::NoText) => {}
                    None => log!("wisp: 读取剪贴板失败"),
                },
                Job::Pull(text) => match run_helper("write", Some(&text)) {
                    Some(_) => sync.incoming(&text),
                    None => log!("wisp: 写入剪贴板失败"),
                },
            }
        }
    });
    jobs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_output_round_trips() {
        for c in [
            Content::Text("第一行\n第二行\n".into()),
            Content::Text(String::new()),
            Content::Concealed,
            Content::TooLarge(5_000_000),
            Content::NoText,
        ] {
            assert_eq!(Content::decode(&c.encode()), c);
        }
        assert_eq!(Content::decode(b""), Content::NoText, "a helper that printed nothing");
        assert_eq!(Content::decode(b"garbage\n"), Content::NoText);
        assert_eq!(Content::decode(b"text\n\xff\xfe"), Content::NoText, "not UTF-8");
    }
}
