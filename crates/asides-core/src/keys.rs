//! Key chords for the Measure & Annotate overlay (`[measure.keys]`): `cmd+shift+c`, several for one
//! action separated by commas, `""` for none. Keys are named as typed (`a`, `=`, `/`) or by name
//! (`enter`, `escape`, `tab`, `space`, `backspace`, `left`, `f1`, `comma`, …).

/// One chord: a key and the modifiers held with it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Chord {
    pub key: String,
    pub cmd: bool,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

/// The modifiers held with a key press.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub cmd: bool,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

/// Keys `global-hotkey` names otherwise than the settings do (letters, digits and F-keys aside).
const GLOBAL_NAMES: &[(&str, &str)] = &[
    ("space", "Space"),
    ("enter", "Enter"),
    ("escape", "Escape"),
    ("tab", "Tab"),
    ("backspace", "Backspace"),
    ("delete", "Delete"),
    ("left", "ArrowLeft"),
    ("right", "ArrowRight"),
    ("up", "ArrowUp"),
    ("down", "ArrowDown"),
    ("=", "Equal"),
    ("-", "Minus"),
    ("/", "Slash"),
    ("\\", "Backslash"),
    (",", "Comma"),
    (".", "Period"),
    (";", "Semicolon"),
    ("'", "Quote"),
    ("[", "BracketLeft"),
    ("]", "BracketRight"),
    ("`", "Backquote"),
];

/// US-layout symbols and what Shift makes of them.
const SHIFTED: &[(&str, &str)] = &[
    ("=", "+"),
    ("-", "_"),
    ("/", "?"),
    ("1", "!"),
    ("2", "@"),
    ("3", "#"),
    ("8", "*"),
    (";", ":"),
    ("'", "\""),
    (",", "<"),
    (".", ">"),
    ("[", "{"),
    ("]", "}"),
    ("\\", "|"),
    ("`", "~"),
];

/// A key's canonical name.
pub fn normalize(key: &str) -> String {
    let lower = key.trim().to_lowercase();
    match lower.as_str() {
        "return" => "enter".into(),
        "esc" => "escape".into(),
        "delete" | "del" => "backspace".into(),
        "forwarddelete" => "delete".into(),
        "plus" => "+".into(),
        "comma" => ",".into(),
        "spacebar" => "space".into(),
        _ => lower,
    }
}

/// Parses a list of chords; ones that don't make sense are skipped.
pub fn parse(list: &str) -> Vec<Chord> {
    list.split(',').filter_map(parse_chord).collect()
}

fn parse_chord(text: &str) -> Option<Chord> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let parts: Vec<&str> = if text == "+" { vec!["+"] } else { text.split('+').collect() };
    let (key, mods) = parts.split_last()?;
    // `shift++`: the last part is empty, the key is "+".
    let key = if key.is_empty() && text.ends_with("++") { "+" } else { key };
    let mut chord = modified(mods)?;
    chord.key = normalize(key);
    (!chord.key.is_empty()).then_some(chord)
}

/// A chord with the modifiers named (`cmd`, `shift`, …), its key still to set.
fn modified(mods: &[&str]) -> Option<Chord> {
    let mut chord = Chord::default();
    for m in mods {
        match m.trim().to_lowercase().as_str() {
            "cmd" | "command" | "super" | "meta" => chord.cmd = true,
            "shift" => chord.shift = true,
            "alt" | "option" | "opt" => chord.alt = true,
            "ctrl" | "control" => chord.ctrl = true,
            "" => {}
            _ => return None,
        }
    }
    Some(chord)
}

/// A global shortcut as the settings write it (`ctrl+alt+Space`, `global-hotkey`'s names), as a
/// chord: to compare with the frozen screen's keys.
pub fn global(text: &str) -> Option<Chord> {
    let parts: Vec<&str> = text.split('+').map(str::trim).collect();
    let (key, mods) = parts.split_last()?;
    let mut chord = modified(mods)?;
    let lower = key.to_lowercase();
    chord.key = match GLOBAL_NAMES.iter().find(|(_, name)| name.eq_ignore_ascii_case(key)) {
        Some((plain, _)) => plain.to_string(),
        None => match lower.strip_prefix("key").or_else(|| lower.strip_prefix("digit")) {
            Some(k) if k.len() == 1 => k.to_string(),
            _ => lower,
        },
    };
    (!chord.key.is_empty()).then_some(chord)
}

