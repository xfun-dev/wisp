//! When clipboard text crosses to the other machine (RULES.md CB1–CB4): only new
//! text, only within the size limit, and never straight back to where it came from.

use crate::proto::MAX_CLIPBOARD;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Debug, PartialEq)]
pub enum Outgoing {
    /// send it
    Send,
    /// the peer already has this text
    Unchanged,
    /// over the limit by this many bytes; skipped this time
    TooLarge(usize),
}

/// Remembers the last text both machines agree on, by hash, so neither side
/// resends it or bounces back what it just received.
#[derive(Default)]
pub struct ClipSync {
    shared: Option<u64>,
}

fn digest(text: &str) -> u64 {
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

impl ClipSync {
    /// Decides whether this machine's current clipboard `text` goes to the peer.
    pub fn outgoing(&mut self, text: &str) -> Outgoing {
        let d = digest(text);
        if self.shared == Some(d) {
            return Outgoing::Unchanged;
        }
        if text.len() > MAX_CLIPBOARD {
            return Outgoing::TooLarge(text.len());
        }
        self.shared = Some(d);
        Outgoing::Send
    }

    /// Records text received from the peer (and placed on this clipboard).
    pub fn incoming(&mut self, text: &str) {
        self.shared = Some(digest(text));
    }
}

/// Line breaks travel as `\n`. Windows stores `\r\n`; these convert at its edge.
#[cfg(any(test, windows))]
pub fn from_crlf(text: &str) -> String {
    text.replace("\r\n", "\n")
}

#[cfg(any(test, windows))]
pub fn to_crlf(text: &str) -> String {
    from_crlf(text).replace('\n', "\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_sends_only_new_text_within_the_limit() {
        let mut sync = ClipSync::default();
        assert_eq!(sync.outgoing("hello"), Outgoing::Send);
        assert_eq!(sync.outgoing("hello"), Outgoing::Unchanged, "same text twice");
        assert_eq!(sync.outgoing("world"), Outgoing::Send);
        assert_eq!(sync.outgoing("hello"), Outgoing::Send, "copying the old text again is new to the peer");

        let huge = "x".repeat(MAX_CLIPBOARD + 1);
        assert_eq!(sync.outgoing(&huge), Outgoing::TooLarge(MAX_CLIPBOARD + 1));
        assert_eq!(sync.outgoing(&huge), Outgoing::TooLarge(MAX_CLIPBOARD + 1), "still skipped, still reported");
        assert_eq!(sync.outgoing(&"x".repeat(MAX_CLIPBOARD)), Outgoing::Send, "exactly at the limit");
    }

    #[test]
    fn clipboard_never_echoes_what_it_received() {
        let mut mac = ClipSync::default();
        let mut win = ClipSync::default();

        // copied on the Mac, cursor goes to Windows
        assert_eq!(mac.outgoing("from mac"), Outgoing::Send);
        win.incoming("from mac");
        // cursor comes back without a new copy on Windows: nothing to bring back
        assert_eq!(win.outgoing("from mac"), Outgoing::Unchanged);

        // copied on Windows, cursor returns to the Mac
        assert_eq!(win.outgoing("from win"), Outgoing::Send);
        mac.incoming("from win");
        // next hop to Windows: the Mac's clipboard holds what Windows already has
        assert_eq!(mac.outgoing("from win"), Outgoing::Unchanged);
    }

    #[test]
    fn line_breaks_travel_as_lf_and_land_as_crlf_on_windows() {
        assert_eq!(from_crlf("a\r\nb\r\n"), "a\nb\n");
        assert_eq!(to_crlf("a\nb\n"), "a\r\nb\r\n");
        assert_eq!(to_crlf("a\r\nb"), "a\r\nb", "already CRLF is not doubled");
        assert_eq!(from_crlf(&to_crlf("x\ny")), "x\ny", "round trip is lossless");
    }
}
