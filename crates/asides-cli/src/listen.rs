//! `asides listen`: the user's asides on standard output as they arrive, for agents that watch a
//! command in the background instead of waiting in a tool call (Claude Code's Monitor tool turns
//! each burst of output into a notification in the chat). The agent stays free between asides.
//!
//! It listens as a session (`--session`, which `asides mcp` gives in its instructions), and gets
//! the asides the user speaks to that session. With `--codex-thread` it pushes them into a Codex
//! thread instead (`codex queue`), for a session `asides codex` started. Delivery follows `agent_delivery` like `asides mcp`
//! does. With `--timeout` it stops by itself and prints the command that carries on from there
//! (`--after`), so nothing recorded in between is lost.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use asides_core::agents::{self, Listener, Listening};
use asides_core::config::{AgentDelivery, Config, pretty_shortcut};
use asides_core::paths::{self, tilde};
use asides_core::sessions;
use asides_core::state::AppState;

use crate::{codex, mcp, session};

const POLL: Duration = Duration::from_millis(400);
/// Checking where the user's asides go starts a process, so it's checked less often than the notes.
const CHECK_TARGET: Duration = Duration::from_secs(10);

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn stop(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

/// Where deliveries go instead of standard output: a Codex thread, while the process `while_pid`
/// (the Codex window) runs, or until stopped.
pub struct Push {
    pub thread: String,
    pub while_pid: Option<u32>,
}

pub fn run(
    dir: PathBuf,
    timeout: Option<u64>,
    after: Option<String>,
    id: Option<String>,
    push: Option<Push>,
) -> Result<()> {
    // Stopped by the agent (or Ctrl-C): finish the poll, then remove the listener record.
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        unsafe { libc::signal(signal, stop as *const () as libc::sighandler_t) };
    }
    let delivery = || Config::load(&paths::config_path()).unwrap_or_default().agent_delivery;
    let sends = || AppState::read(&paths::state_path()).map(|s| s.agent_sends).unwrap_or_default();
    let id = id.unwrap_or_else(sessions::new_id);
    let client = if push.is_some() { "Codex".to_string() } else { client() };
    let (started_at, made) = session::listen(&dir, &id, &client);
    let mut listening = match &after {
        Some(token) => {
            Listening::resume(&dir, Some(&id), token).context("--after takes the token `asides listen` printed")?
        }
        None => started_at,
    };
    let record = Listener {
        pid: std::process::id(),
        client,
        session: id.clone(),
        dir: dir.clone(),
        waiting: true,
        updated_ms: crate::now_ms(),
    };
    let _ = record.write(&paths::listeners_dir());

    let mode = match delivery() {
        AgentDelivery::Each => "Each aside arrives here as soon as it's transcribed.".to_string(),
        AgentDelivery::Batch => {
            let shortcut =
                pretty_shortcut(&Config::load(&paths::config_path()).unwrap_or_default().shortcuts.send_to_agent);
            format!("Asides arrive here in batches, when the user chooses Send to Agent ({shortcut}).")
        }
    };
    say(&format!("Listening for asides in {}. {mode}", tilde(&dir)));

    let started = Instant::now();
    let mut problem: Option<String> = None;
    let mut next_check = started;
    // Pushes not made yet (Codex wasn't reachable): retried, in order, before anything newer.
    let mut unpushed: Vec<(String, Vec<PathBuf>)> = Vec::new();
    while !STOP.load(Ordering::Relaxed) {
        if let Some(push) = &push {
            if push.while_pid.is_some_and(|pid| !crate::is_running(pid)) {
                break;
            }
            let items = listening.take(delivery(), sends().first());
            if !items.is_empty() {
                let (mut text, images) = mcp::asides_text(&items);
                text.push_str(
                    "\n\nWhen you've handled each, call resolve_aside with its number and a short summary (or \
                     dismiss_aside with a reason, or reply_to_aside to ask me something).",
                );
                unpushed.push((text, images));
            }
            while let Some((text, images)) = unpushed.first() {
                match codex::queue(&push.thread, text, images) {
                    Ok(()) => drop(unpushed.remove(0)),
                    Err(err) => {
                        eprintln!("asides: {err:#}");
                        break;
                    }
                }
            }
            std::thread::sleep(POLL);
            continue;
        }
        if timeout.is_some_and(|secs| started.elapsed() >= Duration::from_secs(secs)) {
            let secs = started.elapsed().as_secs();
            let took = if secs < 60 { format!("{secs} s") } else { format!("{} min", (secs + 30) / 60) };
            let command = command(&dir, timeout, Some(&listening.token()), Some(&id));
            say(&format!("Stopped listening after {took}. To keep listening, run:\n\n{command}"));
            break;
        }
        if Instant::now() >= next_check {
            next_check += CHECK_TARGET;
            let now = session::target_problem(&id);
            if now != problem {
                match &now {
                    Some(text) => say(text),
                    None if problem.is_some() => say("The user's asides come to this session again."),
                    None => {}
                }
                problem = now;
            }
        }
        let items = listening.take(delivery(), sends().first());
        if !items.is_empty() {
            say(&mcp::asides_text(&items).0);
        }
        std::thread::sleep(POLL);
    }
    let _ = fs::remove_file(Listener::file(&paths::listeners_dir(), std::process::id()));
    session::leave(&id, made);
    Ok(())
}

