mod mcp;
mod measure;

use std::path::PathBuf;
use std::process::Command as Process;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use asides_core::config::{Config, pretty_shortcut};
use asides_core::history::{self, Aside};
use asides_core::link::Link;
use asides_core::markdown::Part;
use asides_core::paths::{self, BUNDLE_ID, tilde};
use asides_core::sessions::{self, AgentSession};
use asides_core::state::AppState;
use clap::{Parser, Subcommand};

/// asides: freeze the screen, click what's wrong, say why; it goes to your coding agent.
///
/// `⌃⌥Space` opens asides over any app: click to pin an element, drag an area, talk, and press
/// `⌃⌥Space` again. The aside is typed into the agent session you last typed into (Claude Code,
/// Codex, OpenCode… in Terminal, Ghostty or Diri), as your own message, and kept in
/// ~/.asides/history. Run `asides` to open the asides window.
#[derive(Parser)]
#[command(name = "asides", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Open asides on the screen, as ⌃⌥Space does (open, it sends the aside).
    Open,
    /// List the agent sessions asides can send asides to (the default first).
    Sessions {
        #[arg(long)]
        json: bool,
    },
    /// List the asides in the history, newest first.
    History {
        /// How many.
        #[arg(short = 'n', default_value_t = 20)]
        count: usize,
        #[arg(long)]
        json: bool,
    },
    /// Print an aside (the newest by default) as Markdown, with its screenshots' paths.
    Show {
        #[arg(default_value = "last")]
        id: String,
    },
    /// Send an aside from the history (the newest by default) to a session again.
    Send {
        #[arg(default_value = "last")]
        id: String,
        /// The session (its id, from `asides sessions`), or "history"; by default the one typed
        /// into last.
        #[arg(long)]
        to: Option<String>,
    },
    /// Download the voice model (the one in the settings, or MODEL: small.en, medium.en,
    /// large-v3-turbo…).
    Download { model: Option<String> },
    /// Show whether asides is running and what it's doing.
    Status,
    /// Quit the menu-bar app.
    Quit,
    /// Run the MCP server (stdio): agents can measure screenshots and read your asides.
    Mcp,
    /// Measure a screenshot: distances from a point to the nearest edges (X,Y), or the object
    /// inside an area and the space around it (X,Y,W,H), in the image's pixels.
    Measure {
        image: PathBuf,
        /// X,Y or X,Y,W,H.
        target: String,
        /// Edge tolerance in percent (2: faint borders count; 30: look through shadows).
        #[arg(long, default_value_t = asides_core::measure::Tolerance::DEFAULT.0)]
        tolerance: u8,
        /// Pixels per point, to also show points (2 for Retina screenshots).
        #[arg(long, default_value_t = 1.0)]
        scale: f64,
    },
    /// Development: process a WAV file as if it had just been said.
    #[command(hide = true)]
    Simulate {
        wav: PathBuf,
        #[arg(long)]
        image: Option<PathBuf>,
        #[arg(long, default_value_t = 0.0)]
        at: f32,
        /// Said about this annotation of the open asides session.
        #[arg(long)]
        annotation: Option<u32>,
        /// The session it goes to (outside asides): its id; the history only otherwise.
        #[arg(long)]
        to: Option<String>,
    },
    /// Development: run an asides session on an image, driven by a JSON script.
    #[command(hide = true)]
    SimulateMeasure {
        image: PathBuf,
        script: PathBuf,
        /// The session it goes to: its id; the history only otherwise.
        #[arg(long)]
        to: Option<String>,
    },
}

