//! `asides mcp`: a Model Context Protocol server (JSON-RPC over stdio) that hands the user's
//! asides to a coding agent (Claude Code, Codex, OpenCode, …) as prompts. Agents wait for new
//! entries with `wait_for_asides`: each one as soon as it's transcribed, or in batches when the
//! user chooses Send to Agent (`agent_delivery` in config.toml). `get_asides` reads a whole note.
//! Agents that can watch a command in the background (Claude Code's Monitor tool) are told to run
//! `asides listen` instead, so they stay free between asides.
//!
//! It lets the agent close the loop on each aside, as Agentation's MCP server does: acknowledge it,
//! resolve it with a summary, dismiss it with a reason, or reply with a question (`threads.jsonl`
//! beside the note; the user sees it in the menu).
//! `get_pending_asides` lists what's still open, and `measure_image` measures a screenshot the way
//! Measure & Annotate measures the screen.
//!
//! Each server is one session (`sessions::Session`): `asides claude` and the like give it one, and
//! otherwise it makes its own. Listening (the first wait, or `asides listen` with its session)
//! makes it the session the user's asides go to, so they reach this agent without binding.
//!
//! Reading tools only read notes and the app's published state, so they're marked read-only
//! (agents such as Codex run those without asking). The others append to the aside's thread.

use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use asides_core::agents::{self, Cursor, Item, Listener, Listening};
use asides_core::config::{AgentDelivery, Config};
use asides_core::launch::{self, Via};
use asides_core::measure::{Rect, Tolerance};
use asides_core::notes::{self, Note};
use asides_core::paths::{self, tilde};
use asides_core::routes;
use asides_core::sessions::{self, Session as SessionRecord};
use asides_core::state::AppState;
use asides_core::threads::{self, AsideId, Event, Role, Status};
use serde_json::{Value, json};

use crate::session;

/// Protocol revisions this server speaks, oldest first; a client asking for another gets the newest.
const PROTOCOL_VERSIONS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];
const POLL: Duration = Duration::from_millis(400);
const PROGRESS_EVERY: Duration = Duration::from_secs(20);
/// Most clients give up on a tool call after 60 s (Codex, and the MCP SDKs' default), so a wait
/// returns before that unless the client is known to allow more or `--timeout` says otherwise.
const DEFAULT_WAIT: Duration = Duration::from_secs(50);
const CLAUDE_CODE_WAIT: Duration = Duration::from_secs(25 * 60);
const MAX_WAIT_SECS: u64 = 3600;
/// Screenshots larger than this are only linked; image content has size limits in most clients.
const MAX_INLINE_IMAGE: u64 = 3_750_000;
const MAX_INLINE_IMAGES: usize = 6;

/// What the server tells the agent about asides when it connects; `listen` is how this session
/// listens with Claude Code's Monitor tool.
fn instructions(listen: &str) -> String {
    format!(
        "Asides lets the user talk while they test their app: they press a shortcut, speak (a bug, a request, \
         an idea), and Asides transcribes it on their Mac and appends it to a Markdown note in this repo \
         (.asides/<date title>/notes.md), with screenshots inline. Each aside is a request from the user. When \
         the user asks you to listen to their asides and you can watch a command's output in the background \
         (Claude Code's Monitor tool), run `{listen}` that way: each aside arrives as an event. Otherwise call \
         wait_for_asides, act on what you get, then call it again; the user can't see its results, so first list \
         the asides you got, one short line each. Keep listening until they tell you to stop. Listening makes \
         this session the one the user's asides go to; if they switch to another session, asides wait until they \
         switch back. When they ask you to work through their notes, call get_asides (or get_pending_asides for \
         what's still open). Transcripts are verbatim speech-to-text: expect misheard words, especially names of \
         projects, files and libraries. Measure & Annotate shows up in entries as an annotated screenshot with \
         what the user measured (sizes and distances in points, guides) and numbered marks (### 1., ### 2.): an \
         element, an area, or a drawing (an arrow from one thing to another, a circled or underlined part), with \
         the app and window, the element's location (for web pages the URL and a CSS-like selector) and any typed \
         feedback. In a recorded entry, each mark sits where the user was speaking when they made it, so what \
         they said around it is about it; entries marked `Screen` were made without speaking. An aside is #N \
         (entry N of the note) or #N.M (mark M of entry N). When you've handled one, call resolve_aside with a \
         short summary; dismiss_aside with a reason if you won't act on it; reply_to_aside to ask the user \
         something. The user sees these in the Asides menu. measure_image measures a screenshot (distances \
         between edges, the bounds of an object)."
    )
}

/// How long one `asides listen` runs when an agent starts it, just under the Monitor tool's 30 min.
pub(crate) const LISTEN_SECS: u64 = 29 * 60;

/// Where the server stands: what this agent has been given so far.
struct Session {
    dir: PathBuf,
    /// The session this agent is (`sessions::Session`).
    id: String,
    /// This server made the session's record (the agent wasn't started by `asides <agent>`).
    made: bool,
    /// The client's name, as the agent introduced itself.
    client: String,
    /// Wait in seconds when a call doesn't say; `None` picks one for the client.
    timeout: Option<u64>,
    /// Set by the first wait: asides from before the agent listened aren't handed to it.
    listening: Option<Listening>,
    waits: usize,
}

