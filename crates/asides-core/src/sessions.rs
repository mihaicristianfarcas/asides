//! The agent sessions on this Mac, found in the process table: each coding agent (`agents.rs`)
//! running in a terminal, the folder it works in, its terminal device, and the app hosting that
//! terminal, which is how an aside reaches it (`deliver.rs`). Nothing is installed in the agents and
//! nothing has to listen.
//!
//! The app hosting a session is told by the environment the agent was started with: Terminal and
//! Ghostty set `TERM_PROGRAM`; Diri sets `DIRIJOR_SESSION_ID` (and no `TERM_PROGRAM`, so a terminal's
//! own wins over one inherited from Diri). The session the user last typed into is the one whose
//! terminal device was last read from: its access time, which is how `w` tells idle time.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::agents::{AgentKind, LAUNCH_VAR};

/// Where a session's terminal is, which decides how an aside is typed into it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "app", rename_all = "lowercase")]
pub enum Host {
    /// Terminal.app: its tab is found by the terminal device (`/dev/ttys003`).
    Terminal,
    /// Ghostty: its terminal is found by working directory (Ghostty names no device).
    Ghostty,
    /// Diri: its session id.
    Diri { session: String },
    /// An app asides can't type into (Warp, iTerm, VS Code…), as `TERM_PROGRAM` names it.
    Other { name: String },
}

impl Host {
    pub fn name(&self) -> &str {
        match self {
            Host::Terminal => "Terminal",
            Host::Ghostty => "Ghostty",
            Host::Diri { .. } => "Diri",
            Host::Other { name } => name,
        }
    }

    /// asides can type into sessions there.
    pub fn reachable(&self) -> bool {
        !matches!(self, Host::Other { .. })
    }

    fn of(env: &HashMap<String, String>) -> Host {
        match env.get("TERM_PROGRAM").map(String::as_str) {
            Some("Apple_Terminal") => Host::Terminal,
            Some("ghostty") => Host::Ghostty,
            Some("iTerm.app") => Host::Other { name: "iTerm".into() },
            Some("vscode") => Host::Other { name: "VS Code".into() },
            Some("WarpTerminal") => Host::Other { name: "Warp".into() },
            Some("WezTerm") => Host::Other { name: "WezTerm".into() },
            Some(other) if !other.is_empty() => Host::Other { name: other.into() },
            _ => match env.get("DIRIJOR_SESSION_ID") {
                Some(id) if !id.is_empty() => Host::Diri { session: id.clone() },
                _ => Host::Other { name: "an unknown terminal".into() },
            },
        }
    }
}

/// An agent session: a coding agent's process with a terminal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSession {
    pub pid: i32,
    pub agent: AgentKind,
    /// The folder it works in.
    pub dir: PathBuf,
    /// Its terminal device, without `/dev/` (`ttys003`).
    pub tty: String,
    pub host: Host,
    /// When its terminal last got input (the user typing, or asides): the latest is the default
    /// target.
    #[serde(default)]
    pub last_input: Option<SystemTime>,
    /// What the agent calls the session, when it says (Claude Code's `asides-6b`).
    #[serde(default)]
    pub name: Option<String>,
    /// asides started it (with [`LAUNCH_VAR`]): the id of that launch.
    #[serde(default)]
    pub launch: Option<String>,
}

impl AgentSession {
    /// The session's id, for as long as its process lives.
    pub fn id(&self) -> String {
        self.pid.to_string()
    }

    /// The repository (or folder) it works in, by name.
    pub fn place(&self) -> String {
        repo_name(&self.dir)
    }

    /// "Claude Code (asides-6b)", "Codex (asides)".
    pub fn label(&self) -> String {
        format!("{} ({})", self.agent.name(), self.where_())
    }

    /// Its name, when the agent says it, or the repository it works in.
    fn where_(&self) -> String {
        self.name.clone().unwrap_or_else(|| self.place())
    }
}

/// The name of the git repository `dir` is in (its root folder's), or else of `dir`.
pub fn repo_name(dir: &Path) -> String {
    let root = dir.ancestors().find(|d| d.join(".git").exists()).unwrap_or(dir);
    root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| root.display().to_string())
}

