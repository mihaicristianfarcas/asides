//! `~/Library/Application Support/Asides/config.toml`

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::agents::AgentKind;
use crate::inspect::{Detail, Units};

const HEADER: &str = r#"# Asides settings (the Asides window changes most of them). Changes are picked up automatically.
# Shortcut syntax: modifiers (ctrl, alt/option, shift, cmd) + a key (Space, Escape, KeyN, Digit1, F5, …),
# joined with '+'. Avoid option-only combinations: macOS refuses them.
# mic_on_open: start listening when Asides opens (its microphone button and Space still work).
# languages: Whisper codes to choose between, e.g. ["en", "ro"]; [] detects any language (only the
# multilingual models use them).
# history_days: how long asides stay in ~/.asides/history; 0 keeps them.
# annotation_detail: how much each annotation says in an aside: "compact", "standard", "detailed"
# or "forensic".
# [measure] color: lines and labels (#RRGGBB); units: "pt" or "px"; tolerance: 0-100 (%);
# identify_elements: name what's under each annotation (needs the Accessibility permission);
# padding: points of screen kept around what you annotated in the screenshot.
# [agents] terminal: the app new sessions open in ("Terminal" when ""; "Ghostty", "Diri"); claude,
# codex, opencode, gemini, copilot, qwen: extra arguments for that agent's new sessions, e.g.
# claude = ["--model", "opus"].
# [measure.keys] the overlay's keys: modifiers (cmd, shift, alt, ctrl) + a key (a, enter, escape,
# tab, space, backspace, left, …) joined with '+'; several separated by commas; "" turns one off.

"#;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Start listening when Asides opens (the pill's microphone and Space still turn it on and off).
    pub mic_on_open: bool,
    pub sounds: bool,
    /// Keep an .m4a of what was said with each aside in the history.
    pub keep_audio: bool,
    /// Speech model id, see `models::MODELS`.
    pub model: String,
    /// Input device name; empty means the system default.
    pub input_device: String,
    /// Whisper language codes a recording can be in (e.g. `["en", "ro"]`): each recording is
    /// transcribed in the most likely of these. Empty means any language Whisper knows.
    pub languages: Vec<String>,
    /// How many days the history keeps asides; 0 keeps them all.
    pub history_days: u32,
    /// Wispr Flow dictations made while the screen is frozen join the aside.
    pub wispr_flow: bool,
    /// How much Asides puts in an aside about each annotation.
    pub annotation_detail: Detail,
    pub measure: Measure,
    /// New agent sessions (New session in…).
    pub agents: Agents,
    pub shortcuts: Shortcuts,
}

/// New agent sessions.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Agents {
    /// The app new sessions open in: "Terminal" (the default when empty), "Ghostty" or "Diri".
    pub terminal: String,
    /// The agent New session in… starts unless the user picks another (its program).
    pub default: String,
    /// Extra command-line arguments for each agent's sessions.
    pub claude: Vec<String>,
    pub codex: Vec<String>,
    pub opencode: Vec<String>,
    pub gemini: Vec<String>,
    pub copilot: Vec<String>,
    pub qwen: Vec<String>,
}

impl Agents {
    /// The extra arguments for `kind`'s sessions.
    pub fn args(&self, kind: AgentKind) -> &[String] {
        match kind {
            AgentKind::Claude => &self.claude,
            AgentKind::Codex => &self.codex,
            AgentKind::OpenCode => &self.opencode,
            AgentKind::Gemini => &self.gemini,
            AgentKind::Copilot => &self.copilot,
            AgentKind::Qwen => &self.qwen,
        }
    }

    /// The agent new sessions start with.
    pub fn default_agent(&self) -> AgentKind {
        AgentKind::parse(self.default.trim()).unwrap_or(AgentKind::Claude)
    }
}

/// Asides on screen (the overlay).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Measure {
    /// Lines, labels and guides, as `#RRGGBB`.
    pub color: String,
    pub units: Units,
    /// Edge detection tolerance in percent; `-`/`=` change it while measuring.
    pub tolerance: u8,
    /// Name the element under each annotation from the accessibility tree. Needs the Accessibility
    /// permission, asked for the first time Asides opens.
    pub identify_elements: bool,
    /// Points of screen kept around what was annotated in the screenshot added to the aside.
    pub padding: f64,
    pub keys: OverlayKeys,
}

