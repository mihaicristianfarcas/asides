//! Every aside, kept in `~/.asides/history/<id>/`: `aside.json` (when, where it went and whether it
//! got there), `message.md` (exactly what was sent: the transcript with the pins, measurements and
//! screenshots where they were made), `images/` and, when recordings are kept, `audio/`. Nothing is
//! written into the user's repositories. Old asides are pruned (`history_days`).
//!
//! `ASIDES_HOME` moves the whole store (the end-to-end test uses it).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Local, TimeZone};
use serde::{Deserialize, Serialize};

use crate::agents::AgentKind;
use crate::markdown::{self, Part};

pub const HOME_VAR: &str = "ASIDES_HOME";
pub const RECORD_FILE: &str = "aside.json";
pub const MESSAGE_FILE: &str = "message.md";

/// `~/.asides`, or `$ASIDES_HOME`.
pub fn root() -> PathBuf {
    match std::env::var_os(HOME_VAR).filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => std::env::home_dir().expect("HOME is not set").join(".asides"),
    }
}

/// `~/.asides/history`.
pub fn dir() -> PathBuf {
    root().join("history")
}

/// What `aside.json` says.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Record {
    /// Milliseconds since the epoch.
    pub created_ms: u64,
    /// The start of what was said (or of the first annotation), for lists.
    pub title: String,
    /// Its screenshots, relative to its folder (`images/1.png`), in the order of the message.
    pub images: Vec<String>,
    pub audio: Option<String>,
    /// Each time it was sent somewhere, oldest first.
    pub deliveries: Vec<Delivery>,
    /// Its pins' colors, as on its screenshots.
    pub pins: Vec<Pin>,
    /// The version of asides that made it.
    pub version: String,
    /// When its message was last edited by hand (milliseconds since the epoch).
    pub edited_ms: Option<u64>,
}

/// A pin of an aside: its number, and its color (`#3B82F6`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pin {
    pub number: u32,
    pub color: String,
}

/// One sending of an aside to a session.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Delivery {
    pub at_ms: u64,
    /// The session's label when it was sent ("Claude Code (asides)").
    pub to: String,
    pub agent: Option<AgentKind>,
    pub dir: Option<PathBuf>,
    /// The app it was typed into ("Terminal").
    pub app: String,
    /// `None` once it was typed in, or what went wrong.
    pub error: Option<String>,
}

/// An aside in the history.
#[derive(Debug, Clone, PartialEq)]
pub struct Aside {
    pub id: String,
    pub dir: PathBuf,
    pub record: Record,
}

impl Aside {
    pub fn created(&self) -> DateTime<Local> {
        local(self.record.created_ms)
    }

    /// The message, as Markdown with its screenshots as images (`![Screenshot 1](images/1.png)`).
    pub fn message(&self) -> String {
        fs::read_to_string(self.dir.join(MESSAGE_FILE)).unwrap_or_default()
    }

    /// The message to type into a session: text, and its screenshots (by absolute path) where they
    /// were made.
    pub fn parts(&self) -> Vec<Part> {
        markdown::message_parts(&self.message())
            .into_iter()
            .map(|part| match part {
                Part::Image(rel) => Part::Image(self.dir.join(rel).to_string_lossy().into_owned()),
                text => text,
            })
            .collect()
    }

    /// The message as Markdown with its screenshots' absolute paths, for the clipboard.
    pub fn markdown(&self) -> String {
        self.parts()
            .into_iter()
            .map(|part| match part {
                Part::Text(text) => text,
                Part::Image(path) => format!("![Screenshot]({path})"),
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Where it was last sent, and whether that worked.
    pub fn last_delivery(&self) -> Option<&Delivery> {
        self.record.deliveries.last()
    }

    pub fn record_delivery(&mut self, delivery: Delivery) -> io::Result<()> {
        self.record.deliveries.push(delivery);
        write_record(&self.dir, &self.record)
    }

    /// Its screenshots on disk, as its message names them (`images/1.png`).
    pub fn images(&self) -> Vec<String> {
        self.record.images.iter().filter(|i| self.dir.join(i).is_file()).cloned().collect()
    }

    /// What's wrong with `message` as its message (`markdown::check_message`).
    pub fn check(&self, message: &str) -> Vec<markdown::Problem> {
        markdown::check_message(message, &self.images())
    }

    /// Saves `message`, edited by hand, as its message, with its title again and when: refused
    /// when it doesn't check. Whole or not at all, like its record (read again: a delivery noted
    /// meanwhile stays).
    pub fn save_message(&mut self, message: &str) -> io::Result<()> {
        let message = message.trim_end();
        if let Some(problem) = self.check(message).first() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("line {}: {}", problem.line, problem.text)));
        }
        let tmp = self.dir.join(format!("{MESSAGE_FILE}.tmp"));
        fs::write(&tmp, message)?;
        fs::rename(tmp, self.dir.join(MESSAGE_FILE))?;
        if let Some(now) = read(&self.dir) {
            self.record = now.record;
        }
        self.record.title = title_of(message);
        self.record.edited_ms = Some(now_ms());
        write_record(&self.dir, &self.record)
    }
}

