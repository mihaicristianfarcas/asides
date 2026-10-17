//! Handing entries to coding agents through `asides mcp`: what a listening agent has already
//! seen, what's new since, and the listener records the app shows in its menu.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::AgentDelivery;
use crate::notes::{self, Entry, Note};
use crate::routes::{self, Route};
use crate::state::AgentSend;

/// How far an agent has read: entry `entry` of `note` (a note folder), of which the first `len`
/// bytes of the body. `note: None` means before the first note.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Cursor {
    pub note: Option<PathBuf>,
    pub entry: u32,
    pub len: usize,
}

/// Something an agent hasn't seen yet.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// A new entry.
    Entry { note: Note, entry: Entry },
    /// Text added to an entry the agent already has, such as an image attached after it.
    Addition { note: Note, number: u32, text: String },
}

impl Cursor {
    /// The end of `base`'s notes as they are now: only entries written later are new.
    pub fn at_end(base: &Path) -> Cursor {
        let Some(note) = notes::latest_note(base) else { return Cursor::default() };
        let last = fs::read_to_string(note.file()).ok().and_then(|c| notes::parse_entries(&c).pop());
        Cursor {
            entry: last.as_ref().map_or(0, |e| e.number),
            len: last.map_or(0, |e| e.body.len()),
            note: Some(note.dir),
        }
    }

    /// Before the first entry of `base`'s latest note.
    pub fn at_start(base: &Path) -> Cursor {
        Cursor { note: notes::latest_note(base).map(|n| n.dir), ..Cursor::default() }
    }

    /// Just after entry `number` of the note in `note_dir`, as it is now.
    pub fn after_entry(note_dir: &Path, number: u32) -> Cursor {
        let file = note_dir.join(crate::paths::NOTE_FILE);
        let entries = fs::read_to_string(file).map(|c| notes::parse_entries(&c)).unwrap_or_default();
        let len = entries.iter().find(|e| e.number == number).map_or(0, |e| e.body.len());
        Cursor { note: Some(note_dir.to_path_buf()), entry: number, len }
    }

    /// Moves past `item`.
    pub fn advance(&mut self, item: &Item) {
        match item {
            Item::Entry { note, entry } => {
                *self = Cursor { note: Some(note.dir.clone()), entry: entry.number, len: entry.body.len() };
            }
            Item::Addition { text, .. } => self.len += text.len(),
        }
    }
}

/// One agent listening in `dir`: what it has been given, and the last Send to Agent it acted on.
/// `asides mcp` and `asides listen` both deliver through it.
#[derive(Debug, Clone, PartialEq)]
pub struct Listening {
    pub dir: PathBuf,
    /// The session it listens as (`sessions::Session`): it gets the entries routed to it.
    pub session: Option<String>,
    pub cursor: Cursor,
    /// Sends up to this one (`AgentSend::at_ms`) aren't for this agent: they came before it
    /// started listening, or it already has them.
    pub last_send: u64,
}

impl Listening {
    /// Starts listening in `dir` now, given the last Send to Agent in each directory (newest
    /// first). Sends made earlier are ignored: the user chose them before the agent listened. What
    /// they would have carried isn't lost, though: in batch mode the next send includes everything
    /// since the last send in `dir` (or, without one, the latest note). In each mode, only asides
    /// recorded from now on are new.
    pub fn start(dir: &Path, session: Option<&str>, delivery: AgentDelivery, sends: &[AgentSend]) -> Listening {
        let cursor = match delivery {
            AgentDelivery::Each => Cursor::at_end(dir),
            AgentDelivery::Batch => match sends.iter().find(|s| s.dir == dir) {
                Some(AgentSend { note: Some(note), entry, .. }) => Cursor::after_entry(note, *entry),
                _ => Cursor::at_start(dir),
            },
        };
        Listening {
            dir: dir.to_path_buf(),
            session: session.map(Into::into),
            cursor,
            last_send: sends.first().map_or(0, |s| s.at_ms),
        }
    }

    /// What's ready for the agent now, and moves past it: in each mode, everything new that was
    /// spoken to its session (`routes`); in batch mode, the entries up to a Send to Agent made
    /// since the last one it acted on, when that send was to its session. Either way it moves past
    /// what went to other sessions, so it never gets those later.
    pub fn take(&mut self, delivery: AgentDelivery, send: Option<&AgentSend>) -> Vec<Item> {
        let (items, mine) = match delivery {
            AgentDelivery::Each => (collect(&self.dir, &self.cursor, None), None),
            AgentDelivery::Batch => match send.filter(|s| s.at_ms > self.last_send && s.dir == self.dir) {
                Some(send) => {
                    self.last_send = send.at_ms;
                    let items = match &send.note {
                        Some(note) => collect(&self.dir, &self.cursor, Some((note, send.entry))),
                        None => Vec::new(),
                    };
                    // A batch goes to the session it was sent to, whoever each aside was spoken to.
                    let mine = send.session.is_none() || send.session == self.session;
                    (items, Some(mine))
                }
                None => (Vec::new(), None),
            },
        };
        for item in &items {
            self.cursor.advance(item);
        }
        match mine {
            Some(true) => items,
            Some(false) => Vec::new(),
            None => self.routed_here(items),
        }
    }

