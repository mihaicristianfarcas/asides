//! How the app and the Asides overlay talk. The overlay is a separate process
//! (`asides-overlay`, drawn with GPUI, which runs its own event loop). The app freezes the screens,
//! writes each one's pixels and a `session.json` (`Spec`) into a session directory, and starts
//! the overlay with that directory. Then they exchange JSON lines: `Event`s on the overlay's
//! standard output, `Command`s on its standard input. When the user is done, the overlay writes
//! `result.json` (`Outcome`) and the images it names into the same directory.
//!
//! The app starts the next overlay ahead of time, with `STANDBY` in place of the directory: it
//! starts GPUI and waits, and the app opens the session with `Command::Open`.
//!
//! Everything that needs a permission stays in the app: the screenshots (Screen Recording) and
//! naming elements (Accessibility), which the overlay asks for with `Event::Identify`.

use serde::{Deserialize, Serialize};

use crate::agents::AgentKind;
use crate::config::OverlayKeys;
use crate::inspect::{Detail, Element, PtRect, Screen, Units};
use crate::overlay::{Saved, Script};

pub const SPEC: &str = "session.json";
pub const RESULT: &str = "result.json";
/// The overlay's argument to start ahead of time, without a session.
pub const STANDBY: &str = "--standby";
/// The overlay's argument to open the Asides window (History, Sessions, Settings) instead.
pub const WINDOW: &str = "--window";

/// What the overlay shows and how (`session.json`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Spec {
    pub screens: Vec<SpecScreen>,
    /// Lines and labels, `#RRGGBB`.
    pub color: String,
    pub units: Units,
    pub tolerance: u8,
    pub detail: Detail,
    /// Points of screen kept around what's annotated, in the image added to the aside.
    pub padding: f64,
    /// Ask the app to name the element under each annotation.
    pub identify: bool,
    pub keys: OverlayKeys,
    /// The Asides shortcut, as shown (`⌃⌥Space`), for the pill's hints.
    pub record_shortcut: String,
    /// The freeze shortcut, as shown (`⌃⌥P`), for the pill's hints.
    pub freeze_shortcut: String,
    /// Seconds recorded so far when a recording is under way.
    pub recording: Option<f32>,
    /// Start listening as it opens (the setting); the pill's microphone turns it on and off.
    pub listen: bool,
    /// A speech model is on disk; otherwise the microphone offers to download one.
    pub model: ModelInfo,
    /// Where the aside can go (the agent sessions running), and where it goes unless the user picks
    /// another; the app sends updates (`Command::Targets`).
    pub targets: Targets,
    /// The last session's guides and held measurements, for R.
    pub saved: Saved,
    /// Show the keys at the start (the first time).
    pub tips: bool,
    /// Play these steps instead of waiting for the user (`asides simulate-measure`).
    pub script: Option<Script>,
    /// With a script: open the windows and play it slowly, to watch (development).
    pub show: bool,
    /// Development: one small window per screen at this frame (in the screen's points) instead
    /// of covering the screen.
    pub windowed: Option<PtRect>,
    /// Development: mouse and key steps to play in the windows, with screenshots of them (see the
    /// overlay's `replay`), to check what the user would see.
    pub replay: Option<serde_json::Value>,
    /// Open over the live screen (the pill only), not frozen: the screens have no pixels yet.
    pub live: bool,
    /// Where the pill goes (`AppState::pill_at`).
    pub pill_at: Option<(f64, f64)>,
}

/// The speech model, as the pill shows it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelInfo {
    pub ready: bool,
    /// What a download would get ("English (190 MB)").
    pub label: String,
    /// Downloading: how far (0 to 1).
    pub downloading: Option<f32>,
}

/// Where an aside can go.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Targets {
    /// The agent sessions running, the one typed into last first.
    pub sessions: Vec<TargetSession>,
    /// Folders a new session can start in, most recent first.
    pub folders: Vec<Folder>,
    /// The agents installed, for a new session (the first is the default).
    pub agents: Vec<AgentKind>,
    /// Where the aside goes unless the user picks another.
    pub chosen: Option<Target>,
    /// The app is still looking (the list may grow).
    pub looking: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TargetSession {
    pub id: String,
    /// "Claude Code · asides-6b".
    pub label: String,
    /// The app it runs in ("Terminal").
    pub app: String,
    /// Why Asides can't type into it, if it can't.
    pub unreachable: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Folder {
    pub path: std::path::PathBuf,
    /// Its name ("shop").
    pub name: String,
}

/// Where the aside goes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    /// A running session (its id).
    Session { id: String },
    /// A new session of `agent` in `dir`, started with the aside.
    New { agent: AgentKind, dir: std::path::PathBuf },
    /// Nowhere: kept in the history only.
    History,
}

