//! `asides claude`, `asides codex`, `asides opencode`: starts that agent in this repo, already
//! listening for the user's asides, as a new session the app sends them to. The agent gets asides'
//! MCP server on its command line (nothing to set up first) and the listen prompt as its first
//! message. The menu's New Session runs this in a terminal.
//!
//! When another session is already working in the repo, the new one works in a git worktree, so
//! their edits don't collide; the notes stay in the repo. Claude Code makes the worktree itself;
//! for the others, asides makes it in `.asides/worktrees/`.
//!
//! A Codex session is a thread asides starts in Codex's app-server daemon (`codex.rs`), with a
//! process of `asides listen` pushing each aside into it; the terminal then opens the thread
//! (`codex resume`). Without the daemon, Codex waits for asides in a tool call instead.

use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command as Process, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use asides_core::agents::Listening;
use asides_core::config::{Config, pretty_shortcut};
use asides_core::launch::{self, AgentKind, Launch, Via};
use asides_core::link::Link;
use asides_core::paths::{self, tilde};
use asides_core::project;
use asides_core::sessions::{self, Session};
use asides_core::state::AppState;
use serde_json::Value;

use crate::{codex, listen};

pub struct Options {
    pub background: bool,
    pub no_worktree: bool,
    pub args: Vec<String>,
}

pub fn run(kind: AgentKind, start: &Path, options: Options) -> Result<()> {
    if options.background && !kind.runs_in_background() {
        bail!("{} can't run in the background; run `asides {}` in a terminal instead.", kind.name(), kind.program());
    }
    let repo = project::resolve_bind_dir(start).with_context(|| format!("cannot use {}", start.display()))?;
    if project::ensure_git_exclude(&repo)? {
        println!("Added .asides/ to .git/info/exclude (local only; .gitignore untouched).");
    }
    let config = Config::load(&paths::config_path()).unwrap_or_default();
    let id = sessions::new_id();
    let repo_name = repo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();

    prune_worktrees(&repo);
    let others: Vec<Session> = crate::live_sessions().into_iter().filter(|s| s.dir == repo).collect();
    let worktree =
        (!others.is_empty() && config.agents.worktrees && !options.no_worktree && project::can_branch(&repo))
            .then(|| format!("asides-{id}"));
    let mut workdir = repo.clone();
    if let Some(name) = worktree.as_deref().filter(|_| !kind.makes_worktrees()) {
        workdir = project::worktree_path(&repo, name);
        project::add_worktree(&repo, &workdir, &branch(name))
            .with_context(|| format!("could not make a worktree at {}", tilde(&workdir)))?;
    }

    let sends = AppState::read(&paths::state_path()).map(|s| s.agent_sends).unwrap_or_default();
    let start = Listening::start(&repo, Some(&id), config.agent_delivery, &sends).token();
    let name = format!("{repo_name} · asides {id}");
    let mut session = Session {
        id: id.clone(),
        agent: kind.name().into(),
        dir: repo.clone(),
        worktree: worktree.clone(),
        // `exec` below makes this process the agent; a background agent's isn't known yet.
        pid: if options.background { 0 } else { std::process::id() },
        background: options.background.then(|| name.clone()),
        thread: None,
        pushed: false,
        listening: true,
        start: Some(start),
        started_ms: crate::now_ms(),
    };
    session.write(&paths::sessions_dir()).context("could not record the session")?;
    let asides = listen::program();
    let in_worktree = worktree.as_ref().map(|_| repo.as_path());

    // Codex: a thread in its daemon, that asides pushes the asides into.
    if kind.takes_pushes() {
        let new = codex::NewThread {
            cwd: &workdir,
            name: &name,
            config: launch::codex_thread_config(&asides, &repo, &id),
            prompt: &launch::listen_prompt(&Via::Messages, in_worktree),
        };
        match codex::start_thread(&new) {
            Ok(thread) => {
                session.thread = Some(thread.clone());
                session.pushed = true;
                // In a terminal it pushes while Codex's window is open; in the background, until stopped.
                let pusher = start_pusher(&repo, &id, &thread, (!options.background).then(std::process::id))?;
                if options.background {
                    session.pid = pusher;
                }
                session.write(&paths::sessions_dir()).context("could not record the session")?;
            }
            Err(err) if options.background => {
                Session::remove(&paths::sessions_dir(), &id);
                return Err(err).context("Codex runs in the background through its app-server daemon");
            }
            Err(err) => {
                println!("! Codex's app-server didn't start the session ({err:#}): it waits for asides instead.")
            }
        }
    }
    // The app sends the user's asides to it from now on (and starts, if it isn't running).
    let reached = tell_app(&id);

    let via = if kind.listens_with_monitor() {
        Via::Monitor(listen::command(&repo, Some(crate::mcp::LISTEN_SECS), None, Some(&id)))
    } else {
        Via::Wait
    };
    let prompt = launch::listen_prompt(&via, in_worktree);
    let listen_prefix = format!("{asides} -C {} listen", listen::quote(&repo.to_string_lossy()));
    let mut extra = kind.extra_args(&config.agents).to_vec();
    extra.extend(options.args);
    let command = launch::command(&Launch {
        kind,
        asides: &asides,
        repo: &repo,
        session: &id,
        name: &name,
        prompt: &prompt,
        listen_prefix: Some(&listen_prefix),
        worktree: worktree.as_deref(),
        background: options.background,
        thread: session.thread.as_deref(),
        opencode_1: kind == AgentKind::OpenCode && major_version(kind) == Some(1),
        extra: &extra,
    });

    let place = match &worktree {
        Some(tree) => format!("{} (worktree {tree}: another session works in the repo)", tilde(&repo)),
        None => tilde(&repo),
    };
    println!("Starting {} in {place}, listening for your asides.", kind.name());
    match reached {
        Ok(()) => println!(
            "  {} talk to it · {} switch sessions · notes in {}/.asides",
            pretty_shortcut(&config.shortcuts.record),
            pretty_shortcut(&config.shortcuts.next_target),
            tilde(&repo)
        ),
        Err(err) => println!("  ! Asides didn't answer ({err:#}); run `asides` here once it's installed."),
    }

    if let (true, Some(thread)) = (options.background, &session.thread) {
        println!("Running in the background: `codex resume {thread}` opens it (the Asides menu can too).");
        return Ok(());
    }
    let mut process = Process::new(&command.program);
    process.args(&command.args).envs(command.env.iter().map(|(k, v)| (k, v))).current_dir(&workdir);
    if options.background {
        return start_background(process, &session, &name);
    }
    // From here on this process is the agent, so the session lasts exactly as long as it does.
    let err = process.exec();
    Session::remove(&paths::sessions_dir(), &id);
    Err(err).with_context(|| format!("could not start {} (`{}`): is it installed?", kind.name(), command.program))
}