impl Session {
    /// Where `get_asides` records what the agent has read, so a listening agent doesn't get it again.
    fn read_cursor<'a>(&'a mut self, scratch: &'a mut Cursor) -> &'a mut Cursor {
        match &mut self.listening {
            Some(listening) => &mut listening.cursor,
            None => scratch,
        }
    }
}

struct Server {
    session: Mutex<Session>,
    out: Mutex<io::Stdout>,
    running: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

pub fn run(dir: PathBuf, timeout: Option<u64>, id: Option<String>) -> Result<()> {
    let id = id.unwrap_or_else(sessions::new_id);
    let server = Arc::new(Server {
        session: Mutex::new(Session {
            dir,
            id,
            made: false,
            client: String::new(),
            timeout,
            listening: None,
            waits: 0,
        }),
        out: Mutex::new(io::stdout()),
        running: Mutex::new(HashMap::new()),
    });
    for line in io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(&line) {
            Ok(Value::Array(batch)) => batch.into_iter().for_each(|m| server.dispatch(m)),
            Ok(message) => server.dispatch(message),
            Err(err) => server.send(&json!({
                "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("parse error: {err}") }
            })),
        }
    }
    // The client hung up: stop waiting and stop showing as listening.
    for cancel in server.running.lock().unwrap().values() {
        cancel.store(true, Ordering::Relaxed);
    }
    let _ = fs::remove_file(Listener::file(&paths::listeners_dir(), std::process::id()));
    let session = server.session.lock().unwrap();
    session::leave(&session.id, session.made);
    Ok(())
}

impl Server {
    fn send(&self, message: &Value) {
        let mut out = self.out.lock().unwrap();
        let _ = writeln!(out, "{message}");
        let _ = out.flush();
    }

