//! Why the last link to the PC ended, shown by the settings window while it searches
//! again (RULES.md U13). Decided from how the link ended and from two clocks: the
//! wall clock keeps going while the Mac sleeps, `Instant` (CLOCK_UPTIME_RAW on macOS)
//! does not, so a link that spans much more wall time than awake time slept away.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drop {
    /// the Mac went to sleep under the link
    Slept,
    /// the PC stopped answering: off, asleep, hung, or the network went away
    Silent,
    /// the PC closed the link: Wisp quit or restarted there
    Closed,
}

/// More wall time than awake time, in seconds, that means the Mac slept. Ordinary
/// clock adjustments are far smaller; the shortest real sleep is longer.
const SLEPT_GAP: u64 = 20;

impl Drop {
    /// `timed_out`: the link went quiet past its read timeout. `wall`, `awake`: seconds
    /// the link lasted on each clock.
    pub fn classify(timed_out: bool, wall: u64, awake: u64) -> Drop {
        if wall > awake + SLEPT_GAP {
            Drop::Slept
        } else if timed_out {
            Drop::Silent
        } else {
            Drop::Closed
        }
    }

    /// the word in the status line (`drop=<unix>:<word>`)
    pub fn word(self) -> &'static str {
        match self {
            Drop::Slept => "slept",
            Drop::Silent => "silent",
            Drop::Closed => "closed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drop_says_whether_the_mac_slept_the_pc_went_quiet_or_closed() {
        assert_eq!(Drop::classify(true, 3600, 600), Drop::Slept, "an hour on the wall, ten minutes awake");
        assert_eq!(Drop::classify(false, 900, 300), Drop::Slept, "however it ended, sleep came first");
        assert_eq!(Drop::classify(true, 600, 595), Drop::Silent, "small clock drift is not sleep");
        assert_eq!(Drop::classify(false, 600, 600), Drop::Closed);
        assert_eq!(Drop::Silent.word(), "silent");
    }
}