/// Labels for `sessions`, told apart where two would read the same (by their app, then a number).
pub fn labels(sessions: &[AgentSession]) -> Vec<String> {
    let plain: Vec<String> = sessions.iter().map(AgentSession::label).collect();
    let mut details: Vec<String> = sessions
        .iter()
        .zip(&plain)
        .map(|(s, label)| match plain.iter().filter(|l| *l == label).count() {
            1 => s.where_(),
            _ => format!("{} in {}", s.where_(), s.host.name()),
        })
        .collect();
    let agents: Vec<&str> = sessions.iter().map(|s| s.agent.name()).collect();
    for i in 0..details.len() {
        let same: Vec<usize> =
            (0..details.len()).filter(|&j| details[j] == details[i] && agents[j] == agents[i]).collect();
        if same.len() > 1 {
            for (n, &j) in same.iter().enumerate() {
                details[j] = format!("{}, {}", details[j], n + 1);
            }
        }
    }
    agents.iter().zip(details).map(|(agent, detail)| format!("{agent} ({detail})")).collect()
}

/// The session an aside goes to by default: the reachable one whose terminal got input last.
pub fn latest(sessions: &[AgentSession]) -> Option<&AgentSession> {
    sessions.iter().filter(|s| s.host.reachable()).max_by_key(|s| s.last_input)
}

/// A process, as the process table lists it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Proc {
    pub pid: i32,
    pub ppid: i32,
    /// `ttys003`, or `None` for `??`.
    pub tty: Option<String>,
    /// The executable (`comm`).
    pub executable: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

/// Arguments that make an agent's process something other than an interactive session (a
/// one-off run, a server, a subcommand).
fn interactive(agent: AgentKind, args: &[String]) -> bool {
    let first = args.iter().skip(1).find(|a| !a.starts_with('-') || matches!(a.as_str(), "-p" | "--print"));
    let not = match agent {
        AgentKind::Claude => &["-p", "--print", "agents", "mcp", "doctor", "logs", "stop", "rm", "update"][..],
        AgentKind::Codex => &["exec", "e", "app-server", "mcp", "mcp-server", "login", "logout", "proto", "apply"][..],
        AgentKind::OpenCode => &["run", "serve", "mcp", "debug", "session", "auth", "models", "export"][..],
        _ => &[][..],
    };
    let flag = args.iter().any(|a| matches!(a.as_str(), "-p" | "--print") && agent == AgentKind::Claude);
    !flag && first.is_none_or(|first| !not.contains(&first.as_str()))
}

/// The agent sessions among `procs`: agents' processes with a terminal, one per terminal (the
/// first, not processes it started). Their folders and input times come after
/// ([`AgentSession::dir`] is empty here).
pub fn sessions_of(procs: &[Proc]) -> Vec<AgentSession> {
    let agent = |p: &Proc| {
        let args: Vec<&str> = p.args.iter().map(String::as_str).collect();
        AgentKind::of_process(&p.executable, &args).filter(|&a| interactive(a, &p.args))
    };
    let by_pid: HashMap<i32, &Proc> = procs.iter().map(|p| (p.pid, p)).collect();
    let mut found = Vec::new();
    for p in procs {
        let (Some(kind), Some(tty)) = (agent(p), &p.tty) else { continue };
        // An agent run by another agent in the same terminal (a subagent, a helper) isn't a session.
        let parent_is_agent = std::iter::successors(by_pid.get(&p.ppid), |q| by_pid.get(&q.ppid))
            .take(8)
            .any(|q| q.tty.as_ref() == Some(tty) && agent(q).is_some());
        if parent_is_agent {
            continue;
        }
        found.push(AgentSession {
            pid: p.pid,
            agent: kind,
            dir: PathBuf::new(),
            tty: tty.clone(),
            host: Host::of(&p.env),
            last_input: None,
            name: None,
            launch: p.env.get(LAUNCH_VAR).cloned(),
        });
    }
    found
}

/// Parses `ps -A -ww -o pid=,ppid=,tty=,comm=`.
pub fn parse_ps(output: &str) -> Vec<Proc> {
    output
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let pid = words.next()?.parse().ok()?;
            let ppid = words.next()?.parse().ok()?;
            let tty = words.next()?;
            let executable = words.collect::<Vec<_>>().join(" ");
            let tty = (tty != "??" && !tty.is_empty()).then(|| tty.to_string());
            Some(Proc { pid, ppid, tty, executable, ..Proc::default() })
        })
        .collect()
}