/// Starts a Claude Code background session; `claude attach` opens it.
fn start_background(mut process: Process, session: &Session, name: &str) -> Result<()> {
    let out = process.output();
    let out = match out {
        Ok(out) if out.status.success() => out,
        Ok(out) => {
            Session::remove(&paths::sessions_dir(), &session.id);
            bail!("claude --bg failed: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Err(err) => {
            Session::remove(&paths::sessions_dir(), &session.id);
            return Err(err).context("could not start Claude Code: is it installed?");
        }
    };
    print!("{}", String::from_utf8_lossy(&out.stdout));
    // Its process, so the session ends with it (its `asides mcp` also fills it in once it connects).
    if let Some(pid) = background_pid(name) {
        let mut session = session.clone();
        session.pid = pid;
        let _ = session.write(&paths::sessions_dir());
    }
    println!("Running in the background: `claude attach \"{name}\"` opens it (the Asides menu can too).");
    Ok(())
}

/// The process of the Claude Code session named `name`, from `claude agents --json`.
fn background_pid(name: &str) -> Option<u32> {
    for _ in 0..20 {
        let out = Process::new("claude").args(["agents", "--json"]).output().ok()?;
        let list: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
        let found = list.as_array().into_iter().flatten().find(|s| s["name"].as_str() == Some(name));
        if let Some(pid) = found.and_then(|s| s["pid"].as_u64()) {
            return u32::try_from(pid).ok();
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    None
}

/// The agent's major version, from `<program> --version` ("v2.0.30", "1.4.2").
fn major_version(kind: AgentKind) -> Option<u32> {
    let out = Process::new(kind.program()).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let version =
        text.split_whitespace().find(|w| w.trim_start_matches('v').starts_with(|c: char| c.is_ascii_digit()))?;
    version.trim_start_matches('v').split('.').next()?.parse().ok()
}

/// Starts the process that pushes session `id`'s asides into the Codex `thread`, on its own (a
/// process group of its own, so closing the terminal doesn't stop it), while `while_pid` runs, or
/// until stopped. What goes wrong goes to a log beside New Agent Session's scripts.
fn start_pusher(repo: &Path, id: &str, thread: &str, while_pid: Option<u32>) -> Result<u32> {
    let logs = std::env::temp_dir().join("asides-sessions");
    std::fs::create_dir_all(&logs)?;
    let log = std::fs::File::create(logs.join(format!("push-{id}.log")))?;
    let mut pusher = Process::new(std::env::current_exe()?);
    pusher.arg("-C").arg(repo).args(["listen", "--session", id, "--codex-thread", thread]);
    if let Some(pid) = while_pid {
        pusher.args(["--while-pid", &pid.to_string()]);
    }
    let child = pusher
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .process_group(0)
        .spawn()
        .context("could not start pushing asides to Codex")?;
    Ok(child.id())
}

/// Makes the session the app's target. The app reads its record.
fn tell_app(id: &str) -> Result<()> {
    let request = crate::request_id();
    crate::send(&Link::Session { id: id.into(), request: Some(request.clone()) })?;
    crate::wait_for_ack(&request).map(drop)
}

fn branch(worktree: &str) -> String {
    format!("asides/{}", worktree.trim_start_matches("asides-"))
}

/// Removes the worktrees asides made for sessions that have ended, when nothing was done in them.
fn prune_worktrees(repo: &Path) {
    let Ok(trees) = std::fs::read_dir(project::worktree_path(repo, "")) else { return };
    let live: Vec<String> = crate::live_sessions().into_iter().filter_map(|s| s.worktree).collect();
    for tree in trees.flatten() {
        let name = tree.file_name().to_string_lossy().into_owned();
        if tree.path().is_dir() && !live.contains(&name) {
            project::remove_worktree_if_unchanged(repo, &tree.path(), &branch(&name));
        }
    }
}