    /// The items spoken to this agent's session (or to nobody in particular).
    fn routed_here(&self, items: Vec<Item>) -> Vec<Item> {
        let mut routes: Vec<(PathBuf, std::collections::BTreeMap<u32, Route>)> = Vec::new();
        items
            .into_iter()
            .filter(|item| {
                let (note, number) = match item {
                    Item::Entry { note, entry } => (note, entry.number),
                    Item::Addition { note, number, .. } => (note, *number),
                };
                if !routes.iter().any(|(dir, _)| *dir == note.dir) {
                    routes.push((note.dir.clone(), routes::read(&note.dir)));
                }
                let found = routes.iter().find(|(dir, _)| *dir == note.dir).and_then(|(_, r)| r.get(&number));
                Route::reaches(found, self.session.as_deref())
            })
            .collect()
    }

    /// Where this agent is, as text, to carry on later with `resume` (`asides listen --after`).
    pub fn token(&self) -> String {
        let note = self.cursor.note.as_ref().and_then(|n| n.file_name()).map(|n| n.to_string_lossy().into_owned());
        format!("{}:{}:{}:{}", self.last_send, self.cursor.entry, self.cursor.len, note.unwrap_or_default())
    }

    /// Listening in `dir` as `session` from where `token` left off; `None` if it isn't a token.
    pub fn resume(dir: &Path, session: Option<&str>, token: &str) -> Option<Listening> {
        let mut parts = token.splitn(4, ':');
        let last_send = parts.next()?.parse().ok()?;
        let entry = parts.next()?.parse().ok()?;
        let len = parts.next()?.parse().ok()?;
        let note = parts.next()?;
        let note = (!note.is_empty()).then(|| notes::notes_root(dir).join(note));
        Some(Listening {
            dir: dir.to_path_buf(),
            session: session.map(Into::into),
            cursor: Cursor { note, entry, len },
            last_send,
        })
    }
}

/// What `base`'s notes hold beyond `cursor`, oldest first. With `until` (a note folder and an
/// entry number), entries after that point are left for later.
pub fn collect(base: &Path, cursor: &Cursor, until: Option<(&Path, u32)>) -> Vec<Item> {
    let all = notes::list_notes(base);
    let start = match &cursor.note {
        None => 0,
        Some(dir) => match all.iter().position(|n| &n.dir == dir) {
            Some(i) => i,
            // The note is gone (deleted or renamed): carry on with the notes named after it.
            None => all.iter().position(|n| n.dir.file_name() > dir.file_name()).unwrap_or(all.len()),
        },
    };
    let stop = until.and_then(|(dir, _)| all.iter().position(|n| n.dir == dir));
    let mut items = Vec::new();
    for (i, note) in all.iter().enumerate().skip(start) {
        if stop.is_some_and(|stop| i > stop) {
            break;
        }
        let Ok(content) = fs::read_to_string(note.file()) else { continue };
        let same = cursor.note.as_ref() == Some(&note.dir);
        let last = until.filter(|_| stop == Some(i)).map(|(_, entry)| entry);
        for entry in notes::parse_entries(&content) {
            if same && entry.number < cursor.entry {
                continue;
            }
            if same && entry.number == cursor.entry {
                let added = entry.body.get(cursor.len..).map(str::trim).unwrap_or_default();
                if entry.body.len() > cursor.len && !added.is_empty() {
                    let text = entry.body[cursor.len..].to_string();
                    items.push(Item::Addition { note: note.clone(), number: entry.number, text });
                }
                continue;
            }
            if last.is_some_and(|last| entry.number > last) {
                break;
            }
            items.push(Item::Entry { note: note.clone(), entry });
        }
    }
    items
}

/// Entry text as an agent reads it: links to the note's images and audio made absolute (in angle
/// brackets, since note folders have spaces in their names), so the agent can open them.
pub fn absolute_links(note_dir: &Path, text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("](") {
        out.push_str(&rest[..i + 2]);
        rest = &rest[i + 2..];
        let Some(end) = rest.find(')') else { break };
        let target = &rest[..end];
        if is_note_relative(target) {
            out.push_str(&format!("<{}>", note_dir.join(target).display()));
            rest = &rest[end..];
        }
    }
    out.push_str(rest);
    out
}

