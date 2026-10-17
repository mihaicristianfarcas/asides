//! Agent sessions: coding agents listening for the user's asides. Asides starts them (`asides
//! claude`, New Session in the menu), or the user does (asking an agent they started to listen).
//! The app hands each aside to one session, the target, and binds to its repo.
//!
//! Each session has a record in `sessions/<id>.json`, written when it's launched or when its agent
//! connects to `asides mcp`, and removed when it ends. A record counts once the agent listens
//! (`listening`): an agent that has the MCP server but wasn't asked to listen isn't a session the
//! user talks to. The processes that deliver asides to it (`asides mcp`, `asides listen`) also
//! write listener records naming it (`agents::Listener`), which say whether it's waiting.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::agents::Listener;

/// How long a launched session counts as starting, before its agent listens.
pub const STARTING_MS: u64 = 2 * 60 * 1000;
/// A session whose process isn't known (a background agent that hasn't connected yet) is given up
/// after this long without a listener.
pub const UNKNOWN_PID_MS: u64 = 5 * 60 * 1000;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub id: String,
    /// The agent's name: "Claude Code", "Codex", "OpenCode", or what it called itself.
    pub agent: String,
    /// The repo: where its asides are written, and what the app binds to when it's the target.
    pub dir: PathBuf,
    /// The git worktree it works in, when another session works in the repo itself.
    pub worktree: Option<String>,
    /// The process whose life is the session's: `asides <agent>` waiting for the agent, or the
    /// agent's `asides mcp`. 0 while unknown (a background agent that hasn't connected yet).
    pub pid: u32,
    /// A Claude Code background session's name, which `claude attach` takes.
    pub background: Option<String>,
    /// The Codex thread it runs in, which `codex resume` takes.
    pub thread: Option<String>,
    /// Asides sends its asides to the agent as messages (a Codex turn, an OpenCode prompt): the agent doesn't wait
    /// for them in a tool call.
    pub pushed: bool,
    /// The agent listens for asides (it was launched to, or asked to).
    pub listening: bool,
    /// Where its asides begin (an `agents::Listening` token): asides recorded for it while the
    /// agent was starting reach it once it listens.
    pub start: Option<String>,
    pub started_ms: u64,
}

/// What a session is doing, as the menu shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Launched; its agent hasn't started listening yet.
    Starting,
    /// Waiting for asides.
    Listening,
    /// Working on the last ones it got.
    Working,
    /// Asides can't hand it its asides (they're kept until they get through).
    Unreachable,
}

impl SessionState {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionState::Starting => "starting",
            SessionState::Listening => "listening",
            SessionState::Working => "working",
            SessionState::Unreachable => "not getting asides",
        }
    }
}

impl Session {
    pub fn file(sessions: &Path, id: &str) -> PathBuf {
        sessions.join(format!("{id}.json"))
    }

    pub fn read(sessions: &Path, id: &str) -> Option<Session> {
        valid_id(id).then(|| serde_json::from_str(&fs::read_to_string(Session::file(sessions, id)).ok()?).ok())?
    }

    /// Every record in `sessions`, oldest first, including those of sessions that have ended.
    pub fn read_all(sessions: &Path) -> Vec<Session> {
        let Ok(files) = fs::read_dir(sessions) else { return Vec::new() };
        let mut all: Vec<Session> = files
            .flatten()
            .filter(|f| f.path().extension().is_some_and(|e| e == "json"))
            .filter_map(|f| serde_json::from_str(&fs::read_to_string(f.path()).ok()?).ok())
            .collect();
        all.sort_by(|a: &Session, b| a.started_ms.cmp(&b.started_ms).then(a.id.cmp(&b.id)));
        all
    }

    /// Atomic write (temp file + rename), like `state.json`.
    pub fn write(&self, sessions: &Path) -> io::Result<()> {
        if !valid_id(&self.id) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "bad session id"));
        }
        fs::create_dir_all(sessions)?;
        let path = Session::file(sessions, &self.id);
        let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
        fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(io::Error::other)?)?;
        fs::rename(tmp, path)
    }

    pub fn remove(sessions: &Path, id: &str) {
        if valid_id(id) {
            let _ = fs::remove_file(Session::file(sessions, id));
        }
    }

    /// "Claude Code · shop", with the worktree when it has one.
    pub fn label(&self) -> String {
        let repo = self.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        match &self.worktree {
            Some(tree) => format!("{} · {repo} ({tree})", self.agent),
            None => format!("{} · {repo}", self.agent),
        }
    }

    /// What it's doing, from the listener records of the processes delivering to it.
    pub fn state(&self, listeners: &[Listener], now_ms: u64) -> SessionState {
        let mine: Vec<&Listener> = listeners.iter().filter(|l| l.session == self.id).collect();
        if mine.iter().any(|l| l.problem.is_some()) {
            SessionState::Unreachable
        } else if mine.iter().any(|l| l.waiting) {
            SessionState::Listening
        } else if mine.is_empty() && now_ms.saturating_sub(self.started_ms) < STARTING_MS {
            SessionState::Starting
        } else {
            SessionState::Working
        }
    }

    /// For a record whose process isn't known: whether to give up on it.
    pub fn expired(&self, listeners: &[Listener], now_ms: u64) -> bool {
        self.pid == 0
            && !listeners.iter().any(|l| l.session == self.id)
            && now_ms.saturating_sub(self.started_ms) >= UNKNOWN_PID_MS
    }
}

