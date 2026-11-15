//! Searching the history. An aside is found when it says every word looked for, in any order,
//! ignoring case and accents ("sterge" finds "Șterge"): in what was said, its pins, what was
//! measured, where it was (the app, the window, the page) and where it was sent. A word is found
//! at the start of one ("butt" finds "button", "tts" finds "local-tts", but "ton" doesn't find
//! "button"), as Mail and Spotlight do; words in quotes are found together, as written.
//!
//! History in the asides window, `asides history <words>` and `asides mcp`'s `list_asides` all
//! search this way.

use std::ops::Range;
use std::path::Path;

use crate::history::{self, Aside};
use crate::markdown::{self, Part};

/// Characters an excerpt keeps before what it's cut around, at most.
const BEFORE: usize = 24;
/// An excerpt's length, at most, in characters.
const EXCERPT: usize = 160;

/// What's looked for: its words, and its phrases in quotes, folded as [`Text`] is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    terms: Vec<String>,
}

/// Text to search, folded once: lowercase, without accents, each run of spaces (and line breaks)
/// one space.
#[derive(Debug, Clone, Default)]
pub struct Text {
    text: String,
    folded: String,
    /// Where each of `folded`'s bytes came from in `text`.
    from: Vec<usize>,
}

/// A line of an aside around a word looked for, and where each word looked for is in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Excerpt {
    pub text: String,
    pub found: Vec<Range<usize>>,
}

impl Query {
    pub fn new(text: &str) -> Query {
        let mut terms: Vec<String> = Vec::new();
        // Between quotes (one left open too, as it's typed), a phrase.
        for (i, part) in text.split('"').enumerate() {
            let words: Vec<&str> = if i % 2 == 1 { vec![part] } else { part.split_whitespace().collect() };
            for word in words {
                let term = Text::new(word).folded.trim_end().to_string();
                if !term.is_empty() && !terms.contains(&term) {
                    terms.push(term);
                }
            }
        }
        Query { terms }
    }

    /// Nothing to look for: everything's found.
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// Whether `text` says every word looked for.
    pub fn matches(&self, text: &Text) -> bool {
        self.terms.iter().all(|term| text.starts(term).next().is_some())
    }

    /// Where the words looked for are in `text` (its bytes), in order, those touching joined.
    pub fn found(&self, text: &Text) -> Vec<Range<usize>> {
        let mut found: Vec<Range<usize>> =
            self.terms.iter().flat_map(|term| text.starts(term).map(|at| text.original(at..at + term.len()))).collect();
        found.sort_by_key(|r| (r.start, r.end));
        let mut joined: Vec<Range<usize>> = Vec::with_capacity(found.len());
        for range in found {
            match joined.last_mut() {
                Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
                _ => joined.push(range),
            }
        }
        joined
    }

    /// The line of `text` where the first word looked for that isn't in `shown` (what a list
    /// already shows of it) is, cut around it; `None` when `shown` has every word.
    pub fn excerpt(&self, text: &Text, shown: &str) -> Option<Excerpt> {
        let shown = Text::new(shown);
        let (at, len) = self
            .terms
            .iter()
            .filter(|term| shown.starts(term).next().is_none())
            .filter_map(|term| text.starts(term).next().map(|at| (at, term.len())))
            .min()?;
        let whole = &text.text;
        let start = text.original(at..at + len).start;
        let line_start = whole[..start].rfind('\n').map_or(0, |i| i + 1);
        let line_end = whole[start..].find('\n').map_or(whole.len(), |i| start + i);
        // A few words before it, from a word's start.
        let before = &whole[line_start..start];
        let mut from = line_start;
        if let Some((cut, _)) = before.char_indices().rev().nth(BEFORE) {
            let cut = cut + before[cut..].chars().next().map_or(0, char::len_utf8);
            from = line_start + before[cut..].find(' ').map_or(cut, |space| cut + space + 1);
        }
        let to = whole[from..line_end].char_indices().nth(EXCERPT).map_or(line_end, |(i, _)| from + i);
        let mut cut = String::new();
        if from > line_start {
            cut.push('…');
        }
        cut.push_str(whole[from..to].trim_end());
        if to < line_end {
            cut.push('…');
        }
        let found = self.found(&Text::new(cut.as_str()));
        Some(Excerpt { text: cut, found })
    }
}

impl Text {
    pub fn new(text: impl Into<String>) -> Text {
        let text = text.into();
        let mut folded = String::with_capacity(text.len());
        let mut from = Vec::with_capacity(text.len());
        for (at, c) in text.char_indices() {
            if c.is_whitespace() {
                if !folded.is_empty() && !folded.ends_with(' ') {
                    folded.push(' ');
                    from.push(at);
                }
                continue;
            }
            for lower in c.to_lowercase() {
                if let Some(plain) = unaccented(lower) {
                    folded.push(plain);
                    from.resize(folded.len(), at);
                }
            }
        }
        Text { text, folded, from }
    }