impl Targets {
    /// The pill's label for `target`.
    pub fn label(&self, target: Option<&Target>) -> String {
        match target {
            Some(Target::Session { id }) => self
                .sessions
                .iter()
                .find(|s| &s.id == id)
                .map(|s| s.label.clone())
                .unwrap_or_else(|| "A session".into()),
            Some(Target::New { agent, dir }) => {
                let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                format!("New {} · {name}", agent.name())
            }
            Some(Target::History) | None => "History only".into(),
        }
    }

    /// The choice after `target`, for Tab: the reachable sessions, then History only, then round.
    pub fn next(&self, target: Option<&Target>) -> Target {
        let mut choices: Vec<Target> = self
            .sessions
            .iter()
            .filter(|s| s.unreachable.is_none())
            .map(|s| Target::Session { id: s.id.clone() })
            .collect();
        if let Some(new @ Target::New { .. }) = target {
            choices.push(new.clone());
        }
        choices.push(Target::History);
        let at = target.and_then(|t| choices.iter().position(|c| c == t));
        match at {
            Some(i) => choices[(i + 1) % choices.len()].clone(),
            None => choices[0].clone(),
        }
    }
}

/// One frozen screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpecScreen {
    /// The display's CoreGraphics id.
    pub display: u32,
    pub screen: Screen,
    /// Top-left corner in global points (CoreGraphics coordinates, y down).
    pub origin: (f64, f64),
    /// The display's frame in AppKit's coordinates (origin at the bottom-left of the main screen).
    pub frame: PtRect,
    /// The pixels: a file in the session directory, RGBA, `width × height`.
    pub pixels: String,
    pub width: u32,
    pub height: u32,
    /// Where the user was, when the window they were in is on this screen (`**In:** …`).
    #[serde(default)]
    pub place: Option<String>,
    /// How far down from its top the menu bar reaches, the notch with it, in points: what the overlay
    /// draws stays below it.
    #[serde(default)]
    pub menu_bar: f64,
}

/// From the overlay to the app.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// The windows are up.
    Ready,
    /// Name the element at this point (global points); answered with `Command::Element`.
    Identify { id: u64, x: f64, y: f64 },
    /// The talk key went down or up (hold to talk while the microphone is off), or an annotation's
    /// card's microphone (its key): what's said is about that annotation, transcribed for it.
    Talk {
        down: bool,
        #[serde(default)]
        annotation: Option<u32>,
    },
    /// The pill's microphone: listen, or stop.
    Mic { on: bool },
    /// Download the speech model (the microphone, without one).
    DownloadModel,
    /// Stop recording (an annotation's card closed while recording for it).
    Stop,
    /// Throw away what was said about this annotation (to say it again).
    Unsay { annotation: u32 },
    /// Freeze the screens: answered with `Command::Frozen`.
    Freeze,
    /// Back to the live screen (give the keyboard back to the app that had it).
    Live,
    /// The user picked where the aside goes.
    Target { target: Target },
    /// The user moved the pill (remembered for next time).
    PillPlaced { at: Option<(f64, f64)> },
    /// Something was put on the clipboard (not a screenshot to attach).
    Copied,
    /// Send: `result.json` is written.
    Finished,
    /// Closed without sending anything; what to restore next time.
    Cancelled { saved: Saved },
}

