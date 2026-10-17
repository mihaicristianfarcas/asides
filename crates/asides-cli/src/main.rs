mod codex;
mod listen;
mod mcp;
mod measure;
mod opencode;
mod session;
mod start;

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use asides_core::agents::Listener;
use asides_core::config::{AgentDelivery, Config, pretty_shortcut};
use asides_core::launch::{self, AgentKind};
use asides_core::link::Link;
use asides_core::notes::{self, Note};
use asides_core::paths::{self, BUNDLE_ID, tilde};
use asides_core::project;
use asides_core::sessions::{self, Session};
use asides_core::state::AppState;
use clap::{Args, Parser, Subcommand};

/// Talk through bugs while you test; Asides appends what you said to a Markdown note in this repo.
///
/// Run `asides` in a repo (or any directory) to bind the menu-bar app to it, then use the
/// global shortcuts while testing. Notes live in `<repo>/.asides/`. Run `asides claude` (or
/// `codex`, `opencode`) to start a coding agent there that acts on each aside.
#[derive(Parser)]
#[command(name = "asides", version)]
struct Cli {
    /// Run as if asides was started in DIR (like `git -C`).
    #[arg(short = 'C', global = true, value_name = "DIR")]
    dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Start Claude Code here, listening for your asides: each one you record goes to it.
    Claude(StartArgs),
    /// Start Codex here, listening for your asides: each one you record goes to it.
    Codex(StartArgs),
    /// Start OpenCode here, listening for your asides: each one you record goes to it.
    Opencode(StartArgs),
    /// Bind this directory; the next recording starts a new note, titled TITLE or "Untitled".
    New { title: Vec<String> },
    /// Print the path of the latest note in this directory.
    Path,
    /// Open the latest note in the default Markdown editor.
    Open,
    /// Show what the menu-bar app is doing.
    Status,
    /// Zip the latest note folder (notes.md + images) for sharing.
    Export {
        /// Where to write the zip (default: ./<note folder>.zip).
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
    /// Quit the menu-bar app.
    Quit,
    /// Send to Agent: hand the asides recorded so far to agents listening in batch mode.
    Send,
    /// Run the MCP server that hands your asides to coding agents (Claude Code, Codex, OpenCode).
    ///
    /// Agents start it themselves; register it once, e.g. `claude mcp add asides -- asides mcp`.
    Mcp {
        /// How long a wait for asides lasts before the agent is told to call again, in seconds
        /// (default: 25 min for Claude Code, 50 s for other agents).
        #[arg(long, value_name = "SECONDS")]
        timeout: Option<u64>,
        /// The session this agent is (set by `asides claude` and the like); a new one otherwise.
        #[arg(long, value_name = "ID")]
        session: Option<String>,
    },
    /// Print your asides as they arrive, for an agent watching this command in the background.
    ///
    /// Claude Code runs it with its Monitor tool when you ask it to listen to your asides, so each
    /// aside shows up in the chat while the agent stays free in between. Runs until stopped.
    Listen {
        /// Stop after this many seconds, printing the command that carries on from there.
        #[arg(long, value_name = "SECONDS")]
        timeout: Option<u64>,
        /// Carry on from where an earlier `asides listen` stopped (the token it printed).
        #[arg(long, value_name = "TOKEN")]
        after: Option<String>,
        /// The session to listen as (`asides mcp` gives it in its instructions).
        #[arg(long, value_name = "ID")]
        session: Option<String>,
        /// Push the asides into this Codex thread instead of printing them (`asides codex`).
        #[arg(long, value_name = "THREAD", hide = true, conflicts_with = "opencode_session")]
        codex_thread: Option<String>,
        /// Push the asides into this OpenCode session instead of printing them (`asides opencode`).
        #[arg(long, value_name = "SESSION", hide = true)]
        opencode_session: Option<String>,
        /// When pushing: stop when this process (the agent's window) ends.
        #[arg(long, value_name = "PID", hide = true)]
        while_pid: Option<u32>,
    },
    /// Open Asides with the screen frozen, as its shortcut does; while it's open, add it all to the aside.
    Annotate,
    /// Measure a screenshot (PNG): distances from a point to the nearest edges, or the bounds of the
    /// object inside an area, in the image's pixels.
    Measure {
        image: PathBuf,
        /// X,Y for the distances from a point, or X,Y,WIDTH,HEIGHT for the object inside an area.
        target: String,
        /// How different (in %) a color must be to count as an edge.
        #[arg(long, default_value_t = asides_core::measure::Tolerance::DEFAULT.0)]
        tolerance: u8,
        /// Pixels per point, to also show points (2 for Retina screenshots).
        #[arg(long, default_value_t = 1.0)]
        scale: f64,
    },
    /// Development: process a WAV file as if it had just been recorded.
    #[command(hide = true)]
    Simulate {
        wav: PathBuf,
        #[arg(long)]
        new: bool,
        #[arg(long)]
        image: Option<PathBuf>,
        #[arg(long, default_value_t = 0.0)]
        at: f32,
        /// Said about this annotation of the open Measure & Annotate session.
        #[arg(long)]
        annotation: Option<u32>,
    },
    /// Development: run a Measure & Annotate session on an image, driven by a JSON script.
    #[command(hide = true)]
    SimulateMeasure { image: PathBuf, script: PathBuf },
}

#[derive(Args)]
struct StartArgs {
    /// Run in the background, without a terminal (Claude Code); `claude attach` opens it.
    #[arg(long)]
    background: bool,
    /// Work in the repo itself even when another session already does (no git worktree).
    #[arg(long)]
    no_worktree: bool,
    /// Arguments for the agent, e.g. `asides claude --model opus`.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, value_name = "AGENT ARGS")]
    args: Vec<String>,
}

fn main() {
    // Rust ignores SIGPIPE, which turns `asides status | head -1` into a panic. Behave like other
    // command-line tools instead: exit quietly when the reader goes away.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    if let Err(err) = run(Cli::parse()) {
        eprintln!("asides: {err:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    // Inside a session `asides <agent>` started, `asides` means the session's repo, even from the
    // worktree the agent works in.
    let session_repo = std::env::var_os(launch::REPO_VAR).map(PathBuf::from).filter(|d| d.is_dir());
    let start = match (&cli.dir, session_repo) {
        (Some(dir), _) => dir.clone(),
        (None, Some(repo)) => repo,
        (None, None) => std::env::current_dir()?,
    };
    let dir = || project::resolve_bind_dir(&start).with_context(|| format!("cannot use {}", start.display()));
    let session = |given: Option<String>| -> Result<Option<String>> {
        let id = given.or_else(|| std::env::var(launch::SESSION_VAR).ok()).filter(|id| !id.is_empty());
        match id {
            Some(id) if !sessions::valid_id(&id) => bail!("\"{id}\" isn't a session id"),
            id => Ok(id),
        }
    };
    let agent = |kind: AgentKind, args: StartArgs| {
        let options = start::Options { background: args.background, no_worktree: args.no_worktree, args: args.args };
        start::run(kind, &start, options)
    };
    match cli.command {
        None => bind(&dir()?, false, None),
        Some(Command::Claude(args)) => agent(AgentKind::Claude, args),
        Some(Command::Codex(args)) => agent(AgentKind::Codex, args),
        Some(Command::Opencode(args)) => agent(AgentKind::OpenCode, args),
        Some(Command::New { title }) => {
            let title = Some(title.join(" ")).filter(|t| !t.trim().is_empty());
            bind(&dir()?, true, title)
        }
        Some(Command::Path) => {
            println!("{}", latest(&dir()?)?.file().display());
            Ok(())
        }
        Some(Command::Open) => {
            let file = latest(&dir()?)?.file();
            check(Process::new("open").arg(&file), "open the note")
        }
        Some(Command::Status) => status(&dir()?),
        Some(Command::Export { out }) => export(&dir()?, out),
        Some(Command::Quit) => {
            if running_state().is_none() {
                println!("Asides is not running.");
                return Ok(());
            }
            send(&Link::Quit)
        }
        Some(Command::Send) => send_batch(),
        Some(Command::Mcp { timeout, session: id }) => mcp::run(dir()?, timeout, session(id)?),
        Some(Command::Listen { timeout, after, session: id, codex_thread, opencode_session, while_pid }) => {
            let (dir, id) = (dir()?, session(id)?);
            let codex = codex_thread.map(|thread| {
                // Loaded again after the daemon restarts, the thread gets its asides server back.
                let config = launch::codex_thread_config(&listen::program(), &dir, id.as_deref().unwrap_or_default());
                listen::PushTo::Codex(thread, config)
            });
            let push =
                codex.or(opencode_session.map(listen::PushTo::OpenCode)).map(|to| listen::Push { to, while_pid });
            listen::run(dir, timeout, after, id, push)
        }
        Some(Command::Annotate) => {
            if running_state().is_none() {
                bail!("Asides is not running. Run `asides` in a repo to start it.");
            }
            send(&Link::Measure { request: None })
        }
        Some(Command::Measure { image, target, tolerance, scale }) => {
            let frame = measure::load(&image)?;
            let target = measure::parse_target(&target)?;
            let name = image.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let tolerance = asides_core::measure::Tolerance(tolerance.min(100));
            println!("{}", measure::describe(&frame, &name, target, tolerance, scale)?);
            Ok(())
        }
        Some(Command::SimulateMeasure { image, script }) => {
            let request = request_id();
            let image = image.canonicalize().context("image not found")?;
            let script = script.canonicalize().context("script not found")?;
            send(&Link::SimulateMeasure { image, script, request: Some(request.clone()) })?;
            wait_for_ack(&request)?;
            println!("queued");
            Ok(())
        }
        Some(Command::Simulate { wav, new, image, at, annotation }) => {
            let request = request_id();
            let wav = wav.canonicalize().context("WAV file not found")?;
            let image = image.map(|i| i.canonicalize()).transpose().context("image not found")?;
            let link =
                Link::Simulate { wav, new_note: new, image, image_at: at, annotation, request: Some(request.clone()) };
            send(&link)?;
            wait_for_ack(&request)?;
            println!("queued");
            Ok(())
        }
    }
}

fn bind(dir: &Path, new_note: bool, title: Option<String>) -> Result<()> {
    if project::ensure_git_exclude(dir)? {
        println!("Added .asides/ to .git/info/exclude (local only; .gitignore untouched).");
    }
    let request = request_id();
    send(&Link::Bind { dir: dir.to_path_buf(), new_note, title: title.clone(), request: Some(request.clone()) })?;
    let state = wait_for_ack(&request)?;

    let config = Config::load(&paths::config_path()).unwrap_or_default();
    let target = if new_note {
        match &title {
            Some(t) => format!("next recording starts a new note \"{t}\""),
            None => format!("next recording starts a new note \"{}\"", notes::DEFAULT_TITLE),
        }
    } else {
        let branch = project::current_branch(dir);
        match notes::latest_note(dir) {
            Some(note) => match config
                .auto_new_note
                .then(|| notes::moved_on(&note, branch.as_deref(), std::time::SystemTime::now()))
                .flatten()
            {
                Some(why) => format!("next recording starts a new note ({why})"),
                None => format!("appending to \"{}\" ({})", note.title, entries(note.entry_count())),
            },
            None => "first recording creates a note".into(),
        }
    };
    println!("Asides ready in {}", tilde(dir));
    println!("  → {target}");
    match state.target_session.as_deref().and_then(|id| Session::read(&paths::sessions_dir(), id)) {
        Some(session) => println!("  → your asides go to {}", session.label()),
        None => println!("  → notes only: `asides claude` (or codex, opencode) starts an agent that gets each aside"),
    }
    println!(
        "  {} open Asides (again: add it all to the aside; hold: talk) · {} into a new note",
        pretty_shortcut(&config.shortcuts.asides),
        pretty_shortcut(&config.shortcuts.new_note),
    );
    println!(
        "  {} back to the live screen, or freeze it again · {} attach clipboard image",
        pretty_shortcut(&config.shortcuts.freeze),
        pretty_shortcut(&config.shortcuts.attach_image),
    );
    if state.microphone == "denied" {
        println!("  ! Microphone access is denied: System Settings → Privacy & Security → Microphone → Asides.");
    }
    if (state.status == "downloading" || state.status == "error")
        && let Some(detail) = state.detail
    {
        println!("  ! {detail}");
    }
    Ok(())
}

fn send_batch() -> Result<()> {
    if running_state().is_none() {
        bail!("Asides is not running. Run `asides` in a repo to start it.");
    }
    let last_send = |state: &AppState| state.agent_sends.first().map(|s| s.at_ms);
    let sent_before = AppState::read(&paths::state_path()).and_then(|s| last_send(&s));
    let request = request_id();
    send(&Link::Send { request: Some(request.clone()) })?;
    let state = wait_for_ack(&request)?;
    let sent = last_send(&state) != sent_before;
    if sent && Config::load(&paths::config_path()).unwrap_or_default().agent_delivery == AgentDelivery::Each {
        println!("Sent. (Agents already get each aside as it's transcribed: agent_delivery is \"each\".)");
    } else if sent {
        println!("Sent to the agents listening in {}.", state.bound_dir.as_deref().map(tilde).unwrap_or_default());
    } else if state.status == "recording" || state.status == "transcribing" {
        println!("Sending once the last recording is transcribed.");
    } else {
        bail!("{}", state.detail.unwrap_or_else(|| "nothing was sent".into()));
    }
    Ok(())
}

fn status(dir: &Path) -> Result<()> {
    match running_state() {
        None => println!("Asides is not running. Run `asides` in a repo to start it."),
        Some(state) => {
            println!("Asides is running (pid {}) · {}", state.pid, state.status);
            if let Some(detail) = &state.detail {
                println!("  {detail}");
            }
            match &state.bound_dir {
                Some(bound) => println!("  bound to {}", tilde(bound)),
                None => println!("  not bound to a directory"),
            }
            let live = live_sessions();
            let listeners = Listener::read_all(&paths::listeners_dir());
            let labels = sessions::labels(&live);
            match state.target_session.as_ref().and_then(|id| live.iter().position(|s| &s.id == id)) {
                Some(i) => println!("  asides go to {}", labels[i]),
                None => println!("  asides go to notes only"),
            }
            for (session, label) in live.iter().zip(&labels) {
                println!("  session {label}: {}", session.state(&listeners, now_ms()).as_str());
            }
            if let Some(pending) = &state.pending_new {
                let title = pending.title.as_deref().map(|t| format!(" \"{t}\"")).unwrap_or_default();
                println!("  next recording starts a new note{title}");
            }
            if state.microphone != "granted" && !state.microphone.is_empty() {
                println!("  microphone: {}", state.microphone);
            }
        }
    }
    match notes::latest_note(dir) {
        Some(note) => println!("Latest note here: {} ({})", tilde(&note.file()), entries(note.entry_count())),
        None => println!("No notes in {} yet.", tilde(dir)),
    }
    Ok(())
}

fn export(dir: &Path, out: Option<PathBuf>) -> Result<()> {
    let note = latest(dir)?;
    let name = note.dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let out = match out {
        Some(out) => out,
        None => std::env::current_dir()?.join(format!("{name}.zip")),
    };
    notes::export_zip(&note, &out).with_context(|| format!("could not write {}", out.display()))?;
    println!("{}", out.display());
    Ok(())
}

fn latest(dir: &Path) -> Result<Note> {
    notes::latest_note(dir).with_context(|| format!("no notes in {} yet", tilde(dir)))
}

/// The agent sessions that are listening and still running, oldest first.
pub(crate) fn live_sessions() -> Vec<Session> {
    let listeners = Listener::read_all(&paths::listeners_dir());
    let now = now_ms();
    Session::read_all(&paths::sessions_dir())
        .into_iter()
        .filter(|s| s.listening)
        .filter(|s| if s.pid == 0 { !s.expired(&listeners, now) } else { is_running(s.pid) })
        .collect()
}

pub(crate) fn is_running(pid: u32) -> bool {
    pid != 0 && Process::new("kill").args(["-0", &pid.to_string()]).output().is_ok_and(|o| o.status.success())
}

/// Hands the link to the app via Launch Services (launching it if needed, without stealing focus):
/// the copy `ASIDES_APP` names, or the one this command ships in. Several copies share the bundle
/// ID, and Launch Services, left to pick, may start another one beside the running one.
pub(crate) fn send(link: &Link) -> Result<()> {
    let mut open = Process::new("open");
    open.arg("-g");
    match std::env::var_os("ASIDES_APP").map(PathBuf::from).or_else(own_app) {
        Some(app) => open.arg("-a").arg(app),
        None => open.args(["-b", BUNDLE_ID]),
    };
    let out = open.arg(link.to_url()).output()?;
    if !out.status.success() {
        bail!(
            "could not reach Asides.app ({}). Is it installed? Run `make install` in the asides repo.",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// The app this command ships in (`Asides.app/Contents/Helpers/asides`), when it runs from one.
fn own_app() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let helpers = exe.parent()?;
    let contents = helpers.parent()?;
    let app = contents.parent()?;
    (helpers.file_name()? == "Helpers" && contents.file_name()? == "Contents" && app.extension()? == "app")
        .then(|| app.to_path_buf())
}

pub(crate) fn wait_for_ack(request: &str) -> Result<AppState> {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if let Some(state) = AppState::read(&paths::state_path())
            && state.last_request.as_deref() == Some(request)
        {
            return Ok(state);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    bail!("Asides did not respond within 15 s")
}

/// The published state, if the app that wrote it is still alive.
pub(crate) fn running_state() -> Option<AppState> {
    let state = AppState::read(&paths::state_path())?;
    is_running(state.pid).then_some(state)
}

fn check(cmd: &mut Process, what: &str) -> Result<()> {
    let status = cmd.status().with_context(|| format!("failed to {what}"))?;
    if !status.success() {
        bail!("failed to {what}");
    }
    Ok(())
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub(crate) fn request_id() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    format!("{}-{nanos}", std::process::id())
}

fn entries(n: u32) -> String {
    if n == 1 { "1 entry".into() } else { format!("{n} entries") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start_args(line: &[&str]) -> (bool, bool, Vec<String>) {
        match Cli::try_parse_from(line).unwrap().command {
            Some(Command::Claude(a) | Command::Codex(a) | Command::Opencode(a)) => {
                (a.background, a.no_worktree, a.args)
            }
            _ => panic!("not an agent command"),
        }
    }

    #[test]
    fn agent_commands_pass_the_agents_own_arguments_through() {
        assert_eq!(start_args(&["asides", "claude"]), (false, false, vec![]));
        assert_eq!(
            start_args(&["asides", "claude", "--model", "opus"]),
            (false, false, vec!["--model".into(), "opus".into()])
        );
        assert_eq!(
            start_args(&["asides", "-C", "/r", "claude", "--background", "--no-worktree", "--effort", "high"]),
            (true, true, vec!["--effort".into(), "high".into()])
        );
        assert_eq!(start_args(&["asides", "codex", "-m", "o3"]).2, ["-m", "o3"]);
        assert_eq!(start_args(&["asides", "opencode", "--", "--agent", "plan"]).2, ["--agent", "plan"]);
    }
}