    fn reply(&self, id: &Value, result: Result<Value, (i64, String)>) {
        let message = match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => {
                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
            }
        };
        self.send(&message);
    }

    fn dispatch(self: &Arc<Self>, message: Value) {
        let method = message.get("method").and_then(Value::as_str).unwrap_or_default().to_string();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let Some(id) = message.get("id").cloned() else {
            // A notification (or a response to a request this server never makes).
            if method == "notifications/cancelled"
                && let Some(request) = params.get("requestId")
                && let Some(cancel) = self.running.lock().unwrap().get(&request.to_string())
            {
                cancel.store(true, Ordering::Relaxed);
            }
            return;
        };
        let result = match method.as_str() {
            "initialize" => Ok(self.initialize(&params)),
            "ping" | "logging/setLevel" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools() })),
            "prompts/list" => Ok(json!({ "prompts": prompts() })),
            "prompts/get" => self.prompt(&params),
            "tools/call" => return self.call_tool(id, &params),
            _ => Err((-32601, format!("method not found: {method}"))),
        };
        self.reply(&id, result);
    }

    fn initialize(&self, params: &Value) -> Value {
        let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or_default();
        let version = PROTOCOL_VERSIONS.iter().find(|v| **v == asked).unwrap_or(PROTOCOL_VERSIONS.last().unwrap());
        let client = params.pointer("/clientInfo/name").and_then(Value::as_str).unwrap_or_default();
        let mut session = self.session.lock().unwrap();
        session.client = client.to_string();
        // The agent is here: its session's record, which counts once it listens.
        session.made |= session::connect(&session.dir, &session.id, &agents::client_label(client));
        let listen = crate::listen::command(&session.dir, None, None, Some(&session.id));
        json!({
            "protocolVersion": version,
            "capabilities": { "tools": {}, "prompts": {} },
            "serverInfo": { "name": "asides", "title": "Asides", "version": env!("CARGO_PKG_VERSION") },
            "instructions": instructions(&listen),
        })
    }

    fn prompt(&self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
        let text = match name {
            "listen" => {
                let session = self.session.lock().unwrap();
                listen_prompt(&session.client, &session.dir, &session.id)
            }
            "asides" => {
                let mut session = self.session.lock().unwrap();
                let dir = session.dir.clone();
                let mut scratch = Cursor::default();
                let text = read_note(&dir, None, session.read_cursor(&mut scratch)).map_err(|e| (-32602, e))?;
                format!(
                    "Work through my asides below. I recorded them while testing; they were transcribed \
                     verbatim, so expect misheard words. Treat each entry as a request from me.\n\n{text}"
                )
            }
            "open" => {
                let dir = self.session.lock().unwrap().dir.clone();
                let text = pending_text(&dir, None, false).map_err(|e| (-32602, e))?;
                format!(
                    "Work through my open asides below, one at a time: fix each, then call resolve_aside with a short \
                     summary (or dismiss_aside with a reason, or reply_to_aside if you need to ask me something).\n\n{text}"
                )
            }
            _ => return Err((-32602, format!("unknown prompt: {name}"))),
        };
        Ok(json!({ "messages": [{ "role": "user", "content": { "type": "text", "text": text } }] }))
    }

    fn call_tool(self: &Arc<Self>, id: Value, params: &Value) {
        let name = params.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
        let args = params.get("arguments").cloned().unwrap_or(json!({}));
        match name.as_str() {
            "get_asides" => {
                let after = args.get("after_entry").and_then(Value::as_u64).map(|n| n as u32);
                let note = args.get("note").and_then(Value::as_str).map(str::to_string);
                let mut session = self.session.lock().unwrap();
                let dir = session.dir.clone();
                let mut scratch = Cursor::default();
                let found = match note.as_deref() {
                    Some(name) => find_note(&dir, Some(name)).map(Some),
                    None => Ok(None),
                };
                let me = session.id.clone();
                let read = found
                    .and_then(|note| read_note_at(&dir, note, after, session.read_cursor(&mut scratch), Some(&me)));
                let result = match read {
                    Ok(text) => tool_text(text, false),
                    Err(err) => tool_text(err, true),
                };
                drop(session);
                self.reply(&id, Ok(result));
            }
            "wait_for_asides" => {
                let cancel = Arc::new(AtomicBool::new(false));
                self.running.lock().unwrap().insert(id.to_string(), cancel.clone());
                let progress = params.pointer("/_meta/progressToken").cloned();
                let timeout = args.get("timeout_seconds").and_then(Value::as_u64);
                let server = self.clone();
                std::thread::spawn(move || {
                    let result = server.wait(timeout, progress, &cancel);
                    server.running.lock().unwrap().remove(&id.to_string());
                    // A cancelled request gets no response.
                    if let Some(result) = result {
                        server.reply(&id, Ok(result));
                    }
                });
            }
            "get_pending_asides" => {
                let dir = self.session.lock().unwrap().dir.clone();
                let note = args.get("note").and_then(Value::as_str);
                let all = args.get("all_notes").and_then(Value::as_bool).unwrap_or(false);
                let result = match pending_text(&dir, note, all) {
                    Ok(text) => tool_text(text, false),
                    Err(err) => tool_text(err, true),
                };
                self.reply(&id, Ok(result));
            }
            "list_asides_notes" => {
                let dir = self.session.lock().unwrap().dir.clone();
                self.reply(&id, Ok(tool_text(notes_text(&dir), false)));
            }
            "acknowledge_aside" | "resolve_aside" | "dismiss_aside" | "reply_to_aside" => {
                let (dir, client) = {
                    let session = self.session.lock().unwrap();
                    (session.dir.clone(), agents::client_label(&session.client))
                };
                let result = match record(&dir, &client, &name, &args) {
                    Ok(text) => tool_text(text, false),
                    Err(err) => tool_text(err, true),
                };
                self.reply(&id, Ok(result));
            }
            "measure_image" => {
                let dir = self.session.lock().unwrap().dir.clone();
                let result = match measure_image(&dir, &args) {
                    Ok(text) => tool_text(text, false),
                    Err(err) => tool_text(format!("{err:#}"), true),
                };
                self.reply(&id, Ok(result));
            }
            _ => self.reply(&id, Ok(tool_text(format!("Unknown tool: {name}"), true))),
        }
    }

    /// Waits until there's something to deliver or the time is up; `None` when cancelled.
    fn wait(&self, timeout: Option<u64>, progress: Option<Value>, cancel: &AtomicBool) -> Option<Value> {
        let (dir, id, client, started) = {
            let session = self.session.lock().unwrap();
            let client = agents::client_label(&session.client);
            (session.dir.clone(), session.id.clone(), client, session.listening.is_some())
        };
        // Asides pushes this session's asides into the conversation (`asides codex`, `asides opencode`).
        if SessionRecord::read(&paths::sessions_dir(), &id).is_some_and(|s| s.pushed) {
            return Some(tool_text(
                "Asides sends the user's asides to this session as messages, as they're recorded: there's \
                 nothing to wait for."
                    .into(),
                false,
            ));
        }
        // The first wait: this session listens, and the user's asides come to it.
        if !started {
            let (listening, made) = session::listen(&dir, &id, &client);
            let mut session = self.session.lock().unwrap();
            session.made |= made;
            session.listening.get_or_insert(listening);
        }
        self.set_waiting(1);
        let limit = {
            let session = self.session.lock().unwrap();
            let default = match session.timeout {
                Some(secs) => Duration::from_secs(secs),
                None if session.client.to_ascii_lowercase().contains("claude") => CLAUDE_CODE_WAIT,
                None => DEFAULT_WAIT,
            };
            timeout.map(|s| Duration::from_secs(s.clamp(1, MAX_WAIT_SECS))).unwrap_or(default)
        };
        let started = Instant::now();
        let mut next_progress = started + PROGRESS_EVERY;
        let mut steps = 0;
        let result = loop {
            if cancel.load(Ordering::Relaxed) {
                break None;
            }
            let items = self.take_new();
            if !items.is_empty() {
                break Some(delivery(&items));
            }
            if started.elapsed() >= limit {
                let mut text = format!(
                    "No new asides yet (waited {} s). Call wait_for_asides again to keep listening.",
                    limit.as_secs()
                );
                for notice in app_notices(&id) {
                    text.push_str(&format!("\n\n{notice}"));
                }
                break Some(tool_text(text, false));
            }
            if let Some(token) = &progress
                && Instant::now() >= next_progress
            {
                steps += 1;
                next_progress += PROGRESS_EVERY;
                self.send(&json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/progress",
                    "params": { "progressToken": token, "progress": steps, "message": "Listening for asides…" },
                }));
            }
            std::thread::sleep(POLL);
        };
        self.set_waiting(-1);
        result
    }

    /// Counts the waits in progress and publishes the listener record the app shows.
    fn set_waiting(&self, change: isize) {
        let mut session = self.session.lock().unwrap();
        session.waits = session.waits.saturating_add_signed(change);
        let listener = Listener {
            pid: std::process::id(),
            client: agents::client_label(&session.client),
            session: session.id.clone(),
            dir: session.dir.clone(),
            waiting: session.waits > 0,
            updated_ms: crate::now_ms(),
        };
        let _ = listener.write(&paths::listeners_dir());
    }

    /// What's ready for the agent now, given the delivery setting, and moves past it.
    fn take_new(&self) -> Vec<Item> {
        let delivery = Config::load(&paths::config_path()).unwrap_or_default().agent_delivery;
        let sends = AppState::read(&paths::state_path()).map(|s| s.agent_sends).unwrap_or_default();
        let mut session = self.session.lock().unwrap();
        session.listening.as_mut().map_or_else(Vec::new, |l| l.take(delivery, sends.first()))
    }
}

