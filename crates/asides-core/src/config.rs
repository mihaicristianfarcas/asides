//! `~/Library/Application Support/Asides/config.toml`

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::inspect::{Detail, Units};

const HEADER: &str = r#"# Asides settings. Changes are picked up automatically.
# Shortcut syntax: modifiers (ctrl, alt/option, shift, cmd) + a key (Space, Escape, KeyN, Digit1, F5, …),
# joined with '+'. Avoid option-only combinations: macOS refuses them.
# languages: Whisper codes to choose between, e.g. ["en", "ro"]; [] detects any language.
# agent_delivery: "each" hands every recording to agents listening through `asides mcp` as soon as
# it's transcribed; "batch" holds them until you choose Send to Agent.
# wispr_flow_apps: app names or bundle IDs whose Wispr Flow dictations are added; [] means any app.
# auto_new_note: start a new note when you've switched git branch, or after 8 hours without asides.
# annotation_detail: how much Measure & Annotate puts in an aside: "compact", "standard", "detailed"
# or "forensic".
# [measure] color: lines and labels (#RRGGBB); units: "pt" or "px"; tolerance: 0-100 (%);
# identify_elements: name what's under each mark (needs the Accessibility permission);
# padding: points of screen kept around what you measured in the screenshot;
# record: start recording when Measure & Annotate opens, so you can talk while you mark;
# toolbar: show its toolbar (shift+tab hides and shows it; every action also has a key).
# [measure.keys] the overlay's keys: modifiers (cmd, shift, alt, ctrl) + a key (a, enter, escape,
# tab, space, backspace, left, …) joined with '+'; several separated by commas; "" turns one off.

"#;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Holding the record shortcut records until release; a short tap toggles.
    pub hold_to_talk: bool,
    /// Images copied to the clipboard while recording are attached automatically.
    pub auto_attach_images: bool,
    pub sounds: bool,
    /// Keep an .m4a of each recording next to the note.
    pub keep_audio: bool,
    /// Speech model id, see `models::MODELS`.
    pub model: String,
    /// Input device name; empty means the system default.
    pub input_device: String,
    /// Whisper language codes a recording can be in (e.g. `["en", "ro"]`): each recording is
    /// transcribed in the most likely of these. Empty means any language Whisper knows.
    pub languages: Vec<String>,
    /// When coding agents listening through `asides mcp` receive new entries.
    pub agent_delivery: AgentDelivery,
    /// Start a new note when you've switched git branch, or after a break (`notes::moved_on`).
    pub auto_new_note: bool,
    /// Dictations made with Wispr Flow are added to the note, like recordings.
    pub wispr_flow: bool,
    /// Only Wispr Flow dictations made in these apps (names or bundle IDs, matched ignoring case);
    /// empty means any app.
    pub wispr_flow_apps: Vec<String>,
    /// How much Measure & Annotate puts in an aside.
    pub annotation_detail: Detail,
    pub measure: Measure,
    pub shortcuts: Shortcuts,
}

/// Measure & Annotate (the screen overlay).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Measure {
    /// Lines, labels and guides, as `#RRGGBB`.
    pub color: String,
    pub units: Units,
    /// Edge detection tolerance in percent; `-`/`=` change it while measuring.
    pub tolerance: u8,
    /// Name the element under each annotation marker from the accessibility tree. Needs the
    /// Accessibility permission, asked for when this is turned on.
    pub identify_elements: bool,
    /// Points of screen kept around what was measured in the screenshot added to the aside.
    pub padding: f64,
    /// Start recording when the overlay opens, to talk while marking.
    pub record: bool,
    /// Show the overlay's toolbar (its key hides and shows it; every action has a key).
    pub toolbar: bool,
    pub keys: OverlayKeys,
}

impl Default for Measure {
    fn default() -> Self {
        Self {
            color: "#FF2D55".into(),
            units: Units::Pt,
            tolerance: crate::measure::Tolerance::DEFAULT.0,
            identify_elements: false,
            padding: 48.0,
            record: false,
            toolbar: true,
            keys: OverlayKeys::default(),
        }
    }
}

