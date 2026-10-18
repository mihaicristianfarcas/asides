//! `state.json`: what the running app publishes for the CLI (`asides status`, `asides sessions`) and
//! the Asides window, and what it keeps between launches.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::sessions::AgentSession;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppState {
    pub pid: u32,
    /// `idle`, `open` (Asides is on screen), `recording`, `transcribing`, `sending`,
    /// `downloading`, `error`.
    pub status: String,
    /// Human-readable detail, e.g. the download progress or the last error.
    pub detail: Option<String>,
    /// `granted`, `denied`, `undetermined`.
    pub microphone: String,
    /// Echo of the `req` parameter of the last link the app handled.
    pub last_request: Option<String>,
    /// The agent sessions running, as the app last found them (`sessions::discover`).
    pub sessions: Vec<AgentSession>,
    /// The session the user picked last (its id), which stays the target while it runs.
    pub target: Option<String>,
    /// Folders where the user sent asides or started sessions, most recent first (New session in…).
    pub recent_dirs: Vec<PathBuf>,
    /// The id of the last aside, and whether it got to its session.
    pub last_aside: Option<String>,
    /// The speech model is on disk (asides can listen).
    pub model_ready: bool,
    /// The microphones (input devices), for the window's settings.
    pub input_devices: Vec<String>,
    /// The user declined to put the `asides` command in /usr/local/bin; don't ask again.
    pub command_link_declined: bool,
    /// Asides showed its keys once, the first time a screen was frozen.
    pub overlay_tips_seen: bool,
    /// Where the user put Asides' pill: its center, as a fraction of the screen's width and height
    /// (`None`: at the top, in the middle).
    pub pill_at: Option<(f64, f64)>,
    pub updated_ms: u64,
}

/// How many folders New session in… offers.
pub const MAX_RECENT_DIRS: usize = 10;

/// Moves `dir` to the front of `recent`, dropping duplicates and the oldest beyond the limit.
pub fn remember_dir(recent: &mut Vec<PathBuf>, dir: &Path) {
    recent.retain(|d| d != dir);
    recent.insert(0, dir.to_path_buf());
    recent.truncate(MAX_RECENT_DIRS);
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
            status: "idle".into(),
            last_request: Some("abc".into()),
            recent_dirs: vec!["/x".into(), "/y".into()],
            target: Some("123".into()),
            pill_at: Some((0.5, 0.1)),
            ..Default::default()
        };
        state.write(&path).unwrap();
        assert_eq!(AppState::read(&path), Some(state));
    }

    #[test]
    fn older_state_files_still_load() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("state.json");
        fs::write(&path, r#"{"pid": 1, "status": "idle", "bound_dir": "/x", "agent_sends": []}"#).unwrap();
        let state = AppState::read(&path).unwrap();
        assert_eq!((state.pid, state.recent_dirs), (1, Vec::<PathBuf>::new()));
    }

    #[test]
    fn remembers_recent_dirs() {
        let mut recent = Vec::new();
        for dir in ["/a", "/b", "/a"] {
            remember_dir(&mut recent, Path::new(dir));
        }
        assert_eq!(recent, [PathBuf::from("/a"), PathBuf::from("/b")]);
    }
}