/// Splits what `ps -E -ww -o command=` prints for a process (its arguments, then its environment)
/// with what `ps -ww -o command=` prints (its arguments): the variables (`NAME=value` words after
/// the arguments; values with spaces are cut at the first, which is enough for the names asides
/// reads).
pub fn parse_env(with_env: &str, args: &str) -> HashMap<String, String> {
    let rest = with_env.strip_prefix(args.trim_end()).unwrap_or(with_env);
    rest.split_whitespace()
        .filter_map(|word| {
            let (name, value) = word.split_once('=')?;
            let valid = !name.is_empty() && name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric());
            valid.then(|| (name.to_string(), value.to_string()))
        })
        .collect()
}

/// Parses `lsof -a -d cwd -Fpn -p …`: each process's working directory.
pub fn parse_lsof_cwd(output: &str) -> HashMap<i32, PathBuf> {
    let mut dirs = HashMap::new();
    let mut pid = None;
    for line in output.lines() {
        if let Some(p) = line.strip_prefix('p') {
            pid = p.parse().ok();
        } else if let (Some(n), Some(p)) = (line.strip_prefix('n'), pid) {
            dirs.insert(p, PathBuf::from(n));
        }
    }
    dirs
}

/// The agent sessions running now, with their folders, last input and names. Takes a few tens of
/// milliseconds (`ps`, `lsof`).
pub fn discover() -> Vec<AgentSession> {
    let run = |program: &str, args: &[&str]| {
        Command::new(program)
            .args(args)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    };
    let mut procs = parse_ps(&run("/bin/ps", &["-A", "-ww", "-o", "pid=,ppid=,tty=,comm="]));
    // Arguments and environments only for agents' processes (and scripts', which may be agents).
    let wanted: Vec<i32> = procs
        .iter()
        .filter(|p| {
            p.tty.is_some()
                && (crate::agents::is_runtime(&p.executable) || AgentKind::of_process(&p.executable, &[]).is_some())
        })
        .map(|p| p.pid)
        .collect();
    if wanted.is_empty() {
        return Vec::new();
    }
    let list = wanted.iter().map(i32::to_string).collect::<Vec<_>>().join(",");
    let args_out = run("/bin/ps", &["-ww", "-o", "pid=,command=", "-p", &list]);
    let env_out = run("/bin/ps", &["-E", "-ww", "-o", "pid=,command=", "-p", &list]);
    let by_pid = |out: &str| -> HashMap<i32, String> {
        out.lines()
            .filter_map(|l| {
                let l = l.trim_start();
                let (pid, rest) = l.split_once(char::is_whitespace)?;
                Some((pid.parse().ok()?, rest.trim_start().to_string()))
            })
            .collect()
    };
    let (args, envs) = (by_pid(&args_out), by_pid(&env_out));
    for p in procs.iter_mut().filter(|p| wanted.contains(&p.pid)) {
        let line = args.get(&p.pid).cloned().unwrap_or_default();
        p.env = envs.get(&p.pid).map(|e| parse_env(e, &line)).unwrap_or_default();
        p.args = line.split_whitespace().map(String::from).collect();
    }
    let mut sessions = sessions_of(&procs);
    if sessions.is_empty() {
        return sessions;
    }
    let list = sessions.iter().map(|s| s.pid.to_string()).collect::<Vec<_>>().join(",");
    let dirs = parse_lsof_cwd(&run("/usr/sbin/lsof", &["-a", "-d", "cwd", "-Fpn", "-p", &list]));
    for s in &mut sessions {
        s.dir = dirs.get(&s.pid).cloned().unwrap_or_default();
        s.last_input = std::fs::metadata(format!("/dev/{}", s.tty)).and_then(|m| m.accessed()).ok();
        if s.agent == AgentKind::Claude {
            s.name = claude_name(s.pid);
        }
    }
    sessions.retain(|s| !s.dir.as_os_str().is_empty());
    sessions
}

