//! `state.json`: what the running app publishes for the CLI (`asides status`, `asides sessions`) and
//! the asides window, and what it keeps between launches.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::agents::AgentKind;
use crate::sessions::AgentSession;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppState {
    pub pid: u32,
    /// `idle`, `open` (asides is on screen), `recording`, `transcribing`, `sending`,
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
    /// The agents installed, for a new session (the first is the default).
    pub agents: Vec<AgentKind>,
    /// The id of the last aside, and whether it got to its session.
    pub last_aside: Option<String>,
    /// The speech model is on disk (asides can listen).
    pub model_ready: bool,
    /// The microphones (input devices), for the window's settings.
    pub input_devices: Vec<String>,
    /// The user declined to put the `asides` command in /usr/local/bin; don't ask again.
    pub command_link_declined: bool,
    /// asides showed its keys once, the first time a screen was frozen.
    pub overlay_tips_seen: bool,
    /// Where the user put asides' pill: its top middle, as a fraction of the screen's width and
    /// height, so its width never moves it (`None`: at the top, in the middle).
    pub pill_at: Option<(f64, f64)>,
    /// asides' own updates: the one found, its download, how the last check went, What's New.
    pub update: crate::update::Status,
    /// The newest version whose What's New the user saw (a fresh install starts at its own).
    pub whats_new_seen: Option<String>,
    /// What macOS lets asides do, besides the microphone (`microphone`), as last looked at.
    pub permissions: Permissions,
    /// Whether asides opens at login, as macOS last said (looked at with the permissions).
    pub open_at_login: LoginItem,
    /// The user went through setup (or skipped it): the window no longer opens on it at launch.
    pub onboarded: bool,
    /// How far setup got (its step), to go on from there after asides restarts (Screen Recording
    /// can ask for that).
    pub setup_step: Option<String>,
    pub updated_ms: u64,
}

/// What macOS lets asides do, looked at while its window is open (setup shows it).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Permissions {
    /// Screen Recording: to freeze the screen.
    pub screen: bool,
    /// Accessibility: to name what's under a pin.
    pub accessibility: bool,
    /// Automation, by app ("Terminal", "Ghostty"): `granted`, `denied`, `undetermined`, or
    /// `not running` (macOS says only for an app that runs).
    pub automation: Vec<(String, String)>,
    /// When they were looked at (milliseconds since the epoch; 0: not yet).
    pub checked_ms: u64,
}

impl Permissions {
    /// Automation for `app`, as last looked at.
    pub fn automation(&self, app: &str) -> Option<&str> {
        self.automation.iter().find(|(a, _)| a.eq_ignore_ascii_case(app)).map(|(_, s)| s.as_str())
    }
}

/// Whether asides opens at login: macOS's Login Items, which System Settings → General → Login
/// Items changes too, so it's never kept in the settings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginItem {
    /// Not looked at yet.
    #[default]
    Unknown,
    Off,
    On,
    /// Turned off in System Settings: only there can it be turned on again.
    NeedsApproval,
}

/// A permission asides can ask for (setup's buttons).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Permission {
    Microphone,
    Screen,
    Accessibility,
    /// Typing into this app ("Terminal", "Ghostty").
    Automation(String),
}

impl Permission {
    pub fn as_str(&self) -> String {
        match self {
            Permission::Microphone => "microphone".into(),
            Permission::Screen => "screen".into(),
            Permission::Accessibility => "accessibility".into(),
            Permission::Automation(app) => format!("automation:{app}"),
        }
    }

    pub fn parse(text: &str) -> Option<Permission> {
        Some(match text {
            "microphone" => Permission::Microphone,
            "screen" => Permission::Screen,
            "accessibility" => Permission::Accessibility,
            other => Permission::Automation(other.strip_prefix("automation:").filter(|a| !a.is_empty())?.into()),
        })
    }
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
            permissions: Permissions {
                screen: true,
                automation: vec![("Terminal".into(), "granted".into())],
                ..Default::default()
            },
            open_at_login: LoginItem::NeedsApproval,
            onboarded: true,
            setup_step: Some("voice".into()),
            ..Default::default()
        };
        state.write(&path).unwrap();
        assert_eq!(AppState::read(&path), Some(state.clone()));
        assert_eq!(state.permissions.automation("terminal"), Some("granted"));
        for permission in [
            Permission::Microphone,
            Permission::Screen,
            Permission::Accessibility,
            Permission::Automation("Ghostty".into()),
        ] {
            assert_eq!(Permission::parse(&permission.as_str()), Some(permission));
        }
        assert_eq!(Permission::parse("automation:"), None);
    }

    #[test]
    fn older_state_files_still_load() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("state.json");
        fs::write(&path, r#"{"pid": 1, "status": "idle", "bound_dir": "/x", "agent_sends": []}"#).unwrap();
        let state = AppState::read(&path).unwrap();
        assert_eq!((state.pid, state.recent_dirs), (1, Vec::<PathBuf>::new()));
        assert_eq!(state.open_at_login, LoginItem::Unknown);
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
