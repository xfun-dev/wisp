//! Sending side: which screen owns the cursor, where it lands when it hops, and
//! how trackpad scrolling and pinching become wheel and zoom steps.

pub use crate::logic::settings::Edge;

/// A screen in the sending side's global coordinates (y grows downward).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// What the platform layer has to do after a mouse move.
#[derive(Debug, PartialEq)]
pub enum Hop {
    /// the cursor stays on this screen
    Local,
    /// the cursor crossed onto the remote screen at (x, y) remote pixels
    Enter { x: i32, y: i32 },
    /// the cursor moved to (x, y) on the remote screen
    Move { x: i32, y: i32 },
    /// the cursor is still on the same remote pixel
    Unmoved,
    /// the cursor came back; place the local cursor at (x, y)
    Leave { x: f64, y: f64 },
}

struct OnRemote {
    /// the edge the cursor came in through, which is also the way back
    edge: Edge,
    w: f64,
    h: f64,
    x: f64,
    y: f64,
    last: (i32, i32),
}

/// Tracks which screen owns the cursor.
#[derive(Default)]
pub struct Switcher {
    remote: Option<OnRemote>,
}

fn pixel(x: f64, y: f64) -> (i32, i32) {
    (x.round() as i32, y.round() as i32)
}

impl Switcher {
    /// The local cursor at `p` moved by `d`. Hops when it is pushed past `edge` of
    /// `local` while a remote screen of size `remote` is connected, landing on the
    /// remote's opposite edge at the same fraction along it. Both closures run only
    /// for a push toward that edge, which keeps the common case free of display
    /// lookups and locking.
    pub fn local_move(
        &mut self,
        edge: Edge,
        p: (f64, f64),
        d: (f64, f64),
        local: impl FnOnce() -> Rect,
        remote: impl FnOnce() -> Option<(f64, f64)>,
    ) -> Hop {
        let toward = match edge {
            Edge::Top => d.1 < 0.0,
            Edge::Left => d.0 < 0.0,
            Edge::Right => d.0 > 0.0,
        };
        if self.remote.is_some() || !toward {
            return Hop::Local;
        }
        let b = local();
        let (px, py) = p;
        let along_x = px >= b.x && px < b.x + b.w;
        let along_y = py >= b.y && py < b.y + b.h;
        let at_edge = match edge {
            Edge::Top => along_x && py <= b.y + 0.5,
            Edge::Left => along_y && px <= b.x + 0.5,
            Edge::Right => along_y && px >= b.x + b.w - 1.5,
        };
        if !at_edge {
            return Hop::Local;
        }
        let Some((w, h)) = remote() else { return Hop::Local };
        let (fx, fy) = ((px - b.x) / b.w, (py - b.y) / b.h);
        let (x, y) = match edge {
            Edge::Top => (fx * (w - 1.0), h - 1.0),
            Edge::Left => (w - 1.0, fy * (h - 1.0)),
            Edge::Right => (0.0, fy * (h - 1.0)),
        };
        let last = pixel(x, y);
        self.remote = Some(OnRemote { edge, w, h, x, y, last });
        Hop::Enter { x: last.0, y: last.1 }
    }

    /// Raw motion `d` while on the remote screen, scaled by `speed`. Pushing back out
    /// through the edge it came in by returns to this screen beside the same spot,
    /// but only with no button held, so a drag never strands a button on either side.
    pub fn remote_move(&mut self, speed: f64, d: (f64, f64), buttons_held: bool, local: impl FnOnce() -> Rect) -> Hop {
        let Some(r) = self.remote.as_mut() else { return Hop::Local };
        let (dx, dy) = (d.0 * speed, d.1 * speed);
        let leaving = !buttons_held
            && match r.edge {
                Edge::Top => dy > 0.0 && r.y + dy > r.h - 1.0,
                Edge::Left => dx > 0.0 && r.x + dx > r.w - 1.0,
                Edge::Right => dx < 0.0 && r.x + dx < 0.0,
            };
        if leaving {
            let (fx, fy, edge) = (r.x / r.w, r.y / r.h, r.edge);
            self.remote = None;
            let b = local();
            let (x, y) = match edge {
                Edge::Top => (b.x + fx * b.w, b.y + 2.0),
                Edge::Left => (b.x + 2.0, b.y + fy * b.h),
                Edge::Right => (b.x + b.w - 3.0, b.y + fy * b.h),
            };
            return Hop::Leave { x, y };
        }
        r.x = (r.x + dx).clamp(0.0, r.w - 1.0);
        r.y = (r.y + dy).clamp(0.0, r.h - 1.0);
        let pos = pixel(r.x, r.y);
        if pos == r.last {
            return Hop::Unmoved;
        }
        r.last = pos;
        Hop::Move { x: pos.0, y: pos.1 }
    }