/// Folders a new session can start in, most likely first, at most `limit`: those of the sessions
/// running, the user's recent ones, then the projects Claude Code (`~/.claude.json`) and Codex
/// (`~/.codex/config.toml`) know, most recently changed first. Each is its repository's root, once.
pub fn known_folders(home: &Path, sessions: &[AgentSession], recent: &[PathBuf], limit: usize) -> Vec<PathBuf> {
    let root = |dir: &Path| dir.ancestors().find(|d| d.join(".git").exists()).unwrap_or(dir).to_path_buf();
    let mut known: Vec<PathBuf> = Vec::new();
    if let Ok(text) = std::fs::read_to_string(home.join(".claude.json"))
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
        && let Some(projects) = value.get("projects").and_then(|p| p.as_object())
    {
        known.extend(projects.keys().map(PathBuf::from));
    }
    if let Ok(text) = std::fs::read_to_string(home.join(".codex/config.toml")) {
        known.extend(
            text.lines().filter_map(|l| l.trim().strip_prefix("[projects.\"")?.strip_suffix("\"]").map(PathBuf::from)),
        );
    }
    let changed = |d: &Path| std::fs::metadata(d).and_then(|m| m.modified()).ok();
    known.sort_by_key(|d| std::cmp::Reverse(changed(d)));
    let mut folders: Vec<PathBuf> = Vec::new();
    let candidates = sessions.iter().map(|s| s.dir.clone()).chain(recent.iter().cloned()).chain(known);
    for dir in candidates {
        // Not the home folder itself, nor a scratch folder that's gone.
        if !dir.is_dir() || dir == home || dir.starts_with("/private/tmp") || dir.starts_with("/tmp") {
            continue;
        }
        let dir = root(&dir);
        if !folders.contains(&dir) {
            folders.push(dir);
        }
        if folders.len() >= limit {
            break;
        }
    }
    folders
}

