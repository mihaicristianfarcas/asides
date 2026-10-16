mod listen;
mod mcp;
mod measure;

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use asides_core::config::{AgentDelivery, Config, pretty_shortcut};
use asides_core::link::Link;
use asides_core::notes::{self, Note};
use asides_core::paths::{self, BUNDLE_ID, tilde};
use asides_core::project;
use asides_core::state::AppState;
use clap::{Parser, Subcommand};

/// Talk through bugs while you test; Asides appends what you said to a Markdown note in this repo.
///
/// Run `asides` in a repo (or any directory) to bind the menu-bar app to it, then use the
/// global shortcuts while testing. Notes live in `<repo>/.asides/`.
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
    },
    /// Freeze the screen to measure and annotate it (Measure & Annotate), as the shortcut does.
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
    },
    /// Development: run a Measure & Annotate session on an image, driven by a JSON script.
    #[command(hide = true)]
    SimulateMeasure { image: PathBuf, script: PathBuf },
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
    let start = match &cli.dir {
        Some(dir) => dir.clone(),
        None => std::env::current_dir()?,
    };
    let dir = || project::resolve_bind_dir(&start).with_context(|| format!("cannot use {}", start.display()));
    match cli.command {
        None => bind(&dir()?, false, None),
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
        Some(Command::Mcp { timeout }) => mcp::run(dir()?, timeout),
        Some(Command::Listen { timeout, after }) => listen::run(dir()?, timeout, after),
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
        Some(Command::Simulate { wav, new, image, at }) => {
            let request = request_id();
            let wav = wav.canonicalize().context("WAV file not found")?;
            let image = image.map(|i| i.canonicalize()).transpose().context("image not found")?;
            send(&Link::Simulate { wav, new_note: new, image, image_at: at, request: Some(request.clone()) })?;
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
    println!(
        "  {} record · {} record into a new note · {} attach clipboard image · {} cancel",
        pretty_shortcut(&config.shortcuts.record),
        pretty_shortcut(&config.shortcuts.record_new_note),
        pretty_shortcut(&config.shortcuts.attach_image),
        pretty_shortcut(&config.shortcuts.cancel),
    );
    if !config.shortcuts.measure.trim().is_empty() {
        println!("  {} measure & annotate the screen", pretty_shortcut(&config.shortcuts.measure));
    }
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

/// Hands the link to the app via Launch Services (launching it if needed, without stealing focus).
/// `ASIDES_APP` names the copy to use, when several are installed: Launch Services picks one by
/// bundle ID otherwise (the end-to-end test points it at the build under test).
fn send(link: &Link) -> Result<()> {
    let mut open = Process::new("open");
    open.arg("-g");
    match std::env::var_os("ASIDES_APP") {
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

fn wait_for_ack(request: &str) -> Result<AppState> {
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
fn running_state() -> Option<AppState> {
    let state = AppState::read(&paths::state_path())?;
    let alive = Process::new("kill").args(["-0", &state.pid.to_string()]).output().ok()?.status.success();
    (state.pid != 0 && alive).then_some(state)
}

fn check(cmd: &mut Process, what: &str) -> Result<()> {
    let status = cmd.status().with_context(|| format!("failed to {what}"))?;
    if !status.success() {
        bail!("failed to {what}");
    }
    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn request_id() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    format!("{}-{nanos}", std::process::id())
}

fn entries(n: u32) -> String {
    if n == 1 { "1 entry".into() } else { format!("{n} entries") }
}