impl Chord {
    /// Whether a key press (the key as the UI names it, lowercase) is this chord. Shifted symbols
    /// match either way they're reported: `shift+/` is also `?`.
    pub fn matches(&self, key: &str, mods: Mods) -> bool {
        let key = normalize(key);
        let others = mods.cmd == self.cmd && mods.alt == self.alt && mods.ctrl == self.ctrl;
        if !others {
            return false;
        }
        if key == self.key && mods.shift == self.shift {
            return true;
        }
        let shifted = |k: &str| SHIFTED.iter().find(|(plain, _)| *plain == k).map(|(_, s)| *s);
        let plain = |k: &str| SHIFTED.iter().find(|(_, s)| *s == k).map(|(p, _)| *p);
        // Written `shift+/`, reported as `?` (with or without shift).
        (self.shift && shifted(&self.key) == Some(key.as_str()))
            // Written `?`, reported as `shift+/` or `?`.
            || (plain(&self.key) == Some(key.as_str()) && mods.shift)
            || (plain(&self.key).is_some() && key == self.key)
    }

    /// `⇧⌘C`, `⏎`, `Esc`, `Space`.
    pub fn label(&self) -> String {
        let mut text = String::new();
        for (on, symbol) in [(self.ctrl, "⌃"), (self.alt, "⌥"), (self.shift, "⇧"), (self.cmd, "⌘")] {
            if on {
                text.push_str(symbol);
            }
        }
        let key = match self.key.as_str() {
            "enter" => "⏎".to_string(),
            "escape" => "Esc".into(),
            "backspace" => "⌫".into(),
            "delete" => "⌦".into(),
            "tab" => "Tab".into(),
            "space" => "Space".into(),
            "left" => "←".into(),
            "right" => "→".into(),
            "up" => "↑".into(),
            "down" => "↓".into(),
            k => k.to_uppercase(),
        };
        // `⇧/` reads better as `?`.
        if self.shift
            && !self.cmd
            && !self.alt
            && !self.ctrl
            && let Some((_, s)) = SHIFTED.iter().find(|(p, _)| *p == self.key)
        {
            return s.to_string();
        }
        text.push_str(&key);
        text
    }
}

impl Chord {
    /// A key press (the key as the UI names it) as a chord.
    pub fn pressed(key: &str, mods: Mods) -> Option<Chord> {
        let key = normalize(key);
        (!key.is_empty()).then_some(Chord { key, cmd: mods.cmd, shift: mods.shift, alt: mods.alt, ctrl: mods.ctrl })
    }

    /// As the settings write it: `cmd+shift+c` (`comma` and `plus`, which separate chords).
    pub fn to_config(&self) -> String {
        let mut text = String::new();
        for (on, name) in [(self.ctrl, "ctrl+"), (self.alt, "alt+"), (self.shift, "shift+"), (self.cmd, "cmd+")] {
            if on {
                text.push_str(name);
            }
        }
        text.push_str(match self.key.as_str() {
            "," => "comma",
            "+" => "plus",
            key => key,
        });
        text
    }

    /// As a global shortcut (`ctrl+alt+KeyP`, `global-hotkey`'s names), when it can be one: with
    /// ⌃ or ⌘ (macOS refuses ⌥ alone), on a letter, digit, symbol or named key.
    pub fn to_global(&self) -> Option<String> {
        if !self.ctrl && !self.cmd {
            return None;
        }
        let key = match self.key.as_str() {
            k if k.len() == 1 && k.chars().all(|c| c.is_ascii_lowercase()) => format!("Key{}", k.to_uppercase()),
            k if k.len() == 1 && k.chars().all(|c| c.is_ascii_digit()) => format!("Digit{k}"),
            k if k.starts_with('f') && k[1..].parse::<u8>().is_ok_and(|n| (1..=20).contains(&n)) => k.to_uppercase(),
            k => GLOBAL_NAMES.iter().find(|(plain, _)| *plain == k)?.1.to_string(),
        };
        let mods = [(self.ctrl, "ctrl+"), (self.alt, "alt+"), (self.shift, "shift+"), (self.cmd, "cmd+")];
        Some(mods.iter().filter(|(on, _)| *on).map(|(_, m)| *m).collect::<String>() + &key)
    }
}

