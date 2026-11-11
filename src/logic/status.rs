//! What the Windows client tells its status window (RULES.md U5, U8): a few
//! `key=value` lines in a file the client rewrites whenever its state changes,
//! and the words the window shows for them.

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    pub connected: bool,
    /// the Mac's address while connected
    pub peer: String,
    /// the Mac's computer name, once it has said it
    pub name: String,
    /// unix seconds when this state began
    pub since: u64,
    /// why the last attempt failed, while not connected
    pub problem: String,
    /// no pairing on this machine yet (RULES.md P6)
    pub unpaired: bool,
    /// the developer bridge is open (RULES.md X5)
    pub bridge: bool,
}

impl Status {
    pub fn to_text(&self) -> String {
        let state = if self.connected { "connected" } else { "searching" };
        let problem = self.problem.replace('\n', " ");
        let name = self.name.replace('\n', " ");
        format!(
            "state={state}\npeer={}\nname={name}\nsince={}\nproblem={problem}\nunpaired={}\nbridge={}\n",
            self.peer, self.since, self.unpaired as u8, self.bridge as u8
        )
    }

    pub fn parse(text: &str) -> Status {
        let mut s = Status::default();
        for line in text.lines() {
            match line.split_once('=') {
                Some(("state", v)) => s.connected = v == "connected",
                Some(("peer", v)) => s.peer = v.to_string(),
                Some(("name", v)) => s.name = v.to_string(),
                Some(("since", v)) => s.since = v.parse().unwrap_or(0),
                Some(("problem", v)) => s.problem = v.to_string(),
                Some(("unpaired", v)) => s.unpaired = v == "1",
                Some(("bridge", v)) => s.bridge = v == "1",
                _ => {}
            }
        }
        s
    }
}

#[derive(Debug, PartialEq)]
pub enum Light {
    Green,
    Orange,
    Red,
}