/// Writes one event: all of it at once, so a watching agent gets it as one notification.
fn say(text: &str) {
    let mut out = io::stdout().lock();
    let _ = writeln!(out, "{text}\n");
    let _ = out.flush();
}

/// The agent running this, from the variables agents set for the commands they run.
fn client() -> String {
    if std::env::var_os("CLAUDECODE").is_some() {
        return "Claude Code".into();
    }
    agents::client_label(&std::env::var("AI_AGENT").unwrap_or_default())
}

/// How agents should run this program: `asides` when that's this program on the PATH, otherwise
/// its full path.
pub fn program() -> String {
    let exe = std::env::current_exe().and_then(fs::canonicalize).ok();
    let on_path = std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|d| fs::canonicalize(d.join("asides")).ok().is_some_and(|p| Some(p) == exe))
    });
    match (&exe, on_path) {
        (Some(exe), false) => quote(&exe.to_string_lossy()),
        _ => "asides".into(),
    }
}

/// The command line for `asides listen` in `dir`, as session `id`.
pub fn command(dir: &Path, timeout: Option<u64>, after: Option<&str>, id: Option<&str>) -> String {
    let mut command = format!("{} -C {} listen", program(), quote(&dir.to_string_lossy()));
    if let Some(id) = id {
        command.push_str(&format!(" --session {id}"));
    }
    if let Some(secs) = timeout {
        command.push_str(&format!(" --timeout {secs}"));
    }
    if let Some(token) = after {
        command.push_str(&format!(" --after {}", quote(token)));
    }
    command
}

/// `text` as one shell word.
pub fn quote(text: &str) -> String {
    if !text.is_empty() && text.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+@%=:,".contains(c)) {
        text.to_string()
    } else {
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_shell_words() {
        assert_eq!(quote("/Users/sam/code/shop"), "/Users/sam/code/shop");
        assert_eq!(quote("/Users/sam/My Repo"), "'/Users/sam/My Repo'");
        assert_eq!(quote("0:2:14:2026-10-16-0800 Sam's note"), r"'0:2:14:2026-10-16-0800 Sam'\''s note'");
        assert_eq!(quote(""), "''");
    }

    #[test]
    fn the_command_names_the_session() {
        let command = command(Path::new("/r/my shop"), Some(1740), Some("1:2:3:n"), Some("a1b2"));
        assert!(
            command.ends_with(" -C '/r/my shop' listen --session a1b2 --timeout 1740 --after 1:2:3:n"),
            "{command}"
        );
    }
}