/// The listen prompt. Claude Code can watch `asides listen` with its Monitor tool and stay free in
/// between; other agents wait in a tool call.
fn listen_prompt(client: &str, dir: &Path, id: &str) -> String {
    let record = SessionRecord::read(&paths::sessions_dir(), id);
    let via = if record.as_ref().is_some_and(|s| s.pushed) {
        Via::Messages
    } else if client.to_ascii_lowercase().contains("claude") {
        Via::Monitor(crate::listen::command(dir, Some(LISTEN_SECS), None, Some(id)))
    } else {
        Via::Wait
    };
    let worktree = record.and_then(|s| s.worktree).map(|_| dir);
    launch::listen_prompt(&via, worktree)
}

/// Why asides might not be arriving, for the agent to pass on.
fn app_notices(id: &str) -> Vec<String> {
    if let Some(problem) = session::target_problem(id) {
        return vec![problem];
    }
    let Some(state) = crate::running_state() else { return Vec::new() };
    let mut notices = Vec::new();
    match state.status.as_str() {
        "recording" => notices.push("The user is recording an aside right now.".into()),
        "transcribing" => notices.push("An aside is being transcribed.".into()),
        "error" => notices.push(format!("Asides reports a problem: {}", state.detail.unwrap_or_default())),
        _ => {}
    }
    if Config::load(&paths::config_path()).unwrap_or_default().agent_delivery == AgentDelivery::Batch {
        notices.push("The user gets asides to you in batches: they arrive when the user chooses Send to Agent.".into());
    }
    notices
}

/// The tool result for new asides: their text, grouped by note, then the screenshots they show.
fn delivery(items: &[Item]) -> Value {
    let (mut text, images) = asides_text(items);
    text.push_str(
        "\n\nThe user can't see this result: first list these asides for them, one short line each (#N and a few \
         words), then act on them, and resolve each with resolve_aside (or reply_to_aside to ask). When you've \
         handled them, call wait_for_asides again to keep listening.",
    );
    let mut content = vec![json!({ "type": "text", "text": text })];
    content.extend(images.iter().filter_map(|path| inline_image(path)).take(MAX_INLINE_IMAGES));
    json!({ "content": content, "isError": false })
}

/// New asides as an agent reads them: what they are, then their text grouped by note, with
/// absolute links. Also returns the screenshots they show.
pub(crate) fn asides_text(items: &[Item]) -> (String, Vec<PathBuf>) {
    let count = items.iter().filter(|i| matches!(i, Item::Entry { .. })).count();
    let mut text = match count {
        0 => "The user added to asides you already have.".to_string(),
        1 => "A new aside from the user.".to_string(),
        n => format!("{n} new asides from the user."),
    };
    text.push_str(
        " They were spoken while testing and transcribed verbatim (expect misheard words); treat each as a \
         request from the user.",
    );
    let mut images = Vec::new();
    let mut current: Option<&Note> = None;
    for item in items {
        let (note, heading, body) = match item {
            Item::Entry { note, entry } => (note, entry.heading.clone(), entry.body.as_str()),
            Item::Addition { note, number, text } => (note, format!("{number} (added since)"), text.as_str()),
        };
        if current.is_none_or(|c| c.dir != note.dir) {
            // Once per note: what the user was testing (branch, commit, browser), in a line.
            let context = fs::read_to_string(note.file()).ok().and_then(|c| notes::context_line(&c));
            let context = context.map(|c| format!(", {c}")).unwrap_or_default();
            text.push_str(&format!("\n\nNote \"{}\" ({}{context}):", note.title, note.file().display()));
            current = Some(note);
        }
        let heading = agents::absolute_links(&note.dir, &heading);
        text.push_str(&format!("\n\n## {heading}\n\n{}", agents::absolute_links(&note.dir, body.trim())));
        images.extend(agents::image_paths(&note.dir, body));
    }
    (text, images)
}

/// The latest note with absolute links, or only its entries after `after`; moves `cursor` past them.
fn read_note(dir: &Path, after: Option<u32>, cursor: &mut Cursor) -> Result<String, String> {
    read_note_at(dir, None, after, cursor, None)
}

