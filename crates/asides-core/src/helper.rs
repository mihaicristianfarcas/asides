//! How the app and the Measure & Annotate overlay talk. The overlay is a separate process
//! (`asides-overlay`, drawn with GPUI, which runs its own event loop). The app freezes the screens,
//! writes each one's pixels and a `session.json` (`Spec`) into a session directory, and starts
//! the overlay with that directory. Then they exchange JSON lines: `Event`s on the overlay's
//! standard output, `Command`s on its standard input. When the user is done, the overlay writes
//! `result.json` (`Outcome`) and the images it names into the same directory.
//!
//! Everything that needs a permission stays in the app: the screenshots (Screen Recording) and
//! naming elements (Accessibility), which the overlay asks for with `Event::Identify`.

use serde::{Deserialize, Serialize};

use crate::config::OverlayKeys;
use crate::inspect::{Detail, Element, PtRect, Screen, Units};
use crate::overlay::{Saved, Script};

pub const SPEC: &str = "session.json";
pub const RESULT: &str = "result.json";

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
    /// The Asides shortcut, as shown (`⌃⌥Space`), for the toolbar's hint.
    pub record_shortcut: String,
    /// The freeze shortcut, as shown (`⌃⌥P`), for the toolbar's hints.
    pub freeze_shortcut: String,
    /// Seconds recorded so far when a recording is under way.
    pub recording: Option<f32>,
    /// Agents get asides in batches (Send to Agent), so the toolbar offers Send; otherwise each
    /// aside reaches them when it's written, and Done is enough.
    pub batch: bool,
    /// The bound directory's name; `None` when there's nowhere to add the aside (or record).
    pub bound: Option<String>,
    /// Show the toolbar (the user can hide it; every action has a key).
    pub toolbar: bool,
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
    /// Open over the live screen (a toolbar only), not frozen: the screens have no pixels yet.
    pub live: bool,
    /// Where the toolbar goes, and folded (`AppState::toolbar_at`, `toolbar_collapsed`).
    pub toolbar_at: Option<(f64, f64)>,
    pub toolbar_collapsed: bool,
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
    /// The talk key or a microphone button went down or up (hold to talk, tap to toggle). From an
    /// annotation's card (its key), what's said is about that annotation: transcribed for it.
    Talk {
        down: bool,
        #[serde(default)]
        annotation: Option<u32>,
    },
    /// Stop recording (an annotation's card closed while recording for it).
    Stop,
    /// Throw away what was said about this annotation (to say it again).
    Unsay { annotation: u32 },
    /// Freeze the screens: answered with `Command::Frozen`.
    Freeze,
    /// Back to the live screen (give the keyboard back to the app that had it).
    Live,
    /// The user moved or folded the toolbar (remembered for next time).
    ToolbarPlaced { at: Option<(f64, f64)>, collapsed: bool },
    /// The user showed or hid the toolbar (remembered for next time).
    Toolbar { shown: bool },
    /// Something was put on the clipboard (not a screenshot to attach).
    Copied,
    /// Done: `result.json` is written.
    Finished,
    /// Closed without adding anything; what to restore next time.
    Cancelled { saved: Saved },
}

/// From the app to the overlay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
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
    /// The Asides shortcut tapped again: add it all to the aside.
    Finish,
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
    /// Send it to the listening agent too.
    pub send: bool,
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
        let spec: Spec = serde_json::from_str(r#"{"screens": [], "tolerance": 4}"#).unwrap();
        assert_eq!((spec.tolerance, spec.identify, spec.keys.annotate.as_str()), (4, false, "a"));
    }
}
