//! A click on something drawn: pressed down on it, let go on the same thing. Windows
//! sends WM_CAPTURECHANGED from inside ReleaseCapture, before the button-up handler goes
//! on, so the press must be taken before the capture is released (a bug in the first
//! version of the drawn status window: every click was dropped).

#[derive(Debug)]
pub struct Press<T> {
    down: Option<T>,
}

impl<T> Default for Press<T> {
    fn default() -> Self {
        Press { down: None }
    }
}

impl<T: Copy + PartialEq> Press<T> {
    pub fn down(&mut self, on: Option<T>) {
        self.down = on;
    }

    pub fn held(&self) -> Option<T> {
        self.down
    }

    /// The pointer was taken away mid-press (Alt+Tab, a dialog): no click. Returns
    /// whether something was held, so the caller knows to redraw.
    pub fn lost(&mut self) -> bool {
        self.down.take().is_some()
    }

    /// Button up over `on`; `release` lets go of the mouse capture, and may call
    /// `lost` on the way, as Windows does. Returns what was clicked.
    pub fn up(&mut self, on: Option<T>, release: impl FnOnce(&mut Self)) -> Option<T> {
        let held = self.down.take();
        release(self);
        held.filter(|&h| Some(h) == on)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_survives_the_capture_change_sent_while_releasing() {
        let mut p = Press::default();
        p.down(Some('q'));
        assert_eq!(p.up(Some('q'), |p| {
            p.lost();
        }), Some('q'), "WM_CAPTURECHANGED arrives inside ReleaseCapture");
    }

    #[test]
    fn letting_go_elsewhere_or_losing_the_pointer_is_no_click() {
        let mut p = Press::default();
        p.down(Some('q'));
        assert_eq!(p.up(Some('r'), |_| {}), None, "dragged off onto another thing");
        p.down(Some('q'));
        assert!(p.lost());
        assert_eq!(p.up(Some('q'), |_| {}), None, "taken away before the button came up");
        p.down(None);
        assert_eq!(p.up(None, |_| {}), None, "pressed on nothing");
    }
}