/// Like [`read_note`], for `note` (`None`: the latest note). Each entry is followed by what
/// happened to it since (its thread: status changes and replies).
fn read_note_at(
    dir: &Path,
    note: Option<Note>,
    after: Option<u32>,
    cursor: &mut Cursor,
    me: Option<&str>,
) -> Result<String, String> {
    let latest = notes::latest_note(dir);
    let note = note.or(latest.clone()).ok_or_else(|| format!("There are no asides in {} yet.", tilde(dir)))?;
    let content =
        fs::read_to_string(note.file()).map_err(|e| format!("Could not read {}: {e}", note.file().display()))?;
    let entries: Vec<_> =
        notes::parse_entries(&content).into_iter().filter(|e| after.is_none_or(|after| e.number > after)).collect();
    let threads = threads::fold(&threads::read(&note.dir));
    let routes = routes::read(&note.dir);
    let mut text = format!("Note \"{}\" ({})", note.title, note.file().display());
    if let Some(meta) = notes::front_matter(&content) {
        text.push_str(&format!("\n\n---\n{meta}\n---"));
    }
    if entries.is_empty() {
        text.push_str("\n\nNo entries yet.");
    }
    for entry in &entries {
        let heading = agents::absolute_links(&note.dir, &entry.heading);
        text.push_str(&format!("\n\n## {heading}\n\n{}", agents::absolute_links(&note.dir, &entry.body)));
        if let Some(route) = routes.get(&entry.number).filter(|r| r.session.is_some() && r.session.as_deref() != me) {
            let agent = route.agent.as_deref().unwrap_or("another agent");
            text.push_str(&format!("\n\n> The user said this to another session ({agent})."));
        }
        text.push_str(&thread_text(&threads, entry));
    }
    if content.contains("](images/") {
        text.push_str("\n\nScreenshots are linked by absolute path; open them to see what the user saw.");
    }
    // Don't hand these to a listening agent again (only the latest note's entries are delivered).
    if let Some(last) = entries.last().filter(|_| latest.as_ref() == Some(&note)) {
        let later_note = cursor.note.as_ref().is_none_or(|c| c.file_name() < note.dir.file_name());
        if later_note || (cursor.note.as_ref() == Some(&note.dir) && last.number >= cursor.entry) {
            *cursor = Cursor { note: Some(note.dir.clone()), entry: last.number, len: last.body.len() };
        }
    }
    Ok(text)
}

/// What happened to an entry and its annotations since it was written, quoted below it.
fn thread_text(threads: &std::collections::BTreeMap<AsideId, threads::Thread>, entry: &notes::Entry) -> String {
    let mut text = String::new();
    let mut ids = vec![AsideId::entry(entry.number)];
    ids.extend(threads::parts(&entry.body).into_iter().map(|p| AsideId { entry: entry.number, part: Some(p) }));
    for id in ids {
        if let Some(thread) = threads.get(&id) {
            text.push_str(&format!("\n\n> #{id}\n{}", threads::render(thread)));
        }
    }
    text
}

/// The note named `name` (its folder name or its title; the newest of that title), or the latest.
fn find_note(dir: &Path, name: Option<&str>) -> Result<Note, String> {
    let all = notes::list_notes(dir);
    let Some(name) = name.map(str::trim).filter(|n| !n.is_empty() && *n != "latest") else {
        return all.last().cloned().ok_or_else(|| format!("There are no asides in {} yet.", tilde(dir)));
    };
    all.iter()
        .rev()
        .find(|n| n.dir.file_name().is_some_and(|f| f.to_string_lossy() == name) || n.title == name)
        .cloned()
        .ok_or_else(|| format!("There's no note \"{name}\" in {}; list_asides_notes lists them.", tilde(dir)))
}

/// The asides nobody has resolved or dismissed yet, in the latest note (or `note`, or all notes).
fn pending_text(dir: &Path, note: Option<&str>, all: bool) -> Result<String, String> {
    let notes = if all { notes::list_notes(dir) } else { vec![find_note(dir, note)?] };
    if notes.is_empty() {
        return Err(format!("There are no asides in {} yet.", tilde(dir)));
    }
    let mut text = String::new();
    let mut count = 0;
    for note in &notes {
        let Ok(content) = fs::read_to_string(note.file()) else { continue };
        let threads = threads::fold(&threads::read(&note.dir));
        let mut section = String::new();
        for entry in notes::parse_entries(&content) {
            let status = threads::entry_status(&threads, &entry);
            if !status.is_open() {
                continue;
            }
            count += 1;
            let heading = agents::absolute_links(&note.dir, &entry.heading);
            section.push_str(&format!(
                "\n\n## {heading} ({})\n\n{}",
                status.as_str(),
                agents::absolute_links(&note.dir, &entry.body)
            ));
            let parts = threads::parts(&entry.body);
            if !parts.is_empty() {
                let states: Vec<String> = parts
                    .iter()
                    .map(|&p| {
                        let id = AsideId { entry: entry.number, part: Some(p) };
                        format!("#{id} {}", threads::status_of(&threads, id).as_str())
                    })
                    .collect();
                section.push_str(&format!("\n\nAnnotations: {}", states.join(", ")));
            }
            section.push_str(&thread_text(&threads, &entry));
        }
        if !section.is_empty() {
            text.push_str(&format!("\n\nNote \"{}\" ({}):{section}", note.title, note.file().display()));
        }
    }
    let scope = if all { "in this repo's notes".to_string() } else { format!("in note \"{}\"", notes[0].title) };
    Ok(match count {
        0 => format!("No open asides {scope}: every one is resolved or dismissed."),
        n => format!(
            "{n} open aside{} {scope} (pending or acknowledged). Resolve each with resolve_aside once it's handled.{text}",
            if n == 1 { "" } else { "s" }
        ),
    })
}