fn main() {
    // `asides history | head`: a closed pipe ends it quietly, as it does other commands.
    // SAFETY: before any thread exists.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    let cli = Cli::parse();
    if let Err(err) = run(cli) {
        eprintln!("asides: {err:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        None => {
            send(&Link::Window { request: None })?;
            Ok(())
        }
        Some(Command::Open) => send(&Link::Open { request: None }),
        Some(Command::Sessions { json }) => list_sessions(json),
        Some(Command::History { count, json }) => list_history(count, json),
        Some(Command::Show { id }) => {
            let aside = find(&id)?;
            print!("{}", absolute_message(&aside));
            Ok(())
        }
        Some(Command::Send { id, to }) => send_again(&id, to),
        Some(Command::Download { model }) => {
            let model = model.unwrap_or_else(|| Config::load(&paths::config_path()).unwrap_or_default().model);
            let request = request_id();
            send(&Link::Download { model: model.clone(), request: Some(request.clone()) })?;
            wait_for_ack(&request)?;
            println!("Downloading the voice model {model}: `asides status` shows how far it got.");
            Ok(())
        }
        Some(Command::Status) => status(),
        Some(Command::Quit) => {
            if running_state().is_none() {
                println!("asides is not running.");
                return Ok(());
            }
            send(&Link::Quit)
        }
        Some(Command::Mcp) => mcp::run(),
        Some(Command::Measure { image, target, tolerance, scale }) => {
            let frame = measure::load(&image)?;
            let target = measure::parse_target(&target)?;
            let name = image.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let tolerance = asides_core::measure::Tolerance(tolerance.min(100));
            println!("{}", measure::describe(&frame, &name, target, tolerance, scale)?);
            Ok(())
        }
        Some(Command::SimulateMeasure { image, script, to }) => {
            let request = request_id();
            let image = image.canonicalize().context("image not found")?;
            let script = script.canonicalize().context("script not found")?;
            send(&Link::SimulateMeasure { image, script, to, request: Some(request.clone()) })?;
            wait_for_ack(&request)?;
            println!("queued");
            Ok(())
        }
        Some(Command::Simulate { wav, image, at, annotation, to }) => {
            let request = request_id();
            let wav = wav.canonicalize().context("WAV file not found")?;
            let image = image.map(|i| i.canonicalize()).transpose().context("image not found")?;
            let link = Link::Simulate { wav, image, image_at: at, annotation, to, request: Some(request.clone()) };
            send(&link)?;
            wait_for_ack(&request)?;
            println!("queued");
            Ok(())
        }
    }
}

/// The sessions: as the running app last found them, or found now.
fn current_sessions() -> Vec<AgentSession> {
    let mut found = sessions::discover();
    found.sort_by_key(|s| std::cmp::Reverse(s.last_input));
    found
}

fn list_sessions(json: bool) -> Result<()> {
    let found = current_sessions();
    if json {
        println!("{}", serde_json::to_string_pretty(&found)?);
        return Ok(());
    }
    if found.is_empty() {
        println!("No agent sessions are running. Start Claude Code, Codex or OpenCode in Terminal, Ghostty or Diri.");
        return Ok(());
    }
    let default = sessions::latest(&found).map(AgentSession::id);
    let labels = sessions::labels(&found);
    for (session, label) in found.iter().zip(labels) {
        let mark = if Some(session.id()) == default { "●" } else { " " };
        let reach = if session.host.reachable() { String::new() } else { " (asides can't type there)".into() };
        let ago = session.last_input.map(ago).unwrap_or_default();
        println!(
            "{mark} {:<8} {label}  {}  in {}{reach}  {ago}",
            session.id(),
            tilde(&session.dir),
            session.host.name()
        );
    }
    Ok(())
}

fn ago(at: SystemTime) -> String {
    let secs = SystemTime::now().duration_since(at).unwrap_or_default().as_secs();
    match secs {
        0..60 => "typed into just now".into(),
        60..3600 => format!("typed into {} min ago", secs / 60),
        3600..86400 => format!("typed into {} h ago", secs / 3600),
        _ => format!("typed into {} days ago", secs / 86400),
    }
}