/// The images an entry shows, as absolute paths, in order.
pub fn image_paths(note_dir: &Path, text: &str) -> Vec<PathBuf> {
    text.match_indices("![")
        .filter_map(|(i, _)| {
            let after = &text[i..];
            let start = after.find("](")? + 2;
            let end = start + after[start..].find(')')?;
            let target = &after[start..end];
            is_note_relative(target).then(|| note_dir.join(target))
        })
        .collect()
}

fn is_note_relative(target: &str) -> bool {
    (target.starts_with("images/") || target.starts_with("audio/")) && !target.contains("..")
}

/// An agent session that listens through `asides mcp`, as the app shows it in its menu.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Listener {
    pub pid: u32,
    /// The agent's name, e.g. "Claude Code".
    pub client: String,
    /// The session it delivers to (`sessions::Session`).
    pub session: String,
    /// The bound directory it listens in.
    pub dir: PathBuf,
    /// Waiting for asides right now; otherwise working on the last ones it got.
    pub waiting: bool,
    pub updated_ms: u64,
}

impl Listener {
    pub fn file(listeners: &Path, pid: u32) -> PathBuf {
        listeners.join(format!("{pid}.json"))
    }

    /// Atomic write (temp file + rename), like `state.json`.
    pub fn write(&self, listeners: &Path) -> io::Result<()> {
        fs::create_dir_all(listeners)?;
        let path = Listener::file(listeners, self.pid);
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec(self).map_err(io::Error::other)?)?;
        fs::rename(tmp, path)
    }

    /// Every listener record in `listeners`, including those of processes that have since died.
    pub fn read_all(listeners: &Path) -> Vec<Listener> {
        let Ok(files) = fs::read_dir(listeners) else { return Vec::new() };
        let mut all: Vec<Listener> = files
            .flatten()
            .filter(|f| f.path().extension().is_some_and(|e| e == "json"))
            .filter_map(|f| serde_json::from_str(&fs::read_to_string(f.path()).ok()?).ok())
            .collect();
        all.sort_by_key(|l| l.pid);
        all
    }
}