/// This repo's notes, oldest first, with how many of their asides are still open.
fn notes_text(dir: &Path) -> String {
    let all = notes::list_notes(dir);
    if all.is_empty() {
        return format!("There are no asides in {} yet.", tilde(dir));
    }
    let mut text = format!("Notes in {} (oldest first; the last one is where new asides go):", tilde(dir));
    for note in &all {
        let content = fs::read_to_string(note.file()).unwrap_or_default();
        let threads = threads::fold(&threads::read(&note.dir));
        let entries = notes::parse_entries(&content);
        let open = entries.iter().filter(|e| threads::entry_status(&threads, e).is_open()).count();
        let name = note.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        text.push_str(&format!("\n- \"{name}\": {} entries, {open} open", entries.len()));
    }
    text
}

/// Acknowledges, resolves, dismisses or replies to an aside, as `tool` says.
fn record(dir: &Path, client: &str, tool: &str, args: &Value) -> Result<String, String> {
    let id: AsideId =
        args.get("aside").and_then(Value::as_str).ok_or("aside is required, e.g. \"3\" or \"3.2\"")?.parse()?;
    let note = find_note(dir, args.get("note").and_then(Value::as_str))?;
    let content =
        fs::read_to_string(note.file()).map_err(|e| format!("Could not read {}: {e}", note.file().display()))?;
    let entries = notes::parse_entries(&content);
    let entry = entries
        .iter()
        .find(|e| e.number == id.entry)
        .ok_or_else(|| format!("Note \"{}\" has no aside #{}.", note.title, id.entry))?;
    if let Some(part) = id.part
        && !threads::parts(&entry.body).contains(&part)
    {
        return Err(format!("Aside #{} has no annotation {part}.", id.entry));
    }
    let text = |key: &str| args.get(key).and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty());
    let (status, message, done) = match tool {
        "acknowledge_aside" => (Some(Status::Acknowledged), None, "Acknowledged"),
        "resolve_aside" => (Some(Status::Resolved), text("summary"), "Resolved"),
        "dismiss_aside" => (Some(Status::Dismissed), Some(text("reason").ok_or("reason is required")?), "Dismissed"),
        _ => (None, Some(text("message").ok_or("message is required")?), "Replied to"),
    };
    let event = Event {
        aside: id,
        at_ms: crate::now_ms(),
        by: Role::Agent,
        author: Some(client.to_string()).filter(|c| !c.is_empty()),
        status,
        message: message.map(Into::into),
    };
    threads::append(&note.dir, &event).map_err(|e| format!("Could not record it: {e}"))?;
    Ok(format!("{done} #{id} in note \"{}\". The user sees it in the Asides menu.", note.title))
}

fn measure_image(dir: &Path, args: &Value) -> Result<String> {
    use anyhow::Context;
    let path = args.get("path").and_then(Value::as_str).context("path is required")?;
    let path = dir.join(path);
    let frame = crate::measure::load(&path)?;
    let n = |v: &Value, key: &str| v.get(key).and_then(Value::as_f64).map(|v| v.max(0.0).round() as u32);
    let target = match (args.get("area"), n(args, "x"), n(args, "y")) {
        (Some(area), ..) => {
            let (Some(x), Some(y), Some(w), Some(h)) =
                (n(area, "x"), n(area, "y"), n(area, "width"), n(area, "height"))
            else {
                anyhow::bail!("area needs x, y, width and height");
            };
            crate::measure::Target::Area(Rect::new(x, y, w.max(1), h.max(1)))
        }
        (None, Some(x), Some(y)) => crate::measure::Target::Point(x, y),
        _ => anyhow::bail!("give a point (x and y) or an area"),
    };
    let tolerance = Tolerance(n(args, "tolerance").unwrap_or(Tolerance::DEFAULT.0 as u32).min(100) as u8);
    let scale = args.get("scale").and_then(Value::as_f64).filter(|s| *s > 0.0).unwrap_or(1.0);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    crate::measure::describe(&frame, &name, target, tolerance, scale)
}

fn inline_image(path: &Path) -> Option<Value> {
    let mime = match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => return None,
    };
    if fs::metadata(path).ok()?.len() > MAX_INLINE_IMAGE {
        return None;
    }
    Some(json!({ "type": "image", "data": base64(&fs::read(path).ok()?), "mimeType": mime }))
}

