//! WispService's decisions (RULES.md W5, W6): when to (re)start its SYSTEM client, and
//! putting an update in place. The service itself (service_win.rs) only carries them out.

use std::path::Path;

/// The client exe the service runs, under Program Files.
pub const CLIENT_EXE: &str = "wisp.exe";
/// A developer update waiting in the protected state folder.
pub const NEXT_EXE: &str = "wisp.next.exe";
/// Where the replaced client goes while the new one moves in.
pub const OLD_EXE: &str = "wisp.old.exe";
/// What the SYSTEM client exits with when the user quits: the service leaves it stopped.
pub const QUIT_EXIT: u32 = 0x5157;
/// What WTSGetActiveConsoleSessionId says while no session is attached.
pub const NO_SESSION: u32 = 0xFFFF_FFFF;

/// The client the service started: still running, and in which session.
#[derive(Debug, Clone, Copy)]
pub struct Child {
    pub alive: bool,
    pub session: u32,
}

/// What to do this round.
#[derive(Debug, PartialEq)]
pub struct Step {
    /// stop (or forget, if it already ended) the current client
    pub stop: bool,
    /// start a client in the console session
    pub start: bool,
}

/// One round of the service loop: `quit` says the user quit (and has not started Wisp
/// again since), `session` is the console session now.
pub fn step(child: Option<Child>, quit: bool, session: u32) -> Step {
    let stop = child.is_some_and(|c| !c.alive || c.session != session);
    let running = child.is_some() && !stop;
    Step { stop, start: !running && !quit && session != NO_SESSION }
}

/// Whether the client's exit code means the user quit.
pub fn quit_by_user(exit_code: u32) -> bool {
    exit_code == QUIT_EXIT
}

/// Puts the update `next` in place of `exe`, the client stopped beforehand. `exe`
/// steps aside to `old` first; if the new one cannot take its place, the old one comes
/// back. `next` is gone afterwards either way, so a bad update is not retried forever.
/// Returns whether the update went in.
pub fn put_in_place(next: &Path, exe: &Path, old: &Path) -> bool {
    if !next.exists() {
        return false;
    }
    let _ = std::fs::remove_file(old);
    let _ = std::fs::rename(exe, old);
    let ok = std::fs::rename(next, exe).is_ok();
    if ok {
        let _ = std::fs::remove_file(old);
    } else {
        // the old one back; it is never deleted unless the new one is in place
        let _ = std::fs::rename(old, exe);
        let _ = std::fs::remove_file(next);
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: u32 = 1;

    #[test]
    fn the_service_keeps_one_client_in_the_console_session() {
        assert_eq!(step(None, false, S), Step { stop: false, start: true }, "none yet: start one");
        assert_eq!(step(Some(Child { alive: true, session: S }), false, S), Step { stop: false, start: false }, "running: leave it");
        assert_eq!(step(Some(Child { alive: false, session: S }), false, S), Step { stop: true, start: true }, "crashed: start again");
        assert_eq!(step(Some(Child { alive: true, session: 2 }), false, S), Step { stop: true, start: true }, "another session took the console");
        assert_eq!(step(None, false, NO_SESSION), Step { stop: false, start: false }, "no session to start in");
    }

    #[test]
    fn a_quit_keeps_the_client_stopped_until_started_again() {
        assert!(quit_by_user(QUIT_EXIT));
        assert!(!quit_by_user(0) && !quit_by_user(1), "a crash or a kill is not a quit");
        assert_eq!(step(Some(Child { alive: false, session: S }), true, S), Step { stop: true, start: false }, "it quit: not started again");
        assert_eq!(step(None, true, S), Step { stop: false, start: false });
        assert_eq!(step(None, false, S), Step { stop: false, start: true }, "started again by the user");
    }

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("wisp-service-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn an_update_replaces_the_client() {
        let (state, bin) = (dir("state"), dir("bin"));
        let (next, exe, old) = (state.join(NEXT_EXE), bin.join(CLIENT_EXE), bin.join(OLD_EXE));
        std::fs::write(&exe, "old").unwrap();
        assert!(!put_in_place(&next, &exe, &old), "nothing waiting");
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "old");
        std::fs::write(&next, "new").unwrap();
        assert!(put_in_place(&next, &exe, &old));
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "new");
        assert!(!next.exists() && !old.exists());
        let _ = (std::fs::remove_dir_all(&state), std::fs::remove_dir_all(&bin));
    }

    #[test]
    fn an_update_that_cannot_move_in_leaves_the_old_client() {
        let (state, bin) = (dir("state2"), dir("bin2"));
        let (next, old) = (state.join(NEXT_EXE), bin.join(OLD_EXE));
        // the target's folder is missing: the rename fails
        let exe = bin.join("missing").join(CLIENT_EXE);
        std::fs::write(&next, "new").unwrap();
        assert!(!put_in_place(&next, &exe, &old));
        assert!(!next.exists(), "not retried forever");
        let _ = (std::fs::remove_dir_all(&state), std::fs::remove_dir_all(&bin));
    }
}