    /// What's searched in `aside`: its message as History shows it (without its Markdown, its
    /// screenshots, its details' labels or its pins' numbers), a line each, then where it was sent.
    pub fn of(aside: &Aside) -> Text {
        let mut lines: Vec<String> = Vec::new();
        for part in markdown::message_parts(&aside.message()) {
            if let Part::Text(text) = part {
                lines.extend(text.lines().filter_map(searched_line));
            }
        }
        for delivery in &aside.record.deliveries {
            let line = match delivery.app.as_str() {
                "" => delivery.to.clone(),
                app => format!("{} in {app}", delivery.to),
            };
            if !line.is_empty() && !lines.contains(&line) {
                lines.push(line);
            }
        }
        Text::new(lines.join("\n"))
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Where `term` (folded) is in `folded`, at the start of a word: after anything but a letter or
    /// a digit (a term starting with neither, anywhere).
    fn starts<'a>(&'a self, term: &'a str) -> impl Iterator<Item = usize> + 'a {
        let anywhere = !term.starts_with(char::is_alphanumeric);
        self.folded
            .match_indices(term)
            .map(|(at, _)| at)
            .filter(move |&at| anywhere || !self.folded[..at].chars().next_back().is_some_and(char::is_alphanumeric))
    }

    /// Where `range` of `folded` came from in `text`.
    fn original(&self, range: Range<usize>) -> Range<usize> {
        let start = self.from[range.start];
        let last = self.from[range.end - 1];
        start..last + self.text[last..].chars().next().map_or(0, char::len_utf8)
    }
}

/// The asides in `history` that say what `query` looks for, newest first, each with its text.
pub fn search(history: &Path, query: &Query) -> Vec<(Aside, Text)> {
    history::list(history)
        .into_iter()
        .map(|aside| {
            let text = Text::of(&aside);
            (aside, text)
        })
        .filter(|(_, text)| query.matches(text))
        .collect()
}

/// A line of a message as searched: a pin's heading is what it's on, a detail its value, without
/// Markdown or the pins' numbers.
fn searched_line(line: &str) -> Option<String> {
    let line = line.trim();
    let line = match line.strip_prefix("### ") {
        Some(head) => head.split_once(". ").map_or(head, |(_, on)| on),
        None => line,
    };
    let line = line.strip_prefix("- ").unwrap_or(line);
    let line = match line.strip_prefix("**").and_then(|l| l.split_once("**")) {
        Some((label, value)) if label.ends_with(':') => value.trim(),
        _ => line,
    };
    let line = without_pins(&line.replace("**", "").replace('`', ""));
    (!line.is_empty()).then_some(line)
}

/// `this [1], and [2] that` as `this, and that`.
fn without_pins(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(open) = rest.find('[') {
        let (before, after) = rest.split_at(open);
        out.push_str(before);
        let digits = after[1..].find(']').filter(|&n| n > 0 && after[1..1 + n].bytes().all(|b| b.is_ascii_digit()));
        match digits {
            Some(n) => {
                rest = &after[n + 2..];
                if out.ends_with(' ') && (rest.is_empty() || rest.starts_with([' ', ',', '.', ';', ':', '!', '?', ')']))
                {
                    out.pop();
                }
            }
            None => {
                out.push('[');
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

/// A lowercase letter without its accent (`ș` is `s`); `None` for an accent on its own (a
/// combining mark).
fn unaccented(c: char) -> Option<char> {
    const LETTERS: [(&str, char); 19] = [
        ("àáâãäåāăą", 'a'),
        ("çćĉċč", 'c'),
        ("ďđ", 'd'),
        ("èéêëēĕėęě", 'e'),
        ("ĝğġģ", 'g'),
        ("ĥħ", 'h'),
        ("ìíîïĩīĭįı", 'i'),
        ("ĵ", 'j'),
        ("ķ", 'k'),
        ("ĺļľŀł", 'l'),
        ("ñńņňŉ", 'n'),
        ("òóôõöøōŏő", 'o'),
        ("ŕŗř", 'r'),
        ("śŝşšș", 's'),
        ("ţťŧț", 't'),
        ("ùúûüũūŭůűų", 'u'),
        ("ŵ", 'w'),
        ("ýÿŷ", 'y'),
        ("źżž", 'z'),
    ];
    if c.is_ascii() {
        return Some(c);
    }
    if ('\u{300}'..='\u{36f}').contains(&c) {
        return None;
    }
    Some(LETTERS.iter().find(|(accented, _)| accented.contains(c)).map_or(c, |(_, plain)| *plain))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{Delivery, Image};
    use chrono::Local;

    fn finds(query: &str, text: &str) -> bool {
        Query::new(query).matches(&Text::new(text))
    }

    #[test]
    fn finds_every_word_in_any_order_ignoring_case_and_accents() {
        assert!(finds("sterge butonul", "Șterge butonul de jos"));
        assert!(finds("BUTONUL ȘTERGE", "sterge butonul"));
        // Romanian's cedilla and comma letters are one.
        assert!(finds("ş", "ș") && finds("ţară", "țara"));
        // An accent apart (decomposed) too.
        assert!(finds("sterge", "s\u{326}terge"));
        assert!(finds("button save", "Save the button"));
        assert!(!finds("button save", "Save the form"));
        // At a word's start.
        assert!(finds("butt", "the button") && !finds("ton", "the button"));
        assert!(finds("tts", "local-tts") && finds("3000", "localhost:3000") && finds(":3000", "localhost:3000"));
        assert!(finds("ţară", "o ţară") && !finds("ste", "instead"));
        assert!(finds("", "anything") && Query::new("  ").is_empty());
    }

    #[test]
    fn finds_words_in_quotes_together() {
        assert!(!finds("\"save button\"", "the button to save"));
        assert!(finds("\"save button\"", "the Save\n   button"));
        // Still being typed.
        assert!(finds("\"save butt", "the save button"));
        assert!(!finds("\"save butt", "the button to save"));
    }

    #[test]
    fn says_where_the_words_are() {
        let text = Text::new("Ștergeți butonul, șterge tot");
        let found = Query::new("sterge tot").found(&text);
        let words: Vec<&str> = found.iter().map(|r| &text.as_str()[r.clone()]).collect();
        assert_eq!(words, ["Șterge", "șterge", "tot"]);
        // Words touching are one.
        let text = Text::new("button");
        let found = Query::new("butt button").found(&text);
        assert_eq!((found.len(), found[0].clone()), (1, 0..6));
    }

    #[test]
    fn searches_what_history_shows() {
        let tmp = tempfile::tempdir().unwrap();
        let message = "**In:** Safari (\"Shop\", http://localhost:3000)\n\nThe save [1] button is too low.\n\n\
            ![Screenshot 1]({image:0})\n\n### 1. button \"Save\"\n**Feedback:** make it `blue`";
        let png = [0x89, b'P', b'N', b'G'];
        let images = [Image { bytes: &png, extension: "png" }];
        let mut aside = history::create(tmp.path(), Local::now(), message, &images, Vec::new(), None).unwrap();
        let delivery = Delivery { to: "Codex (shop)".into(), app: "Ghostty".into(), ..Default::default() };
        aside.record_delivery(delivery.clone()).unwrap();
        aside.record_delivery(delivery).unwrap();
        let text = Text::of(&aside);
        assert_eq!(
            text.as_str(),
            "Safari (\"Shop\", http://localhost:3000)\nThe save button is too low.\nbutton \"Save\"\nmake it blue\n\
             Codex (shop) in Ghostty"
        );
        let query = |q: &str| Query::new(q).matches(&text);
        assert!(query("localhost:3000") && query("codex ghostty") && query("\"save button\""));
        assert!(!query("png") && !query("screenshot") && !query("feedback"));

        let found = search(tmp.path(), &Query::new("blue safari"));
        assert_eq!(found.len(), 1);
        assert!(search(tmp.path(), &Query::new("green")).is_empty());
    }

    #[test]
    fn cuts_excerpts_around_what_the_list_doesnt_show() {
        let text = Text::new("Save it\nmake the save button blue");
        let query = Query::new("save blue");
        let excerpt = query.excerpt(&text, "Save it").unwrap();
        assert_eq!(excerpt.text, "make the save button blue");
        let words: Vec<&str> = excerpt.found.iter().map(|r| &excerpt.text[r.clone()]).collect();
        assert_eq!(words, ["save", "blue"]);
        assert_eq!(query.excerpt(&text, "Save it blue"), None, "the list shows them");
        assert_eq!(Query::new("green").excerpt(&text, ""), None);

        // A long line: from a word's start a little before it, and at most so long.
        let long = format!("{} the Șterge button {}", "word ".repeat(20), "more ".repeat(60));
        let excerpt = Query::new("sterge").excerpt(&Text::new(long), "").unwrap();
        assert!(excerpt.text.starts_with("…word") && excerpt.text.ends_with('…'), "{}", excerpt.text);
        assert!(excerpt.text.find("Șterge").unwrap() <= BEFORE + 3);
        assert!(excerpt.text.chars().count() <= EXCERPT + 2);
        assert_eq!(&excerpt.text[excerpt.found[0].clone()], "Șterge");
    }

    #[test]
    fn leaves_out_the_pins_numbers() {
        assert_eq!(without_pins("this [1], and [2] that [3]"), "this, and that");
        assert_eq!(without_pins("[1][2] here [x] [] [12a]"), "here [x] [] [12a]");
    }
}