fn tool_text(text: String, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

fn tools() -> Value {
    json!([
        {
            "name": "wait_for_asides",
            "title": "Wait for asides",
            "description": "Wait for the user's next spoken asides in this repo and return them. The user records \
                asides with a global shortcut while they test (bugs, requests, ideas); Asides transcribes them on \
                their Mac. Each aside is a request from the user: act on it, then call this tool again to keep \
                listening. Asides arrive one at a time as they're transcribed, or in a batch when the user chooses \
                Send to Agent, depending on their setting. Returns with no asides when the wait times out; call it \
                again to keep listening. Screenshots the user attached come back as images and absolute paths.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "timeout_seconds": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MAX_WAIT_SECS,
                        "description": "How long to wait before returning with no asides. Leave it out to use a \
                            wait that suits your client's tool timeout."
                    }
                }
            },
            "annotations": { "title": "Wait for asides", "readOnlyHint": true, "openWorldHint": false }
        },
        {
            "name": "get_asides",
            "title": "Read the latest asides note",
            "description": "Read the user's latest asides note in this repo: its metadata (git branch and commit, \
                browser, display) and its entries, with absolute paths to screenshots. Use it when the user asks \
                you to work through their asides or notes. Pass after_entry to get only later entries.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "after_entry": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Only return entries numbered higher than this."
                    },
                    "note": {
                        "type": "string",
                        "description": "Another note than the latest: its folder name or title (see list_asides_notes)."
                    }
                }
            },
            "annotations": { "title": "Read the latest asides note", "readOnlyHint": true, "openWorldHint": false }
        },
        {
            "name": "get_pending_asides",
            "title": "Get open asides",
            "description": "The user's asides nobody has resolved or dismissed yet (pending or acknowledged), in \
                the latest note, with their text, screenshots (absolute paths), annotations and replies so far. Use \
                it to work through what's left; resolve each one with resolve_aside when it's handled.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "note": { "type": "string", "description": "Another note than the latest: its folder name or title." },
                    "all_notes": { "type": "boolean", "description": "Look through every note in this repo." }
                }
            },
            "annotations": { "title": "Get open asides", "readOnlyHint": true, "openWorldHint": false }
        },
        {
            "name": "list_asides_notes",
            "title": "List asides notes",
            "description": "The asides notes in this repo, oldest first, with how many entries each has and how \
                many are still open. New asides go to the last one.",
            "inputSchema": { "type": "object", "properties": {} },
            "annotations": { "title": "List asides notes", "readOnlyHint": true, "openWorldHint": false }
        },
        lifecycle_tool(
            "acknowledge_aside",
            "Acknowledge an aside",
            "Tell the user you've seen an aside and are working on it (it shows as acknowledged in the Asides \
             menu).",
            None
        ),
        lifecycle_tool(
            "resolve_aside",
            "Resolve an aside",
            "Mark an aside as handled once you've done what it asked, with a short summary of what you did. Only \
             for what you actually addressed; if the user rejected your change, leave it open.",
            Some(("summary", "What you did, in a sentence.", false))
        ),
        lifecycle_tool(
            "dismiss_aside",
            "Dismiss an aside",
            "Decline to act on an aside (not a request, out of scope, already the case), with the reason.",
            Some(("reason", "Why you won't act on it.", true))
        ),
        lifecycle_tool(
            "reply_to_aside",
            "Reply to an aside",
            "Add a message to an aside's thread: a clarifying question, or an update. The user sees it in the \
             Asides menu and answers by recording another aside.",
            Some(("message", "Your message to the user.", true))
        ),
        {
            "name": "measure_image",
            "title": "Measure a screenshot",
            "description": "Measure a screenshot (PNG) like PixelSnap: from a point, the distances to the nearest \
                edges across and down; or, for an area, the bounds of the object inside it and the space around it \
                (padding), with its colors. Coordinates are the image's pixels; screenshots of Retina screens have \
                2 pixels per point (pass scale 2 to get points too). Use it to check spacing, alignment and sizes \
                in the user's screenshots or your own.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The PNG, absolute or relative to the repo." },
                    "x": { "type": "number", "description": "A point to measure from, in pixels." },
                    "y": { "type": "number" },
                    "area": {
                        "type": "object",
                        "description": "An area around an object, in pixels, to find the object's bounds.",
                        "properties": {
                            "x": { "type": "number" }, "y": { "type": "number" },
                            "width": { "type": "number" }, "height": { "type": "number" }
                        },
                        "required": ["x", "y", "width", "height"]
                    },
                    "tolerance": {
                        "type": "integer", "minimum": 0, "maximum": 100,
                        "description": "How different (in %) a color must be to count as an edge; raise it to look \
                            through shadows and gradients. Default 2."
                    },
                    "scale": { "type": "number", "description": "Pixels per point, to also report points." }
                },
                "required": ["path"]
            },
            "annotations": { "title": "Measure a screenshot", "readOnlyHint": true, "openWorldHint": false }
        }
    ])
}

/// A tool that adds to an aside's thread: `extra` is its text argument (name, description, required).
fn lifecycle_tool(name: &str, title: &str, description: &str, extra: Option<(&str, &str, bool)>) -> Value {
    let mut properties = json!({
        "aside": { "type": "string", "description": "The aside: \"3\" for entry 3, \"3.2\" for annotation 2 of it." },
        "note": { "type": "string", "description": "Its note, if not the latest: folder name or title." }
    });
    let mut required = vec!["aside"];
    if let Some((key, text, needed)) = extra {
        properties[key] = json!({ "type": "string", "description": text });
        if needed {
            required.push(key);
        }
    }
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": { "type": "object", "properties": properties, "required": required },
        "annotations": {
            "title": title,
            "readOnlyHint": false,
            "destructiveHint": false,
            "idempotentHint": name != "reply_to_aside",
            "openWorldHint": false
        }
    })
}