/// The name Claude Code gives its session (`~/.claude/sessions/<pid>.json`).
fn claude_name(pid: i32) -> Option<String> {
    let home = std::env::var_os("HOME")?;
    let file = Path::new(&home).join(".claude/sessions").join(format!("{pid}.json"));
    let value: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()?;
    value.get("name").and_then(|n| n.as_str()).filter(|n| !n.is_empty()).map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn proc(pid: i32, ppid: i32, tty: Option<&str>, args: &str, env: &[(&str, &str)]) -> Proc {
        let args: Vec<String> = args.split(' ').map(String::from).collect();
        Proc {
            pid,
            ppid,
            tty: tty.map(String::from),
            executable: args[0].clone(),
            args,
            env: env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    }

    #[test]
    fn finds_sessions_and_their_apps() {
        let procs = [
            proc(10, 1, Some("ttys001"), "claude", &[("DIRIJOR_SESSION_ID", "s_1"), ("TERM", "xterm")]),
            proc(11, 10, Some("ttys001"), "claude --print hi", &[]),
            proc(20, 1, Some("ttys002"), "codex", &[("TERM_PROGRAM", "Apple_Terminal")]),
            proc(21, 20, Some("ttys002"), "codex", &[("TERM_PROGRAM", "Apple_Terminal")]),
            proc(30, 1, None, "codex app-server", &[]),
            proc(40, 1, Some("ttys004"), "opencode", &[("TERM_PROGRAM", "ghostty"), ("DIRIJOR_SESSION_ID", "s_2")]),
            proc(50, 1, Some("ttys005"), "claude -p hello", &[("TERM_PROGRAM", "ghostty")]),
            proc(60, 1, Some("ttys006"), "node /opt/homebrew/bin/gemini", &[("TERM_PROGRAM", "WarpTerminal")]),
            proc(70, 1, Some("ttys007"), "codex exec fix", &[("TERM_PROGRAM", "ghostty")]),
            proc(80, 1, Some("ttys008"), "zsh", &[]),
            proc(
                90,
                1,
                Some("ttys009"),
                "/Users/me/.local/share/claude/versions/2.1.303",
                &[("TERM_PROGRAM", "ghostty")],
            ),
        ];
        let found = sessions_of(&procs);
        let summary: Vec<(i32, AgentKind, Host)> = found.iter().map(|s| (s.pid, s.agent, s.host.clone())).collect();
        assert_eq!(
            summary,
            [
                (10, AgentKind::Claude, Host::Diri { session: "s_1".into() }),
                (20, AgentKind::Codex, Host::Terminal),
                // A terminal's own program wins over a Diri id it inherited.
                (40, AgentKind::OpenCode, Host::Ghostty),
                (60, AgentKind::Gemini, Host::Other { name: "Warp".into() }),
                (90, AgentKind::Claude, Host::Ghostty),
            ]
        );
    }

    #[test]
    fn the_default_is_the_session_typed_into_last() {
        let at = |secs: u64| Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs));
        let session = |pid: i32, host: Host, last_input| AgentSession {
            pid,
            agent: AgentKind::Claude,
            dir: PathBuf::from("/r/app"),
            tty: format!("ttys00{pid}"),
            host,
            last_input,
            name: None,
            launch: None,
        };
        let sessions = [
            session(1, Host::Terminal, at(100)),
            session(2, Host::Other { name: "Warp".into() }, at(300)),
            session(3, Host::Ghostty, at(200)),
        ];
        assert_eq!(latest(&sessions).map(|s| s.pid), Some(3), "Warp can't be typed into");
        assert_eq!(
            labels(&sessions),
            ["Claude Code (app in Terminal)", "Claude Code (app in Warp)", "Claude Code (app in Ghostty)"]
        );
        let same = [session(1, Host::Terminal, None), session(4, Host::Terminal, None)];
        assert_eq!(labels(&same), ["Claude Code (app in Terminal, 1)", "Claude Code (app in Terminal, 2)"]);
        assert_eq!(labels(&sessions[..1]), ["Claude Code (app)"]);
    }

    #[test]
    fn parses_the_process_table() {
        let ps = "  95393 48885 ttys001  claude\n 36105     1 ??       /Users/me/.codex/bin/codex\n";
        let procs = parse_ps(ps);
        assert_eq!(procs.len(), 2);
        assert_eq!((procs[0].pid, procs[0].ppid, procs[0].tty.as_deref()), (95393, 48885, Some("ttys001")));
        assert_eq!((procs[1].tty.as_deref(), procs[1].executable.as_str()), (None, "/Users/me/.codex/bin/codex"));

        let env = parse_env(
            "claude --resume x TERM=xterm-256color DIRIJOR_SESSION_ID=s_0dd5 PATH=/bin:/usr/bin LANG=en_US.UTF-8",
            "claude --resume x",
        );
        assert_eq!(env.get("DIRIJOR_SESSION_ID").map(String::as_str), Some("s_0dd5"));
        assert_eq!(env.get("TERM").map(String::as_str), Some("xterm-256color"));
        assert!(!env.contains_key("--resume"));

        let lsof = "p95393\nfcwd\nn/Users/me/Developer/asides\np20\nfcwd\nn/tmp/x y\n";
        let dirs = parse_lsof_cwd(lsof);
        assert_eq!(dirs.get(&20), Some(&PathBuf::from("/tmp/x y")));
        assert_eq!(dirs.len(), 2);
    }

    /// Lists the sessions running on this Mac: `cargo test -p asides-core discovers -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn discovers_the_sessions_running() {
        for session in discover() {
            println!("{session:?} → {}", session.label());
        }
    }

    #[test]
    fn suggests_folders_from_sessions_recents_and_the_agents_projects() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        let dir = |name: &str| {
            let d = home.join(name);
            std::fs::create_dir_all(d.join(".git")).unwrap();
            d
        };
        let (shop, blog, api, cli) = (dir("shop"), dir("blog"), dir("api"), dir("cli"));
        std::fs::create_dir_all(shop.join("web")).unwrap();
        std::fs::write(
            home.join(".claude.json"),
            format!(r#"{{"projects": {{"{}": {{}}, "{}": {{}}, "/gone": {{}}}}}}"#, api.display(), shop.display()),
        )
        .unwrap();
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::write(
            home.join(".codex/config.toml"),
            format!("[projects.\"{}\"]\ntrust_level = \"trusted\"\n", cli.display()),
        )
        .unwrap();
        let running = AgentSession {
            pid: 1,
            agent: AgentKind::Claude,
            dir: shop.join("web"),
            tty: "ttys001".into(),
            host: Host::Terminal,
            last_input: None,
            name: None,
            launch: None,
        };
        let folders = known_folders(&home, &[running], std::slice::from_ref(&blog), 10);
        assert_eq!(folders[..2], [shop.clone(), blog.clone()], "the running session's repo, then the recent ones");
        assert!(folders.contains(&api) && folders.contains(&cli), "{folders:?}");
        assert_eq!(folders.len(), 4);
        assert_eq!(known_folders(&home, &[], &[blog], 1).len(), 1);
    }

    #[test]
    fn names_the_repository() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("shop");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("web/src")).unwrap();
        assert_eq!(repo_name(&repo.join("web/src")), "shop");
        assert_eq!(repo_name(Path::new("/tmp")), "tmp");
    }
}