/// Labels for `sessions` that tell them apart: a second "Claude Code · shop" is "Claude Code · shop 2".
pub fn labels(sessions: &[Session]) -> Vec<String> {
    let base: Vec<String> = sessions.iter().map(Session::label).collect();
    base.iter()
        .enumerate()
        .map(|(i, label)| match base[..i].iter().filter(|l| *l == label).count() {
            0 => label.clone(),
            n => format!("{label} {}", n + 1),
        })
        .collect()
}

/// A new session id: short, lowercase, and unique enough on one Mac.
pub fn new_id() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos() as u64;
    let mixed = (nanos ^ (u64::from(std::process::id()) << 32)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    format!("{:08x}", mixed >> 32)
}

/// Ids name files, so only letters, digits and dashes.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, agent: &str, dir: &str, started_ms: u64) -> Session {
        Session {
            id: id.into(),
            agent: agent.into(),
            dir: dir.into(),
            started_ms,
            listening: true,
            ..Default::default()
        }
    }

    fn listener(session: &str, waiting: bool) -> Listener {
        Listener { pid: 1, session: session.into(), waiting, ..Default::default() }
    }

    #[test]
    fn records_round_trip_oldest_first() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(Session::read_all(&tmp.path().join("none")).is_empty());
        let a = session("bbb", "Codex", "/r/api", 2);
        let b = Session { worktree: Some("asides-1".into()), ..session("aaa", "Claude Code", "/r/shop", 1) };
        a.write(tmp.path()).unwrap();
        b.write(tmp.path()).unwrap();
        assert_eq!(Session::read_all(tmp.path()), [b.clone(), a.clone()]);
        assert_eq!(Session::read(tmp.path(), "bbb"), Some(a));
        assert_eq!(Session::read(tmp.path(), "../x"), None);
        Session::remove(tmp.path(), "bbb");
        assert_eq!(Session::read_all(tmp.path()), [b]);
        assert!(session("a/b", "x", "/r", 0).write(tmp.path()).is_err());
    }

    #[test]
    fn labels_tell_sessions_apart() {
        let sessions = [
            session("1", "Claude Code", "/r/shop", 1),
            session("2", "Codex", "/r/shop", 2),
            Session { worktree: Some("asides-3".into()), ..session("3", "Claude Code", "/r/shop", 3) },
            session("4", "Claude Code", "/r/shop", 4),
        ];
        assert_eq!(
            labels(&sessions),
            ["Claude Code · shop", "Codex · shop", "Claude Code · shop (asides-3)", "Claude Code · shop 2"]
        );
    }

    #[test]
    fn state_follows_the_listeners() {
        let s = session("s1", "Codex", "/r", 1_000);
        assert_eq!(s.state(&[], 2_000), SessionState::Starting);
        assert_eq!(s.state(&[], 1_000 + STARTING_MS), SessionState::Working);
        assert_eq!(s.state(&[listener("s1", true)], 2_000), SessionState::Listening);
        assert_eq!(s.state(&[listener("s1", false), listener("other", true)], 2_000), SessionState::Working);
        let failing = Listener { problem: Some("codex queue failed".into()), ..listener("s1", true) };
        assert_eq!(s.state(&[failing], 2_000), SessionState::Unreachable);
        assert!(!s.expired(&[], 2_000));
        assert!(s.expired(&[], 1_000 + UNKNOWN_PID_MS));
        assert!(!s.expired(&[listener("s1", false)], 1_000 + UNKNOWN_PID_MS));
        assert!(!Session { pid: 7, ..s }.expired(&[], u64::MAX));
    }

    #[test]
    fn ids_are_short_and_safe() {
        let id = new_id();
        assert!(valid_id(&id) && id.len() == 8, "{id}");
        assert!(!valid_id("") && !valid_id("a.json") && !valid_id("../x"));
    }
}