fn prompts() -> Value {
    json!([
        {
            "name": "listen",
            "title": "Listen to my asides",
            "description": "Act on each aside as the user records it, until they say stop."
        },
        {
            "name": "asides",
            "title": "Work through my asides",
            "description": "Work through the latest asides note in this repo."
        },
        {
            "name": "open",
            "title": "Work through my open asides",
            "description": "Fix the asides nobody has resolved yet, resolving each as it's done."
        }
    ])
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n =
            (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone};

    #[test]
    fn encodes_base64() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe, 0x00, 0x01]), "//4AAQ==");
    }

    #[test]
    fn delivers_entries_grouped_by_note_with_images() {
        let tmp = tempfile::tempdir().unwrap();
        let at = Local.with_ymd_and_hms(2026, 10, 15, 14, 32, 0).unwrap();
        let note = notes::create_note(tmp.path(), at, "Checkout", "---\ntitle: Checkout\n---").unwrap();
        fs::create_dir(note.dir.join("images")).unwrap();
        fs::write(note.dir.join("images/01-143300.png"), [0x89, b'P', b'N', b'G']).unwrap();
        note.append("## 1 · 14:32\n\nThe card.\n\n![14:33:00](images/01-143300.png)").unwrap();
        let items = agents::collect(tmp.path(), &Cursor::default(), None);
        let result = delivery(&items);
        let text = result.pointer("/content/0/text").and_then(Value::as_str).unwrap();
        assert!(text.starts_with("A new aside from the user."), "{text}");
        assert!(text.contains(&format!("Note \"Checkout\" ({})", note.file().display())), "{text}");
        assert!(
            text.contains(&format!(
                "## 1 · 14:32\n\nThe card.\n\n![14:33:00](<{}>)",
                note.dir.join("images/01-143300.png").display()
            )),
            "{text}"
        );
        assert_eq!(result.pointer("/content/1/type"), Some(&json!("image")));
        assert_eq!(result.pointer("/content/1/data"), Some(&json!("iVBORw==")));
    }

    #[test]
    fn reading_a_note_moves_the_cursor_past_it() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cursor = Cursor::default();
        assert!(read_note(tmp.path(), None, &mut cursor).unwrap_err().contains("no asides"));
        let at = Local.with_ymd_and_hms(2026, 10, 15, 14, 32, 0).unwrap();
        let note = notes::create_note(tmp.path(), at, "S", "---\ntitle: S\nbranch: main\n---").unwrap();
        note.append("## 1 · 14:32\n\nOne.").unwrap();
        note.append("## 2 · 14:33\n\nTwo.").unwrap();
        let text = read_note(tmp.path(), Some(1), &mut cursor).unwrap();
        assert!(text.contains("---\ntitle: S\nbranch: main\n---") && text.contains("## 2 · 14:33\n\nTwo."), "{text}");
        assert!(!text.contains("One."));
        assert_eq!(cursor, Cursor { note: Some(note.dir.clone()), entry: 2, len: 4 });
        assert!(agents::collect(tmp.path(), &cursor, None).is_empty());
    }

    #[test]
    fn agents_resolve_dismiss_and_reply_to_asides() {
        let tmp = tempfile::tempdir().unwrap();
        let at = Local.with_ymd_and_hms(2026, 10, 15, 14, 32, 0).unwrap();
        let note = notes::create_note(tmp.path(), at, "Checkout", "---\ntitle: Checkout\n---").unwrap();
        note.append("## 1 · 14:32\n\nThe card needs padding.").unwrap();
        note.append(
            "## 2 · 14:33 · Screen\n\n### 1. button \"Pay\"\n**Feedback:** Bigger\n\n### 2. Area 10 × 10 at (0, 0)",
        )
        .unwrap();
        let call = |tool: &str, args: Value| record(tmp.path(), "Claude Code", tool, &args);

        assert!(pending_text(tmp.path(), None, false).unwrap().starts_with("2 open asides in note \"Checkout\""));
        assert_eq!(
            call("resolve_aside", json!({ "aside": "1", "summary": "Added 16px padding" })).unwrap(),
            "Resolved #1 in note \"Checkout\". The user sees it in the Asides menu."
        );
        assert!(call("dismiss_aside", json!({ "aside": "2.2" })).unwrap_err().contains("reason is required"));
        call("dismiss_aside", json!({ "aside": "#2.2", "reason": "Not a request" })).unwrap();
        call("reply_to_aside", json!({ "aside": "2.1", "message": "How much bigger?" })).unwrap();
        assert!(call("acknowledge_aside", json!({ "aside": "2.3" })).unwrap_err().contains("no annotation 3"));
        assert!(call("acknowledge_aside", json!({ "aside": "9" })).unwrap_err().contains("no aside #9"));
        assert!(call("acknowledge_aside", json!({ "aside": "2", "note": "Other" })).unwrap_err().contains("no note"));

        let pending = pending_text(tmp.path(), None, false).unwrap();
        assert!(pending.starts_with("1 open aside in note \"Checkout\""), "{pending}");
        assert!(pending.contains("## 2 · 14:33 · Screen (pending)"), "{pending}");
        assert!(pending.contains("Annotations: #2.1 pending, #2.2 dismissed"), "{pending}");
        assert!(pending.contains("> #2.1\n> **Claude Code**: How much bigger?\n> Status: pending"), "{pending}");
        assert!(!pending.contains("The card needs padding"), "{pending}");

        let read = read_note(tmp.path(), None, &mut Cursor::default()).unwrap();
        assert!(
            read.contains("The card needs padding.\n\n> #1\n> **Claude Code** (resolved): Added 16px padding"),
            "{read}"
        );
        assert!(notes_text(tmp.path()).contains("\": 2 entries, 1 open"));

        call("resolve_aside", json!({ "aside": "2.1" })).unwrap();
        assert_eq!(
            pending_text(tmp.path(), Some("Checkout"), false).unwrap(),
            "No open asides in note \"Checkout\": every one is resolved or dismissed."
        );
    }
}