impl Default for Measure {
    fn default() -> Self {
        Self {
            color: "#FF2D55".into(),
            units: Units::Pt,
            tolerance: crate::measure::Tolerance::DEFAULT.0,
            identify_elements: true,
            padding: 48.0,
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
    pub annotate: String,
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
    /// Hide what's drawn on the frozen screen while it's held.
    pub hide: String,
    pub restore: String,
    /// Hold to talk while the microphone is off.
    pub talk: String,
    /// The next agent session the aside goes to.
    pub target: String,
    pub help: String,
    /// Send the aside (an open card is saved and closed instead).
    pub done: String,
    /// Send it with the whole screen, not just around what's annotated.
    pub done_whole_screen: String,
    /// Back to the live screen (the freeze shortcut does it too, from anywhere).
    pub live: String,
    /// Close without sending anything; a second press when there's something to lose.
    pub close: String,
}

impl Default for OverlayKeys {
    fn default() -> Self {
        let k = |s: &str| s.to_string();
        Self {
            measure: k("m"),
            annotate: k("a"),
            draw: k("d"),
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
            hide: k("shift+tab"),
            restore: k("r"),
            talk: k("space"),
            target: k("tab"),
            help: k("shift+/, f1"),
            done: k("enter"),
            done_whole_screen: k("alt+enter"),
            live: k(""),
            close: k("escape"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shortcuts {
    /// Opens Asides with the screen frozen; while it's open, a tap sends the aside.
    pub asides: String,
    /// Switches between the frozen screen and the live one; with Asides closed, opens it live.
    pub freeze: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mic_on_open: true,
            sounds: true,
            keep_audio: false,
            model: crate::models::DEFAULT.into(),
            input_device: String::new(),
            languages: Vec::new(),
            history_days: 30,
            wispr_flow: false,
            annotation_detail: Detail::default(),
            measure: Measure::default(),
            agents: Agents::default(),
            shortcuts: Shortcuts::default(),
        }
    }
}

impl Default for Shortcuts {
    fn default() -> Self {
        Self { asides: "ctrl+alt+Space".into(), freeze: "ctrl+alt+KeyP".into() }
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
                "tab" => "Tab".into(),
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
        assert_eq!(pretty_shortcut("ctrl+alt+Tab"), "⌃⌥Tab");
    }

    #[test]
    fn round_trips_and_fills_missing_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        assert_eq!(Config::load(&path).unwrap(), Config::default());
        let config = Config {
            keep_audio: true,
            shortcuts: Shortcuts { asides: "cmd+shift+KeyR".into(), ..Default::default() },
            ..Default::default()
        };
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), config);

        fs::write(&path, "sounds = false\n[shortcuts]\nfreeze = \"F12\"\n").unwrap();
        let partial = Config::load(&path).unwrap();
        assert!(!partial.sounds && partial.mic_on_open);
        assert_eq!(partial.shortcuts.freeze, "F12");
        assert_eq!(partial.shortcuts.asides, "ctrl+alt+Space");
        assert_eq!((partial.history_days, partial.wispr_flow), (30, false));
        assert!(partial.measure.identify_elements, "names elements by default");

        // Files from before the redesign still load: what's gone is ignored.
        fs::write(
            &path,
            "agent_delivery = \"batch\"\nhold_to_talk = false\nannotation_detail = \"forensic\"\n[measure]\nunits = \"px\"\n\
             record = true\ntoolbar = false\n[measure.keys]\nannotate = \"q\"\nsend = \"cmd+enter\"\n[shortcuts]\n\
             next_target = \"ctrl+alt+Tab\"\n[agents]\nterminal = \"Ghostty\"\nworktrees = false\nclaude = [\"--model\", \"opus\"]\n",
        )
        .unwrap();
        let old = Config::load(&path).unwrap();
        assert_eq!((old.annotation_detail, old.measure.units), (Detail::Forensic, Units::Px));
        assert_eq!((old.measure.keys.annotate.as_str(), old.measure.keys.measure.as_str()), ("q", "m"));
        assert_eq!((old.agents.terminal.as_str(), old.agents.claude.len()), ("Ghostty", 2));
        assert_eq!(old.agents.default_agent(), AgentKind::Claude);
    }
}