/// What the drawer below the desk holds (RULES.md U5, U8, P8): the same carved tray
/// in every state, with different words in it.
#[derive(Debug, PartialEq)]
pub enum Tray {
    /// how long it has been connected, and the Mac's address
    Connected { since: String, host: String },
    /// six slots for the Mac's pairing code, under this prompt; `fresh` when never paired
    Pair { prompt: &'static str, fresh: bool },
    /// the background client is not running
    Stopped,
}

/// The window's words: a headline beside a coloured light, a line under it, and the tray.
#[derive(Debug, PartialEq)]
pub struct View {
    pub light: Light,
    pub headline: String,
    pub detail: String,
    pub tray: Tray,
    /// "开发者通道已开启" while the Mac has the bridge open (RULES.md X5)
    pub bridge: Option<String>,
}

/// The bridge line: shown whenever the bridge is open, so it is never on unnoticed.
#[cfg(not(any(test, feature = "bridge")))]
fn bridge_line(_: &Status) -> Option<String> {
    None
}

#[cfg(any(test, feature = "bridge"))]
fn bridge_line(s: &Status) -> Option<String> {
    (s.connected && s.bridge).then(|| "开发者通道已开启".to_string())
}

fn duration(secs: u64) -> String {
    match secs {
        0..=59 => "不到 1 分钟".into(),
        60..=3599 => format!("{} 分钟", secs / 60),
        _ => format!("{} 小时 {} 分钟", secs / 3600, secs % 3600 / 60),
    }
}

/// What to show, given the status file (None if missing), whether the client is running, and the time now.
pub fn describe(file: Option<&str>, running: bool, now: u64) -> View {
    if !running {
        return View {
            light: Light::Red,
            headline: "Wisp 没有在运行".into(),
            detail: "登录 Windows 时它会自动启动".into(),
            tray: Tray::Stopped,
            bridge: None,
        };
    }
    let s = file.map(Status::parse).unwrap_or_default();
    if s.connected {
        let host = s.peer.split(':').next().unwrap_or_default().to_string();
        let headline = if s.name.is_empty() { "已连接 Mac".to_string() } else { format!("已连接 {}", s.name) };
        View {
            light: Light::Green,
            headline,
            detail: "Mac 的光标越过屏幕边缘，就会来到这里".into(),
            tray: Tray::Connected { since: duration(now.saturating_sub(s.since)), host },
            bridge: bridge_line(&s),
        }
    } else if s.unpaired {
        View {
            light: Light::Orange,
            headline: "还没有配对".into(),
            detail: "在 Mac 的 Wisp 里点“配对新电脑”".into(),
            tray: Tray::Pair { prompt: "输入 Mac 上显示的配对码", fresh: true },
            bridge: None,
        }
    } else {
        let detail = if s.problem.is_empty() { "正在局域网里寻找 Mac".to_string() } else { s.problem };
        View {
            light: Light::Orange,
            headline: "正在寻找 Mac…".into(),
            detail,
            tray: Tray::Pair { prompt: "要重新配对？输入 Mac 上的配对码", fresh: false },
            bridge: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_file_round_trips() {
        let s = Status { connected: true, peer: "192.168.1.60:24801".into(), name: "小林的 MacBook Pro".into(), since: 1_793_500_000, ..Status::default() };
        assert_eq!(Status::parse(&s.to_text()), s);
        let broken = Status { problem: "第一行\n第二行".into(), ..Status::default() };
        assert_eq!(Status::parse(&broken.to_text()).problem, "第一行 第二行", "one line per field");
        assert_eq!(Status::parse("garbage\nstate=connected"), Status { connected: true, ..Status::default() });
    }

    #[test]
    fn status_window_words_for_each_state() {
        let mut connected = Status { connected: true, peer: "192.168.1.60:24801".into(), since: 1000, ..Status::default() };
        let v = describe(Some(&connected.to_text()), true, 1000 + 3 * 3600 + 5 * 60);
        assert_eq!((v.light, v.headline.as_str()), (Light::Green, "已连接 Mac"));
        assert_eq!(v.tray, Tray::Connected { since: "3 小时 5 分钟".into(), host: "192.168.1.60".into() });
        connected.name = "小林的 MacBook Pro".into();
        let v = describe(Some(&connected.to_text()), true, 1010);
        assert_eq!(v.headline, "已连接 小林的 MacBook Pro");
        assert_eq!(v.tray, Tray::Connected { since: "不到 1 分钟".into(), host: "192.168.1.60".into() });

        let failing = Status { problem: "局域网里没有找到 Mac".into(), ..Status::default() };
        let v = describe(Some(&failing.to_text()), true, 0);
        assert_eq!((v.light, v.detail.as_str()), (Light::Orange, "局域网里没有找到 Mac"));
        assert_eq!(v.tray, Tray::Pair { prompt: "要重新配对？输入 Mac 上的配对码", fresh: false }, "a missing Mac may need re-pairing");
        assert_eq!(describe(None, true, 0).detail, "正在局域网里寻找 Mac", "client just started");

        let stale = describe(Some(&connected.to_text()), false, 0);
        assert_eq!((stale.light, stale.tray), (Light::Red, Tray::Stopped), "a stale file never claims a connection");

        let unpaired = Status { unpaired: true, ..Status::default() };
        let v = describe(Some(&unpaired.to_text()), true, 0);
        assert_eq!((v.light, v.headline.as_str()), (Light::Orange, "还没有配对"));
        assert_eq!(v.tray, Tray::Pair { prompt: "输入 Mac 上显示的配对码", fresh: true });
    }

    #[test]
    fn an_open_bridge_is_always_on_show() {
        let s = Status { connected: true, since: 0, bridge: true, ..Status::default() };
        assert_eq!(Status::parse(&s.to_text()), s);
        assert_eq!(describe(Some(&s.to_text()), true, 365 * 86_400).bridge.as_deref(), Some("开发者通道已开启"), "no time limit");
        assert_eq!(describe(Some(&s.to_text()), false, 0).bridge, None);
        let closed = Status { bridge: false, ..s };
        assert_eq!(describe(Some(&closed.to_text()), true, 0).bridge, None);
    }
}