/// A friendlier name for an MCP client: `claude-code` → "Claude Code".
pub fn client_label(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if lower.contains("claude") {
        "Claude Code".into()
    } else if lower.contains("codex") {
        "Codex".into()
    } else if lower.contains("opencode") {
        "OpenCode".into()
    } else if name.trim().is_empty() {
        "Agent".into()
    } else {
        name.trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone};

    fn note(base: &Path, minute: u32, title: &str) -> Note {
        let at = Local.with_ymd_and_hms(2026, 10, 15, 14, minute, 0).unwrap();
        notes::create_note(base, at, title, "---\ntitle: x\n---").unwrap()
    }

    fn numbers(items: &[Item]) -> Vec<String> {
        items
            .iter()
            .map(|item| match item {
                Item::Entry { note, entry } => format!("{} #{}", note.title, entry.number),
                Item::Addition { note, number, text } => format!("{} #{number} +{}", note.title, text.trim()),
            })
            .collect()
    }

    #[test]
    fn collects_new_entries_and_additions() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(Cursor::at_end(tmp.path()), Cursor::default());
        let a = note(tmp.path(), 30, "A");
        a.append("## 1 · 14:30\n\nOld.").unwrap();
        let mut cursor = Cursor::at_end(tmp.path());
        assert_eq!(cursor, Cursor { note: Some(a.dir.clone()), entry: 1, len: 4 });
        assert!(collect(tmp.path(), &cursor, None).is_empty());

        a.append("![14:31:00](images/01-143100.png)").unwrap();
        a.append("## 2 · 14:32\n\nNew.").unwrap();
        let b = note(tmp.path(), 40, "B");
        b.append("## 1 · 14:40\n\nOther note.").unwrap();
        let items = collect(tmp.path(), &cursor, None);
        assert_eq!(numbers(&items), ["A #1 +![14:31:00](images/01-143100.png)", "A #2", "B #1"]);

        for item in &items {
            cursor.advance(item);
        }
        assert!(collect(tmp.path(), &cursor, None).is_empty());
        b.append("## 2 · 14:41\n\nLatest.").unwrap();
        assert_eq!(numbers(&collect(tmp.path(), &cursor, None)), ["B #2"]);
    }

    #[test]
    fn stops_at_the_batch_end() {
        let tmp = tempfile::tempdir().unwrap();
        let a = note(tmp.path(), 30, "A");
        let cursor = Cursor::at_end(tmp.path());
        a.append("## 1 · 14:30\n\nOne.").unwrap();
        a.append("## 2 · 14:31\n\nTwo.").unwrap();
        let b = note(tmp.path(), 40, "B");
        b.append("## 1 · 14:40\n\nThree.").unwrap();
        assert_eq!(numbers(&collect(tmp.path(), &cursor, Some((&a.dir, 1)))), ["A #1"]);
        assert_eq!(numbers(&collect(tmp.path(), &cursor, Some((&a.dir, 2)))), ["A #1", "A #2"]);
        assert_eq!(numbers(&collect(tmp.path(), &cursor, Some((&b.dir, 1)))), ["A #1", "A #2", "B #1"]);
    }

    fn send(at_ms: u64, dir: &Path, note: &Note, entry: u32) -> AgentSend {
        AgentSend { at_ms, dir: dir.to_path_buf(), note: Some(note.dir.clone()), entry, session: None }
    }

    #[test]
    fn a_send_made_before_listening_is_not_delivered() {
        let tmp = tempfile::tempdir().unwrap();
        let a = note(tmp.path(), 30, "A");
        a.append("## 1 · 14:30\n\nOne.").unwrap();
        a.append("## 2 · 14:31\n\nTwo.").unwrap();
        let early = send(100, tmp.path(), &a, 2);
        let mut listening = Listening::start(tmp.path(), None, AgentDelivery::Batch, std::slice::from_ref(&early));
        assert!(listening.take(AgentDelivery::Batch, Some(&early)).is_empty());
        // Without an earlier send here, the next one carries the latest note so far, even when the
        // newest send was made elsewhere.
        let elsewhere = AgentSend { at_ms: 50, dir: "/elsewhere".into(), ..early.clone() };
        let mut fresh = Listening::start(tmp.path(), None, AgentDelivery::Batch, std::slice::from_ref(&elsewhere));
        assert_eq!(numbers(&fresh.take(AgentDelivery::Batch, Some(&early))), ["A #1", "A #2"]);
        // With one here, after it, even when a send elsewhere is newer.
        a.append("## 3 · 14:32\n\nThree.").unwrap();
        let newer_elsewhere = AgentSend { at_ms: 150, ..elsewhere.clone() };
        let mut back = Listening::start(tmp.path(), None, AgentDelivery::Batch, &[newer_elsewhere, early.clone()]);
        assert_eq!(numbers(&back.take(AgentDelivery::Batch, Some(&send(160, tmp.path(), &a, 3)))), ["A #3"]);

        let later = send(200, tmp.path(), &a, 3);
        assert_eq!(numbers(&listening.take(AgentDelivery::Batch, Some(&later))), ["A #3"]);
        assert!(listening.take(AgentDelivery::Batch, Some(&later)).is_empty());
        // A send for another directory isn't for this agent.
        let elsewhere = AgentSend { at_ms: 300, ..elsewhere };
        assert!(listening.take(AgentDelivery::Batch, Some(&elsewhere)).is_empty());
    }

    #[test]
    fn each_mode_starts_at_the_end_and_resumes_from_a_token() {
        let tmp = tempfile::tempdir().unwrap();
        let a = note(tmp.path(), 30, "A");
        a.append("## 1 · 14:30\n\nBefore.").unwrap();
        let mut listening = Listening::start(tmp.path(), None, AgentDelivery::Each, &[]);
        assert!(listening.take(AgentDelivery::Each, None).is_empty());
        a.append("## 2 · 14:31\n\nAfter.").unwrap();
        assert_eq!(numbers(&listening.take(AgentDelivery::Each, None)), ["A #2"]);

        a.append("## 3 · 14:32\n\nWhile away.").unwrap();
        let resumed = Listening::resume(tmp.path(), None, &listening.token()).unwrap();
        assert_eq!(resumed, listening);
        assert_eq!(numbers(&listening.take(AgentDelivery::Each, None)), ["A #3"]);
        let empty =
            Listening::resume(tmp.path(), None, &Listening::start(tmp.path(), None, AgentDelivery::Batch, &[]).token());
        assert!(empty.is_some());
        assert_eq!(Listening::resume(tmp.path(), None, "nonsense"), None);
    }

    #[test]
    fn each_session_gets_the_asides_spoken_to_it() {
        let tmp = tempfile::tempdir().unwrap();
        let a = note(tmp.path(), 30, "A");
        let mut claude = Listening::start(tmp.path(), Some("claude"), AgentDelivery::Each, &[]);
        let mut codex = Listening::start(tmp.path(), Some("codex"), AgentDelivery::Each, &[]);
        let to = |entry, session: Option<&str>| Route { entry, session: session.map(Into::into), agent: None };
        let say = |n: u32, session: Option<&str>| {
            routes::append(&a.dir, &to(n, session)).unwrap();
            a.append(&format!("## {n} · 14:3{n}\n\nAside {n}.")).unwrap();
        };
        say(1, Some("claude"));
        say(2, Some("codex"));
        say(3, None);
        a.append("![x](images/02-143200.png)").unwrap();
        // Written before routes existed (or while no session was around): everyone gets it.
        a.append("## 4 · 14:34\n\nUnrouted.").unwrap();
        assert_eq!(numbers(&claude.take(AgentDelivery::Each, None)), ["A #1", "A #4"]);
        assert_eq!(numbers(&codex.take(AgentDelivery::Each, None)), ["A #2", "A #4"]);
        // An image added to an aside reaches whoever got the aside.
        a.append("![y](images/04-143500.png)").unwrap();
        assert_eq!(numbers(&claude.take(AgentDelivery::Each, None)), ["A #4 +![y](images/04-143500.png)"]);
        say(5, Some("codex"));
        assert!(claude.take(AgentDelivery::Each, None).is_empty());
        assert_eq!(numbers(&codex.take(AgentDelivery::Each, None)), ["A #4 +![y](images/04-143500.png)", "A #5"]);
    }

    #[test]
    fn a_batch_goes_to_the_session_it_was_sent_to() {
        let tmp = tempfile::tempdir().unwrap();
        let a = note(tmp.path(), 30, "A");
        let mut claude = Listening::start(tmp.path(), Some("claude"), AgentDelivery::Batch, &[]);
        let mut codex = Listening::start(tmp.path(), Some("codex"), AgentDelivery::Batch, &[]);
        a.append("## 1 · 14:30\n\nOne.").unwrap();
        a.append("## 2 · 14:31\n\nTwo.").unwrap();
        let first = AgentSend { session: Some("claude".into()), ..send(10, tmp.path(), &a, 2) };
        assert_eq!(numbers(&claude.take(AgentDelivery::Batch, Some(&first))), ["A #1", "A #2"]);
        assert!(codex.take(AgentDelivery::Batch, Some(&first)).is_empty());
        a.append("## 3 · 14:32\n\nThree.").unwrap();
        let second = AgentSend { session: Some("codex".into()), ..send(20, tmp.path(), &a, 3) };
        assert!(claude.take(AgentDelivery::Batch, Some(&second)).is_empty());
        assert_eq!(numbers(&codex.take(AgentDelivery::Batch, Some(&second))), ["A #3"], "not the first batch again");
    }

    #[test]
    fn makes_links_absolute() {
        let dir = Path::new("/r/.asides/2026-10-15-1432 Checkout");
        let text = "Here.\n\n![14:38:51](images/05-143851.png)\n\n[docs](https://x.dev/a) and ![x](/abs.png)";
        assert_eq!(
            absolute_links(dir, text),
            "Here.\n\n![14:38:51](</r/.asides/2026-10-15-1432 Checkout/images/05-143851.png>)\n\n\
             [docs](https://x.dev/a) and ![x](/abs.png)"
        );
        assert_eq!(
            absolute_links(dir, "5 · 14:38 · [audio](audio/05.m4a)"),
            format!("5 · 14:38 · [audio](<{}>)", dir.join("audio/05.m4a").display())
        );
        assert_eq!(image_paths(dir, text), [dir.join("images/05-143851.png")]);
        assert_eq!(absolute_links(dir, "a ](unclosed"), "a ](unclosed");
    }

    #[test]
    fn listener_records_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(Listener::read_all(&tmp.path().join("none")).is_empty());
        let listener = Listener {
            pid: 42,
            client: "Codex".into(),
            dir: "/r".into(),
            waiting: true,
            updated_ms: 1,
            ..Default::default()
        };
        listener.write(tmp.path()).unwrap();
        assert_eq!(Listener::read_all(tmp.path()), [listener]);
        fs::remove_file(Listener::file(tmp.path(), 42)).unwrap();
        assert!(Listener::read_all(tmp.path()).is_empty());
    }

    #[test]
    fn labels_clients() {
        assert_eq!(client_label("claude-code"), "Claude Code");
        assert_eq!(client_label("codex-mcp-client"), "Codex");
        assert_eq!(client_label("opencode"), "OpenCode");
        assert_eq!(client_label(""), "Agent");
        assert_eq!(client_label("Zed"), "Zed");
    }
}
