//! `state.json`: what the running app publishes for the CLI (`asides status`, bind confirmation).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppState {
    pub pid: u32,
    pub bound_dir: Option<PathBuf>,
    /// Set by `asides new [TITLE]`: the next recording starts a new note.
    pub pending_new: Option<PendingNew>,
    /// `idle`, `recording`, `transcribing`, `downloading`, `error`.
    pub status: String,
    /// Human-readable detail, e.g. the download progress or the last error.
    pub detail: Option<String>,
    /// `granted`, `denied`, `undetermined`.
    pub microphone: String,
    /// Echo of the `req` parameter of the last link the app handled.
    pub last_request: Option<String>,
    /// Directories bound recently, most recent first (shown in the menu for quick switching).
    pub recent_dirs: Vec<PathBuf>,
    /// The last Send to Agent in each directory, newest first. Agents listening in batch mode
    /// deliver everything up to the newest; an agent that starts listening in a directory begins
    /// its first batch after that directory's last one.
    pub agent_sends: Vec<AgentSend>,
    /// The user declined to put the `asides` command in /usr/local/bin; don't ask again.
    pub command_link_declined: bool,
    /// Measure & Annotate showed its keys once, the first time it opened.
    pub overlay_tips_seen: bool,
    pub updated_ms: u64,
}

/// Marks the end of a batch: the entries of `dir`'s notes up to entry `entry` of `note` (a note
/// folder), all of them transcribed by the time it was published.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentSend {
    /// Milliseconds since the Unix epoch; tells one send from the next, across app restarts.
    pub at_ms: u64,
    pub dir: PathBuf,
    pub note: Option<PathBuf>,
    pub entry: u32,
}

/// How many directories the menu offers for quick switching.
pub const MAX_RECENT_DIRS: usize = 8;

/// Puts `send` first in `sends`, replacing its directory's previous one, within the limit.
pub fn remember_send(sends: &mut Vec<AgentSend>, send: AgentSend) {
    sends.retain(|s| s.dir != send.dir);
    sends.insert(0, send);
    sends.truncate(MAX_RECENT_DIRS);
}

/// Moves `dir` to the front of `recent`, dropping duplicates and the oldest beyond the limit.
pub fn remember_dir(recent: &mut Vec<PathBuf>, dir: &Path) {
    recent.retain(|d| d != dir);
    recent.insert(0, dir.to_path_buf());
    recent.truncate(MAX_RECENT_DIRS);
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PendingNew {
    pub title: Option<String>,
}

impl AppState {
    pub fn read(path: &Path) -> Option<AppState> {
        serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
    }

    /// Atomic write (temp file + rename) so readers never see a partial file.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(io::Error::other)?)?;
        fs::rename(tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("state.json");
        assert_eq!(AppState::read(&path), None);
        let state = AppState {
            pid: 42,
            bound_dir: Some("/x".into()),
            pending_new: Some(PendingNew { title: Some("T".into()) }),
            status: "idle".into(),
            last_request: Some("abc".into()),
            recent_dirs: vec!["/x".into(), "/y".into()],
            agent_sends: vec![AgentSend { at_ms: 7, dir: "/x".into(), note: Some("/x/.asides/n".into()), entry: 3 }],
            ..Default::default()
        };
        state.write(&path).unwrap();
        assert_eq!(AppState::read(&path), Some(state));
    }

    #[test]
    fn older_state_files_still_load() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("state.json");
        fs::write(&path, r#"{"pid": 1, "status": "idle"}"#).unwrap();
        let state = AppState::read(&path).unwrap();
        assert_eq!((state.recent_dirs, state.agent_sends), (Vec::<PathBuf>::new(), Vec::new()));
    }

    #[test]
    fn remembers_the_last_send_in_each_dir() {
        let send = |at_ms, dir: &str| AgentSend { at_ms, dir: dir.into(), ..Default::default() };
        let mut sends = Vec::new();
        for (at, dir) in [(1, "/a"), (2, "/b"), (3, "/a")] {
            remember_send(&mut sends, send(at, dir));
        }
        assert_eq!(sends, [send(3, "/a"), send(2, "/b")]);
    }

    #[test]
    fn remembers_recent_dirs_newest_first() {
        let mut recent = Vec::new();
        for d in ["/a", "/b", "/a", "/c"] {
            remember_dir(&mut recent, Path::new(d));
        }
        assert_eq!(recent, [PathBuf::from("/c"), "/a".into(), "/b".into()]);
        for i in 0..20 {
            remember_dir(&mut recent, Path::new(&format!("/d{i}")));
        }
        assert_eq!(recent.len(), MAX_RECENT_DIRS);
        assert_eq!(recent[0], PathBuf::from("/d19"));
    }
}