/// From the app to the overlay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    /// Open the session in this directory (to an overlay started with `STANDBY`).
    Open {
        dir: std::path::PathBuf,
    },
    Element {
        id: u64,
        element: Option<Box<Element>>,
    },
    /// Recording (seconds so far), or not, and for which annotation (from its card).
    Recording {
        seconds: Option<f32>,
        #[serde(default)]
        annotation: Option<u32>,
    },
    /// What was said about an annotation (recording `take`) is being transcribed...
    Transcribing {
        annotation: u32,
        take: u64,
    },
    /// ...and this is its transcript.
    Said {
        annotation: u32,
        take: u64,
        text: String,
    },
    /// The Asides shortcut tapped again: send the aside.
    Finish,
    /// Where the aside can go, updated (sessions found, ended).
    Targets {
        targets: Targets,
    },
    /// The speech model's download.
    Model {
        model: ModelInfo,
    },
    /// The freeze shortcut: freeze the screens, or back to live.
    ToggleFreeze,
    /// The screens, frozen (their pixels are files in the session directory).
    Frozen {
        screens: Vec<SpecScreen>,
    },
    /// Close without adding anything.
    Close,
    /// Something the user should know while the overlay covers the menu bar (an error).
    Notice {
        text: String,
    },
}

/// What the session adds to the aside (`result.json`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Outcome {
    /// Where it goes.
    pub target: Option<Target>,
    /// Guides and held measurements, to restore next time.
    pub saved: Saved,
    /// In order: images (files in the session directory) and Markdown, each with the seconds into
    /// the session it belongs at.
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Item {
    pub time: f32,
    pub image: Option<String>,
    pub text: Option<String>,
    /// An annotation's key, for what was said about it that `text` doesn't have yet (recordings
    /// other than `said`, still being transcribed).
    pub annotation: Option<u32>,
    pub said: Vec<u64>,
}

/// One JSON line.
pub fn line<T: Serialize>(message: &T) -> String {
    serde_json::to_string(message).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_tagged_json_lines() {
        assert_eq!(
            line(&Event::Identify { id: 3, x: 10.0, y: 20.5 }),
            r#"{"type":"identify","id":3,"x":10.0,"y":20.5}"#
        );
        assert_eq!(line(&Event::Ready), r#"{"type":"ready"}"#);
        let command: Command = serde_json::from_str(r#"{"type":"recording","seconds":4.5}"#).unwrap();
        assert_eq!(command, Command::Recording { seconds: Some(4.5), annotation: None });
        assert_eq!(
            line(&Event::Talk { down: true, annotation: Some(2) }),
            r#"{"type":"talk","down":true,"annotation":2}"#
        );
        let said: Command =
            serde_json::from_str(r#"{"type":"said","annotation":2,"take":7,"text":"Bigger."}"#).unwrap();
        assert_eq!(said, Command::Said { annotation: 2, take: 7, text: "Bigger.".into() });
        let element: Command = serde_json::from_str(r#"{"type":"element","id":1,"element":null}"#).unwrap();
        assert_eq!(element, Command::Element { id: 1, element: None });
        assert_eq!(
            line(&Command::Open { dir: "/tmp/asides-overlay-1-0".into() }),
            r#"{"type":"open","dir":"/tmp/asides-overlay-1-0"}"#
        );
        let spec: Spec = serde_json::from_str(r#"{"screens": [], "tolerance": 4}"#).unwrap();
        assert_eq!((spec.tolerance, spec.identify, spec.keys.annotate.as_str()), (4, false, "a"));
        let target = Event::Target { target: Target::New { agent: AgentKind::Codex, dir: "/r/shop".into() } };
        assert_eq!(line(&target), r#"{"type":"target","target":{"kind":"new","agent":"codex","dir":"/r/shop"}}"#);
    }

    #[test]
    fn tab_goes_through_the_sessions_asides_can_reach() {
        let session = |id: &str, unreachable: Option<&str>| TargetSession {
            id: id.into(),
            label: format!("Claude Code · {id}"),
            app: "Terminal".into(),
            unreachable: unreachable.map(String::from),
        };
        let targets = Targets {
            sessions: vec![session("1", None), session("2", Some("Warp")), session("3", None)],
            ..Targets::default()
        };
        let s = |id: &str| Target::Session { id: id.into() };
        assert_eq!(targets.next(None), s("1"));
        assert_eq!(targets.next(Some(&s("1"))), s("3"));
        assert_eq!(targets.next(Some(&s("3"))), Target::History);
        assert_eq!(targets.next(Some(&Target::History)), s("1"));
        let new = Target::New { agent: AgentKind::Claude, dir: "/r/shop".into() };
        assert_eq!(targets.next(Some(&new)), Target::History);
        assert_eq!(targets.label(Some(&new)), "New Claude Code · shop");
        assert_eq!(targets.label(Some(&s("3"))), "Claude Code · 3");
        assert_eq!(targets.label(None), "History only");
    }
}