/// The chords as the settings write them.
pub fn to_config(list: &[Chord]) -> String {
    list.iter().map(Chord::to_config).collect::<Vec<_>>().join(", ")
}

/// Whether any chord of `list` matches the key press.
pub fn matches(list: &[Chord], key: &str, mods: Mods) -> bool {
    list.iter().any(|c| c.matches(key, mods))
}

/// The first chord's label, for hints; empty when the action has no key.
pub fn label(list: &[Chord]) -> String {
    list.first().map(Chord::label).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(cmd: bool, shift: bool, alt: bool) -> Mods {
        Mods { cmd, shift, alt, ctrl: false }
    }

    #[test]
    fn parses_and_matches_chords() {
        let copy = parse("cmd+shift+c");
        assert!(matches(&copy, "c", mods(true, true, false)));
        assert!(!matches(&copy, "c", mods(true, false, false)));
        assert_eq!(label(&copy), "⇧⌘C");
        let undo = parse("delete, cmd+z");
        assert_eq!(undo.len(), 2);
        assert!(matches(&undo, "backspace", Mods::default()) && matches(&undo, "z", mods(true, false, false)));
        assert_eq!(label(&undo), "⌫");
        let help = parse("shift+/");
        assert!(matches(&help, "?", mods(false, true, false)) && matches(&help, "/", mods(false, true, false)));
        assert_eq!(label(&help), "?");
        let up = parse("=, shift+=");
        assert!(matches(&up, "+", mods(false, true, false)) && matches(&up, "=", Mods::default()));
        let plus = parse("+");
        assert!(matches(&plus, "+", Mods::default()) && matches(&plus, "=", mods(false, true, false)));
        assert_eq!(parse("shift++")[0], Chord { key: "+".into(), shift: true, ..Chord::default() });
        assert_eq!(label(&parse("Return")), "⏎");
        assert_eq!(label(&parse("option+enter")), "⌥⏎");
        assert!(parse("").is_empty() && parse("hyper+x").is_empty());
        assert!(!matches(&parse("h"), "h", mods(false, true, false)));
    }

    #[test]
    fn writes_pressed_chords_back() {
        let chord = |key: &str, m: Mods| Chord::pressed(key, m).unwrap();
        let ctrl_alt = Mods { ctrl: true, alt: true, ..Mods::default() };
        assert_eq!(chord("Return", mods(true, true, false)).to_config(), "shift+cmd+enter");
        assert_eq!(to_config(&parse("cmd+s, s, shift+/, comma")), "cmd+s, s, shift+/, comma");
        assert_eq!(parse(&to_config(&[chord(",", Mods::default()), chord("+", Mods::default())])).len(), 2);
        assert_eq!(chord("space", ctrl_alt).to_global().as_deref(), Some("ctrl+alt+Space"));
        assert_eq!(chord("p", ctrl_alt).to_global().as_deref(), Some("ctrl+alt+KeyP"));
        assert_eq!(chord("1", mods(true, true, false)).to_global().as_deref(), Some("shift+cmd+Digit1"));
        assert_eq!(chord("f5", ctrl_alt).to_global().as_deref(), Some("ctrl+alt+F5"));
        assert_eq!(chord("/", ctrl_alt).to_global().as_deref(), Some("ctrl+alt+Slash"));
        // ⌥ alone: macOS refuses it.
        assert_eq!(chord("p", mods(false, false, true)).to_global(), None);
    }

    #[test]
    fn reads_global_shortcuts_back() {
        let shift_cmd = mods(true, true, false);
        assert_eq!(global("cmd+shift+KeyA"), Chord::pressed("a", shift_cmd));
        assert_eq!(global("shift + command + KeyP"), Chord::pressed("p", shift_cmd));
        assert_eq!(
            global("ctrl+alt+Space"),
            Chord::pressed("space", Mods { ctrl: true, alt: true, ..Mods::default() })
        );
        for key in ["a", "1", "f5", "space", "delete", "backspace", "left", "/", "=", "`", ","] {
            let chord = Chord::pressed(key, Mods { ctrl: true, ..Mods::default() }).unwrap();
            assert_eq!(global(&chord.to_global().unwrap()), Some(chord), "{key}");
        }
        assert_eq!(global("hyper+KeyA"), None);
    }
}