/// The overlay's keys (PixelSnap's, mostly), each a comma-separated list of chords like
/// `cmd+shift+c` (see `keys::Chord`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayKeys {
    pub measure: String,
    pub mark: String,
    pub draw: String,
    pub shot: String,
    pub hold_horizontal: String,
    pub hold_vertical: String,
    pub guide_horizontal: String,
    pub guide_vertical: String,
    pub tolerance_up: String,
    pub tolerance_down: String,
    pub tolerance_high: String,
    pub units: String,
    pub copy_dimensions: String,
    pub copy_image: String,
    pub copy_markdown: String,
    pub undo: String,
    pub clear: String,
    pub hide: String,
    /// Hide or show the toolbar.
    pub toolbar: String,
    pub restore: String,
    /// Hold to talk (a tap toggles), like the record shortcut.
    pub talk: String,
    pub help: String,
    /// Add to the aside.
    pub done: String,
    /// Add to the aside with the whole screen, not just around what's marked.
    pub done_whole_screen: String,
    /// Add to the aside and send it to the agent.
    pub send: String,
    pub close: String,
}

impl Default for OverlayKeys {
    fn default() -> Self {
        let k = |s: &str| s.to_string();
        Self {
            measure: k("d"),
            mark: k("a"),
            draw: k("p"),
            shot: k("cmd+s, s"),
            hold_horizontal: k("h"),
            hold_vertical: k("v"),
            guide_horizontal: k("shift+h"),
            guide_vertical: k("shift+v"),
            tolerance_up: k("=, shift+="),
            tolerance_down: k("-"),
            tolerance_high: k("t"),
            units: k("u"),
            copy_dimensions: k("cmd+c"),
            copy_image: k("c"),
            copy_markdown: k("cmd+shift+c"),
            undo: k("backspace, cmd+z"),
            clear: k("x"),
            hide: k("tab"),
            toolbar: k("shift+tab"),
            restore: k("r"),
            talk: k("space"),
            help: k("shift+/, f1"),
            done: k("enter"),
            done_whole_screen: k("alt+enter"),
            send: k("cmd+enter"),
            close: k("escape"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentDelivery {
    /// Each entry as soon as it's written.
    #[default]
    Each,
    /// Entries wait until the user chooses Send to Agent.
    Batch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shortcuts {
    pub record: String,
    pub record_new_note: String,
    pub attach_image: String,
    pub cancel: String,
    /// Only registered while `agent_delivery` is `batch`.
    pub send_to_agent: String,
    /// Measure & Annotate: freezes the screen to measure and annotate it.
    pub measure: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hold_to_talk: true,
            auto_attach_images: true,
            sounds: true,
            keep_audio: false,
            model: crate::models::MODELS[0].id.into(),
            input_device: String::new(),
            languages: Vec::new(),
            agent_delivery: AgentDelivery::default(),
            auto_new_note: true,
            wispr_flow: false,
            wispr_flow_apps: Vec::new(),
            annotation_detail: Detail::default(),
            measure: Measure::default(),
            shortcuts: Shortcuts::default(),
        }
    }
}

impl Default for Shortcuts {
    fn default() -> Self {
        Self {
            record: "ctrl+alt+Space".into(),
            record_new_note: "ctrl+alt+KeyN".into(),
            attach_image: "ctrl+alt+KeyV".into(),
            cancel: "Escape".into(),
            send_to_agent: "ctrl+alt+Enter".into(),
            measure: "ctrl+alt+KeyM".into(),
        }
    }
}

impl Config {
    /// Reads the config, falling back to defaults for a missing file or missing keys.
    pub fn load(path: &Path) -> io::Result<Config> {
        match fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e),
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let body = toml::to_string_pretty(self).map_err(|e| io::Error::other(e.to_string()))?;
        fs::write(path, format!("{HEADER}{body}"))
    }
}

/// `ctrl+alt+Space` → `⌃⌥Space`, `cmd+shift+KeyN` → `⇧⌘N`.
pub fn pretty_shortcut(shortcut: &str) -> String {
    let mut mods = String::new();
    let mut key = String::new();
    let tokens: Vec<String> = shortcut.split('+').map(|t| t.trim().to_ascii_lowercase()).collect();
    for symbol in ["⌃", "⌥", "⇧", "⌘"] {
        let present = tokens.iter().any(|t| match symbol {
            "⌃" => t == "ctrl" || t == "control",
            "⌥" => t == "alt" || t == "option",
            "⇧" => t == "shift",
            _ => t == "cmd" || t == "command" || t == "super",
        });
        if present {
            mods.push_str(symbol);
        }
    }
    if let Some(last) = shortcut.split('+').map(str::trim).next_back() {
        let lower = last.to_ascii_lowercase();
        if !["ctrl", "control", "alt", "option", "shift", "cmd", "command", "super"].contains(&lower.as_str()) {
            key = match lower.as_str() {
                "escape" | "esc" => "Esc".into(),
                "space" => "Space".into(),
                "enter" | "return" => "Enter".into(),
                _ => last.strip_prefix("Key").or_else(|| last.strip_prefix("Digit")).unwrap_or(last).to_uppercase(),
            };
        }
    }
    format!("{mods}{key}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_prints_shortcuts() {
        assert_eq!(pretty_shortcut("ctrl+alt+Space"), "⌃⌥Space");
        assert_eq!(pretty_shortcut("cmd+shift+KeyN"), "⇧⌘N");
        assert_eq!(pretty_shortcut("Escape"), "Esc");
        assert_eq!(pretty_shortcut("ctrl+alt+Digit1"), "⌃⌥1");
        assert_eq!(pretty_shortcut("option+F5"), "⌥F5");
        assert_eq!(pretty_shortcut("ctrl+alt+Enter"), "⌃⌥Enter");
    }

    #[test]
    fn round_trips_and_fills_missing_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        assert_eq!(Config::load(&path).unwrap(), Config::default());
        let config = Config {
            keep_audio: true,
            shortcuts: Shortcuts { record: "cmd+shift+KeyR".into(), ..Default::default() },
            ..Default::default()
        };
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), config);

