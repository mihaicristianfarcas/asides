//! What happened to each aside after it was written, as in Agentation's protocol: an agent
//! acknowledges it, resolves it with a summary, dismisses it with a reason, or replies to it (a
//! clarifying question, an update); the user can reply or reopen it too. Kept beside the note in
//! `threads.jsonl`, one event per line, only ever appended to, like the note itself: several
//! agents (and the app) can add to it at once without coordinating, and notes.md stays untouched.
//!
//! An aside is an entry (`3`) or one annotation in it (`3.2`, the `### 2.` of entry 3).

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

pub const THREADS_FILE: &str = "threads.jsonl";

/// Entry `entry`, or annotation `part` of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AsideId {
    pub entry: u32,
    pub part: Option<u32>,
}

impl AsideId {
    pub fn entry(entry: u32) -> AsideId {
        AsideId { entry, part: None }
    }
}

impl fmt::Display for AsideId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.part {
            Some(part) => write!(f, "{}.{part}", self.entry),
            None => write!(f, "{}", self.entry),
        }
    }
}

impl FromStr for AsideId {
    type Err = String;

    /// `3`, `#3`, `3.2` or `#3.2`.
    fn from_str(s: &str) -> Result<AsideId, String> {
        let bad =
            || format!("\"{s}\" isn't an aside number: use the entry's number, like 3, or 3.2 for annotation 2 of it");
        let s = s.trim().trim_start_matches('#');
        let (entry, part) = match s.split_once('.') {
            Some((entry, part)) => (entry, Some(part.parse::<u32>().map_err(|_| bad())?)),
            None => (s, None),
        };
        let entry = entry.parse::<u32>().map_err(|_| bad())?;
        if entry == 0 || part == Some(0) {
            return Err(bad());
        }
        Ok(AsideId { entry, part })
    }
}

impl Serialize for AsideId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for AsideId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<AsideId, D::Error> {
        String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Nobody has acted on it yet.
    #[default]
    Pending,
    /// An agent has seen it and is on it.
    Acknowledged,
    Resolved,
    /// An agent decided not to act on it (with a reason), or the user withdrew it.
    Dismissed,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Pending => "pending",
            Status::Acknowledged => "acknowledged",
            Status::Resolved => "resolved",
            Status::Dismissed => "dismissed",
        }
    }

    /// Still waiting to be handled: pending or acknowledged.
    pub fn is_open(self) -> bool {
        matches!(self, Status::Pending | Status::Acknowledged)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Human,
    Agent,
}

/// One line of `threads.jsonl`: a status change, a message, or both.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub aside: AsideId,
    pub at_ms: u64,
    pub by: Role,
    /// Who: the agent's name ("Claude Code"); for the user, where they acted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Message {
    pub by: Role,
    pub author: Option<String>,
    pub text: String,
    pub at_ms: u64,
    /// The status this message came with (a resolution's summary, a dismissal's reason).
    pub status: Option<Status>,
}

/// An aside's current state, from all its events.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Thread {
    pub status: Status,
    pub updated_ms: u64,
    /// When it was resolved or dismissed, and by whom.
    pub closed: Option<(u64, Role)>,
    pub messages: Vec<Message>,
}

/// Appends `event` to `note_dir`'s threads in a single write, so concurrent writers never
/// interleave within a line.
pub fn append(note_dir: &Path, event: &Event) -> io::Result<()> {
    let mut line = serde_json::to_string(event).map_err(io::Error::other)?;
    line.push('\n');
    let mut file = OpenOptions::new().create(true).append(true).open(note_dir.join(THREADS_FILE))?;
    file.write_all(line.as_bytes())
}

/// Every event recorded for `note_dir`, oldest first. Lines that don't parse are skipped.
pub fn read(note_dir: &Path) -> Vec<Event> {
    let Ok(text) = fs::read_to_string(note_dir.join(THREADS_FILE)) else { return Vec::new() };
    text.lines().filter_map(|line| serde_json::from_str(line).ok()).collect()
}

/// The state of every aside that has events.
pub fn fold(events: &[Event]) -> BTreeMap<AsideId, Thread> {
    let mut threads: BTreeMap<AsideId, Thread> = BTreeMap::new();
    for event in events {
        let thread = threads.entry(event.aside).or_default();
        thread.updated_ms = thread.updated_ms.max(event.at_ms);
        if let Some(status) = event.status {
            thread.status = status;
            thread.closed = (!status.is_open()).then_some((event.at_ms, event.by));
        }
        if let Some(text) = event.message.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            thread.messages.push(Message {
                by: event.by,
                author: event.author.clone(),
                text: text.to_string(),
                at_ms: event.at_ms,
                status: event.status,
            });
        }
    }
    threads
}

/// The state of an aside; an annotation without events of its own takes its entry's status
/// (resolving entry 3 resolves its annotations too).
pub fn status_of(threads: &BTreeMap<AsideId, Thread>, id: AsideId) -> Status {
    let own = threads.get(&id);
    match (own, id.part) {
        (Some(thread), _) => thread.status,
        (None, Some(_)) => threads.get(&AsideId::entry(id.entry)).map(|t| t.status).unwrap_or_default(),
        (None, None) => Status::Pending,
    }
}

