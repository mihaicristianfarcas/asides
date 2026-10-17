//! The session an `asides mcp` or `asides listen` delivers to (`sessions::Session`): its record,
//! listening as it, and whether the user's asides go to it right now.

use std::path::Path;

use asides_core::config::{Config, pretty_shortcut};
use asides_core::link::Link;
use asides_core::listening::Listening;
use asides_core::paths;
use asides_core::sessions::Session;
use asides_core::state::AppState;

/// The record of session `id` once its agent has connected: the one `asides <agent>` made (whose
/// process this takes over when it wasn't known), or a new one that isn't listening yet. Returns
/// whether it made the record, which it then removes when the agent leaves.
pub fn connect(dir: &Path, id: &str, agent: &str) -> bool {
    let sessions = paths::sessions_dir();
    let pid = std::process::id();
    match Session::read(&sessions, id) {
        Some(mut session) => {
            if !crate::is_running(session.pid) {
                session.pid = pid;
                let _ = session.write(&sessions);
            }
            false
        }
        None => {
            let session = Session {
                id: id.into(),
                agent: agent.into(),
                dir: dir.to_path_buf(),
                pid,
                started_ms: crate::now_ms(),
                ..Session::default()
            };
            session.write(&sessions).is_ok()
        }
    }
}

/// Starts listening as session `id` in `dir`: marks it listening (the user asked its agent to) and,
/// the first time, makes it the app's target, starting the app if it isn't running. Returns where
/// its asides begin (where it was launched, or now), and whether it made the session's record.
pub fn listen(dir: &Path, id: &str, agent: &str) -> (Listening, bool) {
    let sessions = paths::sessions_dir();
    let existing = Session::read(&sessions, id);
    let made = existing.is_none();
    let mut session = existing.unwrap_or_else(|| Session {
        id: id.into(),
        agent: agent.into(),
        dir: dir.to_path_buf(),
        pid: std::process::id(),
        started_ms: crate::now_ms(),
        ..Session::default()
    });
    let first = !session.listening;
    if first {
        session.listening = true;
        let _ = session.write(&sessions);
    }
    if first || crate::running_state().is_none() {
        let request = crate::request_id();
        if crate::send(&Link::Session { id: id.into(), request: Some(request.clone()) }).is_ok() {
            let _ = crate::wait_for_ack(&request);
        }
    }
    let config = Config::load(&paths::config_path()).unwrap_or_default();
    let resumed = session.start.as_deref().and_then(|token| Listening::resume(dir, Some(id), token));
    let listening = resumed.unwrap_or_else(|| {
        let sends = AppState::read(&paths::state_path()).map(|s| s.agent_sends).unwrap_or_default();
        Listening::start(dir, Some(id), config.agent_delivery, &sends)
    });
    (listening, made)
}

/// Removes the record when this process made it: the agent is gone.
pub fn leave(id: &str, made: bool) {
    if made {
        Session::remove(&paths::sessions_dir(), id);
    }
}

/// Why the user's asides wouldn't reach session `id` right now, if they wouldn't.
pub fn target_problem(id: &str) -> Option<String> {
    let Some(state) = crate::running_state() else {
        return Some("Asides isn't running, so nothing can be recorded yet.".into());
    };
    if state.target_session.as_deref() == Some(id) {
        return None;
    }
    let shortcut = pretty_shortcut(&Config::load(&paths::config_path()).unwrap_or_default().shortcuts.next_target);
    let other = state.target_session.as_deref().and_then(|other| Session::read(&paths::sessions_dir(), other));
    Some(match other {
        Some(other) => format!(
            "The user's asides go to another session right now ({}); they come here once the user switches to this \
             one (the Asides menu, or {shortcut}).",
            other.label()
        ),
        None => format!(
            "The user chose Notes Only: their asides are saved, not sent to agents, until they switch to this \
             session (the Asides menu, or {shortcut})."
        ),
    })
}