        fs::write(&path, "sounds = false\n[shortcuts]\ncancel = \"F12\"\n").unwrap();
        let partial = Config::load(&path).unwrap();
        assert!(!partial.sounds && partial.hold_to_talk);
        assert_eq!(partial.shortcuts.cancel, "F12");
        assert_eq!(partial.shortcuts.record, "ctrl+alt+Space");
        assert_eq!((partial.agent_delivery, partial.wispr_flow), (AgentDelivery::Each, false));

        fs::write(&path, "agent_delivery = \"batch\"\nwispr_flow = true\nwispr_flow_apps = [\"Safari\"]\n").unwrap();
        let agents = Config::load(&path).unwrap();
        assert_eq!(agents.agent_delivery, AgentDelivery::Batch);
        assert_eq!((agents.wispr_flow, agents.wispr_flow_apps), (true, vec!["Safari".to_string()]));

        // Older files have no Measure & Annotate settings.
        assert_eq!((agents.measure.clone(), agents.annotation_detail), (Measure::default(), Detail::Standard));
        assert_eq!(agents.shortcuts.measure, "ctrl+alt+KeyM");
        fs::write(
            &path,
            "annotation_detail = \"forensic\"\n[measure]\nunits = \"px\"\ntolerance = 20\n[measure.keys]\nmark = \"m\"\n",
        )
        .unwrap();
        let measure = Config::load(&path).unwrap();
        assert_eq!(
            (measure.annotation_detail, measure.measure.units, measure.measure.tolerance),
            (Detail::Forensic, Units::Px, 20)
        );
        assert_eq!(measure.measure.color, "#FF2D55");
        assert_eq!((measure.measure.keys.mark.as_str(), measure.measure.keys.draw.as_str()), ("m", "p"));
    }
}