    /// Takes the cursor back without placing it (link lost, send failed).
    pub fn reset(&mut self) {
        self.remote = None;
    }
}

/// Pinch magnification that adds up to one zoom step.
pub const ZOOM_PER_STEP: f64 = 0.15;

/// Gathers pinch magnification into whole zoom steps.
#[derive(Default)]
pub struct Pinch {
    gathered: f64,
}

impl Pinch {
    /// Adds a magnification delta and returns the whole steps it completes (positive zooms in).
    pub fn add(&mut self, magnification: f64) -> i32 {
        self.gathered += magnification;
        let steps = (self.gathered / ZOOM_PER_STEP).trunc();
        self.gathered -= steps * ZOOM_PER_STEP;
        steps as i32
    }

    /// Drops the remainder when the gesture ends, so the next pinch starts fresh.
    pub fn end(&mut self) {
        self.gathered = 0.0;
    }
}

/// Converts a macOS scroll event to Windows wheel units (120 per notch) as (x, y).
/// Trackpads report pixel `points`, scaled by `k`; mouse wheels report `lines`.
/// macOS positive x moves content right (scrolls left) while Windows positive
/// hwheel scrolls right, so x flips. None when nothing would move.
pub fn wheel(continuous: bool, points: (f64, f64), lines: (f64, f64), k: f64) -> Option<(i32, i32)> {
    let (dx, dy) = if continuous { (points.0 * k, points.1 * k) } else { (lines.0 * 120.0, lines.1 * 120.0) };
    let (dx, dy) = (-dx.round() as i32, dy.round() as i32);
    (dx != 0 || dy != 0).then_some((dx, dy))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: Rect = Rect { x: 0.0, y: 0.0, w: 1512.0, h: 982.0 };
    const WIN: Option<(f64, f64)> = Some((2560.0, 1440.0));

    fn enter_at(s: &mut Switcher, px: f64) -> Hop {
        s.local_move(Edge::Top, (px, 0.0), (0.0, -3.0), || MAC, || WIN)
    }

    fn back(s: &mut Switcher, d: (f64, f64)) -> Hop {
        s.remote_move(1.0, d, false, || MAC)
    }

    #[test]
    fn pushing_up_at_the_top_edge_enters_the_remote_bottom_edge() {
        let mut s = Switcher::default();
        assert_eq!(enter_at(&mut s, 756.0), Hop::Enter { x: 1280, y: 1439 });
        assert_eq!(enter_at(&mut s, 0.0), Hop::Local, "already remote");
    }

    #[test]
    fn edge_maps_proportionally_across_widths() {
        let mut s = Switcher::default();
        assert_eq!(enter_at(&mut s, 0.0), Hop::Enter { x: 0, y: 1439 });
        let mut s = Switcher::default();
        assert_eq!(enter_at(&mut s, 1511.0), Hop::Enter { x: 2557, y: 1439 });
    }

    #[test]
    fn no_hop_without_upward_push_edge_or_client() {
        let mut s = Switcher::default();
        let looked = std::cell::Cell::new(false);
        let probe = || {
            looked.set(true);
            MAC
        };
        assert_eq!(s.local_move(Edge::Top, (756.0, 0.0), (5.0, 0.0), probe, || WIN), Hop::Local, "not moving up");
        assert!(!looked.get(), "moves away from the edge never query the display");
        assert_eq!(s.local_move(Edge::Top, (756.0, 5.0), (0.0, -3.0), || MAC, || WIN), Hop::Local, "below the edge");
        assert_eq!(s.local_move(Edge::Top, (-1.0, 0.0), (0.0, -3.0), || MAC, || WIN), Hop::Local, "on another display");
        assert_eq!(s.local_move(Edge::Top, (756.0, 0.0), (0.0, -3.0), || MAC, || None), Hop::Local, "no client");
        assert_eq!(s.local_move(Edge::Left, (756.0, 0.0), (0.0, -3.0), || MAC, || WIN), Hop::Local, "edge is elsewhere");
    }

    #[test]
    fn remote_moves_clamp_scale_and_skip_repeats() {
        let mut s = Switcher::default();
        enter_at(&mut s, 756.0);
        assert_eq!(s.remote_move(2.0, (10.0, -5.0), false, || MAC), Hop::Move { x: 1300, y: 1429 });
        assert_eq!(s.remote_move(2.0, (0.1, 0.0), false, || MAC), Hop::Unmoved);
        assert_eq!(s.remote_move(2.0, (-9999.0, -9999.0), false, || MAC), Hop::Move { x: 0, y: 0 });
    }

    #[test]
    fn pushing_down_at_the_remote_bottom_returns_to_the_same_column() {
        let mut s = Switcher::default();
        enter_at(&mut s, 378.0);
        // 378 of 1512 → 639.75 of 2560 → back to 377.85 (the remote's last column is one pixel short)
        assert_eq!(back(&mut s, (0.0, 1.0)), Hop::Leave { x: 378.0 / 1512.0 * 2559.0 / 2560.0 * 1512.0, y: 2.0 });
        assert_eq!(back(&mut s, (0.0, -5.0)), Hop::Local, "back on the Mac");
    }

    #[test]
    fn every_edge_enters_the_opposite_side_and_returns_beside_the_same_spot() {
        // (edge, local point at that edge, push toward it, expected entry, push back out, expected return)
        let cases = [
            (Edge::Top, (756.0, 0.0), (0.0, -3.0), (1280, 1439), (0.0, 1.0), (756.0, 2.0)),
            (Edge::Left, (0.0, 491.0), (-3.0, 0.0), (2559, 720), (1.0, 0.0), (2.0, 491.0)),
            (Edge::Right, (1511.0, 491.0), (3.0, 0.0), (0, 720), (-1.0, 0.0), (1509.0, 491.0)),
        ];
        for (edge, at, push, entry, out, ret) in cases {
            let mut s = Switcher::default();
            assert_eq!(s.local_move(edge, at, push, || MAC, || WIN), Hop::Enter { x: entry.0, y: entry.1 }, "{edge:?} entry");
            assert_eq!(back(&mut s, (0.0, 0.0)), Hop::Unmoved, "{edge:?} no motion, no message");
            match back(&mut s, out) {
                Hop::Leave { x, y } => {
                    assert!((x - ret.0).abs() < 1.0 && (y - ret.1).abs() < 1.0, "{edge:?} return ({x}, {y}) vs {ret:?}")
                }
                other => panic!("{edge:?} did not return: {other:?}"),
            }
            assert_eq!(s.local_move(edge, (756.0, 491.0), push, || MAC, || WIN), Hop::Local, "{edge:?} mid-screen");
        }
    }

    #[test]
    fn dragging_never_leaves_the_remote_screen() {
        let mut s = Switcher::default();
        enter_at(&mut s, 756.0);
        assert_eq!(s.remote_move(1.0, (0.0, 50.0), true, || MAC), Hop::Unmoved, "clamped at the bottom while dragging");
        assert!(matches!(back(&mut s, (0.0, 1.0)), Hop::Leave { .. }));
    }

    #[test]
    fn reset_gives_the_cursor_back() {
        let mut s = Switcher::default();
        enter_at(&mut s, 756.0);
        s.reset();
        assert_eq!(back(&mut s, (1.0, 1.0)), Hop::Local);
        assert!(matches!(enter_at(&mut s, 756.0), Hop::Enter { .. }));
    }

    #[test]
    fn pinch_emits_whole_steps_and_forgets_between_gestures() {
        let mut p = Pinch::default();
        assert_eq!(p.add(0.1), 0);
        assert_eq!(p.add(0.1), 1);
        assert_eq!(p.add(-0.4), -2);
        p.end();
        assert_eq!(p.add(0.1), 0, "remainder from the last gesture is gone");
        assert_eq!(p.add(0.31), 2);
    }

    #[test]
    fn wheel_scales_trackpad_points_and_mouse_lines() {
        assert_eq!(wheel(true, (2.0, -4.0), (0.0, 0.0), 3.0), Some((-6, -12)));
        assert_eq!(wheel(false, (0.0, 0.0), (1.0, 2.0), 3.0), Some((-120, 240)));
        assert_eq!(wheel(true, (0.0, 0.0), (5.0, 5.0), 3.0), None, "continuous ignores line deltas");
    }
}
