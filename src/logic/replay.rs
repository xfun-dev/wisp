//! Receiving side: what is held down, and how incoming keys and positions map
//! onto this machine.

pub const LEFT_CTRL: u16 = 0x1D;
pub const RIGHT_CTRL: u16 = 0xE01D;
pub const LEFT_WIN: u16 = 0xE05B;
pub const RIGHT_WIN: u16 = 0xE05C;

/// Keys arrive by physical position; swap Ctrl and Win so the Mac's Command
/// key acts as Ctrl (Cmd+C / Cmd+V) and Control reaches the Windows key.
pub fn mac_habit(scan: u16) -> u16 {
    match scan {
        LEFT_WIN => LEFT_CTRL,
        RIGHT_WIN => RIGHT_CTRL,
        LEFT_CTRL => LEFT_WIN,
        RIGHT_CTRL => RIGHT_WIN,
        other => other,
    }
}

/// Maps a pixel on a screen `size` pixels long to the absolute 0..=65535 range.
pub fn to_absolute(v: i32, size: i32) -> i32 {
    let last = (size - 1).max(1);
    (v.clamp(0, last) as i64 * 65535 / last as i64) as i32
}

/// Everything currently held down, so it can be released when the cursor leaves
/// or the connection drops and nothing stays stuck.
#[derive(Default)]
pub struct Held {
    keys: Vec<u16>,
    buttons: [bool; 6],
}

impl Held {
    pub fn key(&mut self, scan: u16, down: bool) {
        if !down {
            self.keys.retain(|&k| k != scan);
        } else if !self.keys.contains(&scan) {
            self.keys.push(scan);
        }
    }

    /// Records buttons 1..=5; anything else is ignored.
    pub fn button(&mut self, btn: u8, down: bool) {
        if (1..=5).contains(&btn) {
            self.buttons[btn as usize] = down;
        }
    }

    pub fn has_key(&self, scan: u16) -> bool {
        self.keys.contains(&scan)
    }

    /// Forgets everything held, reporting keys most recent first, then buttons.
    pub fn release_all(&mut self, mut key_up: impl FnMut(u16), mut button_up: impl FnMut(u8)) {
        for scan in std::mem::take(&mut self.keys).into_iter().rev() {
            key_up(scan);
        }
        for btn in 1..=5u8 {
            if std::mem::take(&mut self.buttons[btn as usize]) {
                button_up(btn);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_becomes_ctrl_and_back() {
        assert_eq!(mac_habit(LEFT_WIN), LEFT_CTRL);
        assert_eq!(mac_habit(RIGHT_CTRL), RIGHT_WIN);
        assert_eq!(mac_habit(0x2E), 0x2E, "C is untouched");
        for s in [LEFT_CTRL, RIGHT_CTRL, LEFT_WIN, RIGHT_WIN] {
            assert_eq!(mac_habit(mac_habit(s)), s);
        }
    }

    #[test]
    fn absolute_covers_the_full_range() {
        assert_eq!(to_absolute(0, 2560), 0);
        assert_eq!(to_absolute(2559, 2560), 65535);
        assert_eq!(to_absolute(9999, 2560), 65535);
        assert_eq!(to_absolute(-5, 2560), 0);
        assert_eq!(to_absolute(0, 1), 0, "degenerate screen does not divide by zero");
    }

    #[test]
    fn release_all_lets_go_of_everything_once() {
        let mut held = Held::default();
        held.key(LEFT_CTRL, true);
        held.key(0x2E, true);
        held.key(0x2E, true); // auto-repeat
        held.key(0x1E, true);
        held.key(0x1E, false);
        held.button(1, true);
        held.button(3, true);
        held.button(3, false);
        held.button(9, true); // unknown button
        assert!(held.has_key(LEFT_CTRL));

        let (mut keys, mut buttons) = (vec![], vec![]);
        held.release_all(|k| keys.push(k), |b| buttons.push(b));
        assert_eq!(keys, [0x2E, LEFT_CTRL]);
        assert_eq!(buttons, [1]);

        held.release_all(|_| panic!("released twice"), |_| panic!("released twice"));
        assert!(!held.has_key(LEFT_CTRL));
    }
}