fn list_history(count: usize, json: bool) -> Result<()> {
    let asides: Vec<Aside> = history::list(&history::dir()).into_iter().take(count).collect();
    if json {
        let rows: Vec<serde_json::Value> = asides
            .iter()
            .map(|a| {
                serde_json::json!({
                    "id": a.id,
                    "created": a.created().to_rfc3339(),
                    "title": a.record.title,
                    "images": a.record.images.len(),
                    "deliveries": a.record.deliveries,
                    "dir": a.dir,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    if asides.is_empty() {
        println!("No asides yet. ⌃⌥Space opens asides.");
    }
    for aside in asides {
        let sent = match aside.last_delivery() {
            Some(d) if d.error.is_none() => format!("→ {}", d.to),
            Some(d) => format!("✗ {}", d.to),
            None => "· kept".into(),
        };
        println!("{}  {}  {sent}  {}", aside.id, aside.created().format("%H:%M"), aside.record.title);
    }
    Ok(())
}

fn find(id: &str) -> Result<Aside> {
    history::find(&history::dir(), id).with_context(|| match id {
        "last" => "there are no asides in the history yet".to_string(),
        id => format!("there's no aside {id} in the history (`asides history` lists them)"),
    })
}

/// An aside's message with its screenshots' absolute paths, for reading anywhere.
fn absolute_message(aside: &Aside) -> String {
    let mut text = String::new();
    for part in aside.parts() {
        match part {
            Part::Text(t) => text.push_str(&format!("{t}\n\n")),
            Part::Image(path) => text.push_str(&format!("![Screenshot]({path})\n\n")),
        }
    }
    text
}

/// Sends an aside again and waits for it to get there (or not).
fn send_again(id: &str, to: Option<String>) -> Result<()> {
    if running_state().is_none() {
        bail!("asides is not running. Open it from Applications (or run `asides`).");
    }
    let aside = find(id)?;
    let before = aside.record.deliveries.len();
    let request = request_id();
    send(&Link::Send { id: aside.id.clone(), to, request: Some(request.clone()) })?;
    wait_for_ack(&request)?;
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if let Some(again) = history::find(&history::dir(), &aside.id)
            && again.record.deliveries.len() > before
            && let Some(delivery) = again.last_delivery()
        {
            return match &delivery.error {
                None => {
                    println!("Sent to {}", delivery.to);
                    Ok(())
                }
                Some(error) => bail!("could not send it to {}: {error}", delivery.to),
            };
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    bail!("asides didn't say whether it got there; `asides status` may say why")
}

fn status() -> Result<()> {
    let config = Config::load(&paths::config_path()).unwrap_or_default();
    match running_state() {
        None => println!("asides is not running."),
        Some(state) => {
            println!("asides is running (pid {}): {}", state.pid, state.status);
            if let Some(detail) = &state.detail {
                println!("  {detail}");
            }
            println!(
                "  {} opens it, {} switches frozen and live",
                pretty_shortcut(&config.shortcuts.asides),
                pretty_shortcut(&config.shortcuts.freeze)
            );
            if !state.model_ready {
                println!("  no voice model yet: `asides download` (or click the microphone in asides)");
            }
            if state.microphone != "granted" && !state.microphone.is_empty() {
                println!("  microphone: {}", state.microphone);
            }
            let found = current_sessions();
            match sessions::latest(&found) {
                Some(session) => println!("  asides go to {} (the session typed into last)", session.label()),
                None => println!("  no agent session to send to: asides stay in the history"),
            }
        }
    }
    if let Some(aside) = history::list(&history::dir()).first() {
        let sent = match aside.last_delivery() {
            Some(d) if d.error.is_none() => format!("sent to {}", d.to),
            Some(d) => format!("not sent to {}: {}", d.to, d.error.clone().unwrap_or_default()),
            None => "kept in the history".into(),
        };
        println!("Last aside: {} ({}), {sent}", aside.record.title, aside.id);
    }
    Ok(())
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
            "could not reach asides.app ({}). Is it installed? Run `make install` in the asides repo.",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// The app this command ships in (`asides.app/Contents/Helpers/asides`), when it runs from one.
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
    bail!("asides did not respond within 15 s")
}

/// The published state, if the app that wrote it is still alive.
pub(crate) fn running_state() -> Option<AppState> {
    let state = AppState::read(&paths::state_path())?;
    is_running(state.pid).then_some(state)
}

pub(crate) fn request_id() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    format!("{}-{nanos}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_commands() {
        let parse = |line: &[&str]| Cli::try_parse_from(std::iter::once("asides").chain(line.iter().copied()));
        assert!(parse(&[]).unwrap().command.is_none());
        assert!(matches!(parse(&["open"]).unwrap().command, Some(Command::Open)));
        let Some(Command::Send { id, to }) = parse(&["send", "--to", "4242"]).unwrap().command else { panic!() };
        assert_eq!((id.as_str(), to.as_deref()), ("last", Some("4242")));
    }
}
