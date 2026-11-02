//! Pairing rules (RULES.md P1-P6): a 6-digit code that is good for two minutes and
//! three wrong guesses, and the file each side keeps once paired.

#[cfg(any(test, target_os = "macos"))]
/// seconds a code stays valid (user decision, 2026-11-01)
pub const LIFETIME: u64 = 120;
#[cfg(any(test, target_os = "macos"))]
/// wrong codes before it is void (user decision, 2026-11-01)
pub const TRIES: u8 = 3;

#[cfg(any(test, target_os = "macos"))]
/// Six digits from a random number, as typed: "042917".
pub fn code_from(random: u32) -> String {
    format!("{:06}", random % 1_000_000)
}

/// What the user typed, reduced to its digits if there are exactly six.
#[cfg(any(test, windows))]
pub fn typed(input: &str) -> Option<String> {
    let digits: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    (digits.len() == 6 && digits.chars().all(|c| c.is_ascii_digit())).then_some(digits)
}

#[cfg(any(test, target_os = "macos"))]
/// The Mac's open invitation: the code, until it expires or is guessed wrong too often.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    code: String,
    expires: u64,
    tries_left: u8,
}

#[cfg(any(test, target_os = "macos"))]
impl Window {
    pub fn open(code: String, now: u64) -> Window {
        Window { code, expires: now + LIFETIME, tries_left: TRIES }
    }

    /// The code while it can still be used.
    pub fn code(&self, now: u64) -> Option<&str> {
        (now < self.expires && self.tries_left > 0).then_some(self.code.as_str())
    }

    pub fn seconds_left(&self, now: u64) -> u64 {
        self.expires.saturating_sub(now)
    }

    pub fn tries_left(&self) -> u8 {
        self.tries_left
    }

    /// Counts a wrong code; returns whether the window is still open.
    pub fn failed(&mut self) -> bool {
        self.tries_left = self.tries_left.saturating_sub(1);
        self.tries_left > 0
    }
}

/// Who this machine is paired with, and the key they share.
#[derive(Debug, Clone, PartialEq)]
pub struct Paired {
    pub name: String,
    pub key: [u8; 32],
}

impl Paired {
    pub fn to_text(&self) -> String {
        let hex: String = self.key.iter().map(|b| format!("{b:02x}")).collect();
        format!("name={}\nkey={hex}\n", self.name.replace('\n', " "))
    }

    /// None unless the file holds a whole 32-byte key.
    pub fn parse(text: &str) -> Option<Paired> {
        let mut name = String::new();
        let mut key = None;
        for line in text.lines() {
            match line.split_once('=') {
                Some(("name", v)) => name = v.to_string(),
                Some(("key", v)) if v.len() == 64 => {
                    let mut k = [0u8; 32];
                    for (i, byte) in k.iter_mut().enumerate() {
                        *byte = u8::from_str_radix(&v[2 * i..2 * i + 2], 16).ok()?;
                    }
                    key = Some(k);
                }
                _ => {}
            }
        }
        Some(Paired { name, key: key? })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_six_digits_and_read_back_from_typing() {
        assert_eq!(code_from(42_917), "042917");
        assert_eq!(code_from(u32::MAX), "967295");
        assert_eq!(typed(" 482 917 "), Some("482917".into()));
        assert_eq!(typed("48291"), None);
        assert_eq!(typed("48a917"), None);
    }

    #[test]
    fn a_code_lasts_two_minutes_and_three_wrong_guesses() {
        let mut w = Window::open("482917".into(), 1000);
        assert_eq!(w.code(1000), Some("482917"));
        assert_eq!(w.code(1119), Some("482917"));
        assert_eq!(w.code(1120), None, "expired after LIFETIME");
        assert_eq!(w.seconds_left(1100), 20);

        assert!(w.failed());
        assert!(w.failed());
        assert!(!w.failed(), "the third wrong code closes it");
        assert_eq!(w.code(1001), None);
        assert!(!w.failed(), "stays closed");
    }

    #[test]
    fn pair_file_round_trips_and_rejects_broken_keys() {
        let p = Paired { name: "Alex’s MacBook Pro".into(), key: [7; 32] };
        assert_eq!(Paired::parse(&p.to_text()), Some(p));
        assert_eq!(Paired::parse("name=Fang\nkey=abcd\n"), None, "short key");
        assert_eq!(Paired::parse(&format!("name=Fang\nkey={}\n", "zz".repeat(32))), None, "not hex");
        assert_eq!(Paired::parse(""), None);
    }
}
