//! The controlling machine's settings (RULES.md U2, U3, U7), kept in a small
//! `key = value` file. Reading is lenient, so a hand edit never stops wisp from
//! starting; changing a value through `set` is strict.

use std::ops::RangeInclusive;

/// Which side of this screen the other screen sits on. There is no "below": a
/// MacBook's bottom edge is the Dock's auto-hide strip, which reveals the cursor
/// frozen there while the other screen is in control (user report, 2026-11-01).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Edge {
    Top,
    Left,
    Right,
}

impl Edge {
    pub fn name(self) -> &'static str {
        match self {
            Edge::Top => "top",
            Edge::Left => "left",
            Edge::Right => "right",
        }
    }

    fn parse(s: &str) -> Option<Edge> {
        [Edge::Top, Edge::Left, Edge::Right].into_iter().find(|e| e.name() == s)
    }
}

pub const SPEED: RangeInclusive<f64> = 0.2..=5.0;
pub const SCROLL: RangeInclusive<f64> = 0.5..=10.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub edge: Edge,
    /// local motion to remote pixels
    pub speed: f64,
    /// trackpad scroll points to wheel units
    pub scroll: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { edge: Edge::Top, speed: 1.0, scroll: 3.0 }
    }
}

fn number(value: &str, range: &RangeInclusive<f64>, what: &str) -> Result<f64, String> {
    match value.parse::<f64>() {
        Ok(v) if range.contains(&v) => Ok(v),
        _ => Err(format!("{what}要在 {} 到 {} 之间", range.start(), range.end())),
    }
}

impl Settings {
    /// Applies one change; unknown keys and out-of-range values are refused and leave it as it was.
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        match key {
            "edge" => self.edge = Edge::parse(value).ok_or("方向只能是 top、left、right")?,
            "speed" => self.speed = number(value, &SPEED, "鼠标速度")?,
            "scroll" => self.scroll = number(value, &SCROLL, "滚动速度")?,
            _ => return Err(format!("没有这个设置：{key}")),
        }
        Ok(())
    }

    /// Reads a config file: defaults, overridden by every line that is valid.
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=') {
                let _ = s.set(k.trim(), v.trim());
            }
        }
        s
    }

    pub fn to_text(&self) -> String {
        format!("edge = {}\nspeed = {}\nscroll = {}\n", self.edge.name(), self.speed, self.scroll)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_reject_nonsense() {
        let mut s = Settings::default();
        s.set("edge", "left").unwrap();
        s.set("speed", "1.5").unwrap();
        s.set("scroll", "0.5").unwrap();
        assert_eq!(Settings::parse(&s.to_text()), s, "file round trip");

        assert!(s.set("speed", "9").is_err(), "out of range");
        assert!(s.set("speed", "fast").is_err(), "not a number");
        assert!(s.set("edge", "diagonal").is_err());
        assert!(s.set("volume", "11").is_err(), "unknown key");
        assert_eq!(s.speed, 1.5, "a refused change leaves the value alone");

        let messy = "# hand edited\nspeed = 2\nedge=right\nscroll = 99\nnonsense\n";
        assert_eq!(Settings::parse(messy), Settings { edge: Edge::Right, speed: 2.0, scroll: 3.0 });
        assert_eq!(Settings::parse("edge = bottom\n").edge, Edge::Top, "the retired bottom edge falls back to top");
        assert_eq!(Settings::parse(""), Settings::default());
    }
}
