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
    let mut chord = Chord::default();
    let parts: Vec<&str> = if text == "+" { vec!["+"] } else { text.split('+').collect() };
    let (key, mods) = parts.split_last()?;
    // `shift++`: the last part is empty, the key is "+".
    let key = if key.is_empty() && text.ends_with("++") { "+" } else { key };
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
    chord.key = normalize(key);
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
}