/// An entry's status: an entry with annotations is open while any of them is, and done once
/// they're all resolved or dismissed.
pub fn entry_status(threads: &BTreeMap<AsideId, Thread>, entry: &crate::notes::Entry) -> Status {
    let annotations = parts(&entry.body);
    if annotations.is_empty() {
        return status_of(threads, AsideId::entry(entry.number));
    }
    let statuses: Vec<Status> =
        annotations.iter().map(|&p| status_of(threads, AsideId { entry: entry.number, part: Some(p) })).collect();
    match statuses.iter().find(|s| s.is_open()) {
        Some(open) => *open,
        None if statuses.iter().all(|s| *s == Status::Dismissed) => Status::Dismissed,
        None => Status::Resolved,
    }
}

/// How many of a note's entries are still open.
pub fn open_count(note_dir: &Path, entries: &[crate::notes::Entry]) -> usize {
    let threads = fold(&read(note_dir));
    entries.iter().filter(|e| entry_status(&threads, e).is_open()).count()
}

/// The thread as the agent reads it, quoted under the aside: `> Claude Code (resolved): …`.
pub fn render(thread: &Thread) -> String {
    let mut lines = Vec::new();
    for message in &thread.messages {
        let who = match (&message.author, message.by) {
            (Some(author), _) => author.clone(),
            (None, Role::Agent) => "Agent".into(),
            (None, Role::Human) => "User".into(),
        };
        let status = message.status.map(|s| format!(" ({})", s.as_str())).unwrap_or_default();
        let text = message.text.replace('\n', "\n> ");
        lines.push(format!("> **{who}**{status}: {text}"));
    }
    if lines.is_empty() || thread.messages.last().and_then(|m| m.status) != Some(thread.status) {
        lines.push(format!("> Status: {}", thread.status.as_str()));
    }
    lines.join("\n")
}

/// The annotations (`### N.`) an entry body holds, by number.
pub fn parts(body: &str) -> Vec<u32> {
    body.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("### ")?;
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            rest[digits.len()..].starts_with(". ").then(|| digits.parse().ok()).flatten()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(aside: &str, at_ms: u64, by: Role, status: Option<Status>, message: Option<&str>) -> Event {
        Event {
            aside: aside.parse().unwrap(),
            at_ms,
            by,
            author: (by == Role::Agent).then(|| "Claude Code".into()),
            status,
            message: message.map(Into::into),
        }
    }

    #[test]
    fn parses_aside_ids() {
        assert_eq!("3".parse::<AsideId>(), Ok(AsideId::entry(3)));
        assert_eq!(" #12.4".parse::<AsideId>(), Ok(AsideId { entry: 12, part: Some(4) }));
        assert!("0".parse::<AsideId>().is_err());
        assert!("3.x".parse::<AsideId>().is_err());
        assert!("abc".parse::<AsideId>().unwrap_err().contains("isn't an aside number"));
        assert_eq!(AsideId { entry: 3, part: Some(2) }.to_string(), "3.2");
    }

    #[test]
    fn appends_and_folds_events() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(read(tmp.path()).is_empty());
        let events = [
            event("3", 1, Role::Agent, Some(Status::Acknowledged), None),
            event("3", 2, Role::Agent, None, Some("Should it match the header's blue?")),
            event("3", 3, Role::Human, None, Some("Yes")),
            event("3", 4, Role::Agent, Some(Status::Resolved), Some("Used the header's blue in Button.tsx")),
            event("4.1", 5, Role::Agent, Some(Status::Dismissed), Some("That's the browser's own UI")),
        ];
        for e in &events {
            append(tmp.path(), e).unwrap();
        }
        // A torn or foreign line doesn't hide the rest.
        fs::OpenOptions::new().append(true).open(tmp.path().join(THREADS_FILE)).unwrap().write_all(b"{oops\n").unwrap();
        let read_back = read(tmp.path());
        assert_eq!(read_back, events);

        let threads = fold(&read_back);
        let three = &threads[&AsideId::entry(3)];
        assert_eq!((three.status, three.closed, three.updated_ms), (Status::Resolved, Some((4, Role::Agent)), 4));
        assert_eq!(three.messages.len(), 3);
        assert_eq!(status_of(&threads, "3.2".parse().unwrap()), Status::Resolved);
        assert_eq!(status_of(&threads, "4".parse().unwrap()), Status::Pending);
        assert_eq!(status_of(&threads, "4.1".parse().unwrap()), Status::Dismissed);
        assert_eq!(
            render(three),
            "> **Claude Code**: Should it match the header's blue?\n> **User**: Yes\n\
             > **Claude Code** (resolved): Used the header's blue in Button.tsx"
        );
        let reopened = fold(&[
            event("5", 1, Role::Agent, Some(Status::Resolved), None),
            event("5", 2, Role::Human, Some(Status::Pending), None),
        ]);
        assert_eq!(reopened[&AsideId::entry(5)].closed, None);
        assert_eq!(render(&reopened[&AsideId::entry(5)]), "> Status: pending");
    }

    #[test]
    fn finds_annotations_in_an_entry() {
        let body = "Said first.\n\n### 1. button \"Save\"\n**Feedback:** x\n\n### 2. Area 10 × 10 at (0, 0)\n### 3 nope\n### x. no";
        assert_eq!(parts(body), [1, 2]);
    }
}