/// An image to keep with an aside.
pub struct Image<'a> {
    pub bytes: &'a [u8],
    pub extension: &'a str,
}

fn local(ms: u64) -> DateTime<Local> {
    Local.timestamp_millis_opt(ms as i64).single().unwrap_or_else(Local::now)
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Keeps a new aside in `history`: its message (Markdown whose images are `images[i]`, written as
/// `images/<i+1>.<ext>`, referenced by `{image:i}`), its pins' colors and its recording.
pub fn create(
    history: &Path,
    created: DateTime<Local>,
    message: &str,
    images: &[Image],
    pins: Vec<Pin>,
    audio: Option<(&[u8], &str)>,
) -> io::Result<Aside> {
    fs::create_dir_all(history)?;
    let stem = created.format("%Y-%m-%d-%H%M%S").to_string();
    let (id, dir) = (1..)
        .map(|n| if n == 1 { stem.clone() } else { format!("{stem}-{n}") })
        .map(|id| (id.clone(), history.join(id)))
        .find(|(_, dir)| !dir.exists())
        .expect("a free name");
    fs::create_dir(&dir)?;
    let mut names = Vec::new();
    if !images.is_empty() {
        fs::create_dir(dir.join("images"))?;
    }
    for (i, image) in images.iter().enumerate() {
        let name = format!("images/{}.{}", i + 1, image.extension);
        fs::write(dir.join(&name), image.bytes)?;
        names.push(name);
    }
    let mut text = message.to_string();
    for (i, name) in names.iter().enumerate() {
        text = text.replace(&format!("{{image:{i}}}"), name);
    }
    let audio = match audio {
        Some((bytes, extension)) => {
            fs::create_dir(dir.join("audio"))?;
            let name = format!("audio/aside.{extension}");
            fs::write(dir.join(&name), bytes)?;
            Some(name)
        }
        None => None,
    };
    fs::write(dir.join(MESSAGE_FILE), &text)?;
    let record = Record {
        created_ms: created.timestamp_millis().max(0) as u64,
        title: title_of(&text),
        images: names,
        audio,
        deliveries: Vec::new(),
        pins,
        version: env!("CARGO_PKG_VERSION").into(),
        edited_ms: None,
    };
    write_record(&dir, &record)?;
    Ok(Aside { id, dir, record })
}

fn write_record(dir: &Path, record: &Record) -> io::Result<()> {
    let json = serde_json::to_string_pretty(record).map_err(io::Error::other)?;
    // Whole or not at all: a reader never sees half a record.
    let tmp = dir.join(format!("{RECORD_FILE}.tmp"));
    fs::write(&tmp, json)?;
    fs::rename(tmp, dir.join(RECORD_FILE))
}

/// The first words of a message, on one line: what was said, else the first annotation's heading.
pub fn title_of(message: &str) -> String {
    let line = message
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("![") && !l.starts_with("**In:**"))
        .unwrap_or("Screenshot")
        .trim_start_matches(['#', '-', ' ']);
    let line = line.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.').trim();
    // Without the pins' numbers (`this [1],` reads `this,`).
    let mut words: Vec<String> = Vec::new();
    for word in line.split_whitespace() {
        let pin = word
            .strip_prefix('[')
            .and_then(|w| w.split_once(']'))
            .filter(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
        match (pin, words.last_mut()) {
            (Some((_, tail)), Some(last)) => last.push_str(tail),
            (Some((_, tail)), None) if !tail.is_empty() => words.push(tail.to_string()),
            (Some(_), None) => {}
            (None, _) => words.push(word.to_string()),
        }
    }
    let words = words.join(" ");
    if words.chars().count() <= 80 { words } else { format!("{}…", words.chars().take(79).collect::<String>()) }
}

/// Reads the aside in `dir`.
pub fn read(dir: &Path) -> Option<Aside> {
    let record: Record = serde_json::from_str(&fs::read_to_string(dir.join(RECORD_FILE)).ok()?).ok()?;
    let id = dir.file_name()?.to_string_lossy().into_owned();
    Some(Aside { id, dir: dir.to_path_buf(), record })
}

/// Every aside in `history`, newest first.
pub fn list(history: &Path) -> Vec<Aside> {
    let mut asides: Vec<Aside> = fs::read_dir(history)
        .map(|entries| entries.flatten().filter_map(|e| read(&e.path())).collect())
        .unwrap_or_default();
    asides.sort_by(|a, b| b.record.created_ms.cmp(&a.record.created_ms).then(b.id.cmp(&a.id)));
    asides
}

/// The aside `id`, or the newest for `last` (or an empty id).
pub fn find(history: &Path, id: &str) -> Option<Aside> {
    match id.trim() {
        "" | "last" | "latest" => list(history).into_iter().next(),
        id => read(&history.join(id)),
    }
}

/// Removes asides older than `days` (0 keeps them all); how many went.
pub fn prune(history: &Path, days: u32, now: SystemTime) -> usize {
    if days == 0 {
        return 0;
    }
    let cutoff = now.checked_sub(Duration::from_secs(u64::from(days) * 86_400)).unwrap_or(UNIX_EPOCH);
    let cutoff_ms = cutoff.duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    list(history)
        .into_iter()
        .filter(|a| a.record.created_ms < cutoff_ms)
        .filter(|a| fs::remove_dir_all(&a.dir).is_ok())
        .count()
}

/// Removes an aside.
pub fn delete(aside: &Aside) -> io::Result<()> {
    fs::remove_dir_all(&aside.dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_asides_with_their_screenshots_and_deliveries() {
        let tmp = tempfile::tempdir().unwrap();
        let at = Local.with_ymd_and_hms(2026, 10, 18, 22, 15, 30).unwrap();
        let message = "The save button is too low.\n\n![Screenshot 1]({image:0})\n\n### 1. button \"Save\"";
        let png = [0x89, b'P', b'N', b'G'];
        let mut aside =
            create(tmp.path(), at, message, &[Image { bytes: &png, extension: "png" }], Vec::new(), None).unwrap();
        assert_eq!(aside.id, "2026-10-18-221530");
        assert_eq!(aside.record.images, ["images/1.png"]);
        assert_eq!(aside.record.title, "The save button is too low.");
        assert!(aside.message().contains("![Screenshot 1](images/1.png)"));
        assert_eq!(fs::read(aside.dir.join("images/1.png")).unwrap(), png);

        let parts = aside.parts();
        assert_eq!(parts[0], Part::Text("The save button is too low.".into()));
        assert_eq!(parts[1], Part::Image(aside.dir.join("images/1.png").to_string_lossy().into_owned()));

        let delivery =
            Delivery { at_ms: 1, to: "Claude Code (shop)".into(), app: "Terminal".into(), ..Default::default() };
        aside.record_delivery(delivery.clone()).unwrap();
        let again = find(tmp.path(), "last").unwrap();
        assert_eq!(again.record.deliveries, [delivery]);

        // Two in the same second get their own folders.
        let second = create(tmp.path(), at, "Another.", &[], Vec::new(), None).unwrap();
        assert_eq!(second.id, "2026-10-18-221530-2");
        assert_eq!(list(tmp.path()).len(), 2);
        assert_eq!(
            find(tmp.path(), "2026-10-18-221530").map(|a| a.record.title),
            Some("The save button is too low.".into())
        );
    }

    #[test]
    fn saves_messages_edited_by_hand() {
        let tmp = tempfile::tempdir().unwrap();
        let png = [0x89, b'P', b'N', b'G'];
        let message = "Too low [1].\n\n![Screenshot 1]({image:0})\n\n### 1. button \"Save\"";
        let images = [Image { bytes: &png, extension: "png" }];
        let mut aside = create(tmp.path(), Local::now(), message, &images, Vec::new(), None).unwrap();
        let delivery = Delivery { at_ms: 1, to: "Codex (shop)".into(), ..Default::default() };
        // Noted by the app meanwhile: it stays.
        find(tmp.path(), &aside.id).unwrap().record_delivery(delivery.clone()).unwrap();

        let edited = "Much too low [1].\n\n![Screenshot 1](images/1.png)\n\n### 1. button \"Save\"\n\n";
        aside.save_message(edited).unwrap();
        let again = find(tmp.path(), &aside.id).unwrap();
        assert_eq!(again.message(), edited.trim_end());
        assert_eq!(again.record.title, "Much too low.");
        assert_eq!(again.record.deliveries, [delivery]);
        assert!(again.record.edited_ms.is_some());

        // What doesn't check isn't saved.
        let err = aside.save_message("Gone [2].").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(find(tmp.path(), &aside.id).unwrap().message(), edited.trim_end());
        assert_eq!(aside.images(), ["images/1.png"]);
    }

    #[test]
    fn prunes_old_asides() {
        let tmp = tempfile::tempdir().unwrap();
        let old = Local::now() - chrono::TimeDelta::days(40);
        create(tmp.path(), old, "Old.", &[], Vec::new(), None).unwrap();
        create(tmp.path(), Local::now(), "New.", &[], Vec::new(), None).unwrap();
        assert_eq!(prune(tmp.path(), 0, SystemTime::now()), 0, "0 keeps everything");
        assert_eq!(prune(tmp.path(), 30, SystemTime::now()), 1);
        let left: Vec<String> = list(tmp.path()).into_iter().map(|a| a.record.title).collect();
        assert_eq!(left, ["New."]);
    }

    #[test]
    fn titles_say_what_was_said_first() {
        assert_eq!(
            title_of("**In:** Safari\n\nThe card needs padding. And more."),
            "The card needs padding. And more."
        );
        assert_eq!(title_of("![Screenshot 1](images/1.png)\n\n### 1. button \"Pay\""), "button \"Pay\"");
        assert_eq!(title_of(""), "Screenshot");
        assert_eq!(title_of(&"word ".repeat(40)).chars().count(), 80);
        assert_eq!(title_of("[1] This [2] and this [3]."), "This and this.");
    }
}
