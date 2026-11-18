//! The agent sessions on this Mac, found in the process table: each coding agent (`agents.rs`)
//! running in a terminal, the folder it works in, its terminal device, and the app hosting that
//! terminal, which is how an aside reaches it (`deliver.rs`). Nothing is installed in the agents and
//! nothing has to listen.
//!
//! The app hosting a session is told by the environment the agent was started with: Terminal,
//! Ghostty and iTerm set `TERM_PROGRAM`; Diri sets `DIRIJOR_SESSION_ID` (and no `TERM_PROGRAM`, so a
//! terminal's own wins over one inherited from Diri). What takes text itself comes before the app
//! showing it (`Host::taking_text`): a pane of tmux, Zellij or GNU screen in any terminal, a terminal
//! of Neovim, WezTerm, kitty with its remote control on. Which of those runs the agent, when its
//! variables say several, its processes tell: the nearest of them above it. The session the user
//! last typed into is the one whose terminal device was last read from: its access time, which is
//! how `w` tells idle time.
//!
//! A session is named as its agent titles it from what was said in it (or as the user renamed it):
//! Claude Code in its transcript, Codex in its thread index, OpenCode in its database.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agents::{AgentKind, LAUNCH_VAR};

#[cfg(windows)]
#[path = "sessions_windows.rs"]
pub mod windows;

#[cfg(target_os = "linux")]
#[path = "sessions_linux.rs"]
pub mod linux;

#[path = "sessions_proc.rs"]
pub mod procfs;

#[path = "sessions_wsl.rs"]
pub mod wsl;

/// Where a session's terminal is, which decides how an aside is typed into it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "app", rename_all = "lowercase")]
pub enum Host {
    /// Terminal.app: its tab is found by the terminal device (`/dev/ttys003`).
    Terminal,
    /// Ghostty: its terminal is found by working directory (Ghostty names no device).
    Ghostty,
    /// iTerm: its session is found by the terminal device, as Terminal's tab.
    ITerm,
    /// Diri: its session id.
    Diri { session: String },
    /// An app asides can't type into (Warp, VS Code…), as `TERM_PROGRAM` names it.
    Other { name: String },
    /// A console on Windows, in whichever app shows it (Windows Terminal, a console window, VS
    /// Code's terminal…): asides types into the console itself (`deliver.rs`).
    Console { name: String },
    /// A pane of tmux, which takes text itself (its socket, and the pane), in whatever terminal
    /// shows it.
    Tmux { socket: String, pane: String },
    /// A pane of Zellij (its session, and the pane's number), in whatever terminal shows it.
    Zellij { session: String, pane: String },
    /// A window of GNU screen (its session, `STY`, and the window's number), in whatever terminal
    /// shows it.
    Screen { session: String, window: String },
    /// A terminal of Neovim (`:terminal`), by its server's address: the one whose job the agent
    /// runs under.
    Neovim { server: String },
    /// Linux: a tab of Konsole, which takes text itself over D-Bus.
    Konsole { service: String, session: String },
    /// A pane of WezTerm, which takes text itself (`wezterm cli`).
    WezTerm { pane: String, socket: Option<String> },
    /// A window of kitty, with its remote control on (`kitten @`).
    Kitty { listen: String, window: String },
    /// Linux: any other terminal, typed into through its pty (`asides-keys`), if the system lets
    /// asides (`may_type`).
    Pty { name: String, may_type: bool },
    /// Windows: a terminal in one of WSL's Linux distributions (`sessions_wsl.rs`), typed into through
    /// the console of the `wsl.exe` showing it (`console`, a Windows process), or by the multiplexer
    /// or editor running it there (`inner`), whose command runs through `wsl.exe`. Windows' drives
    /// are in `mount` there (`/mnt/`).
    Wsl { distro: String, console: Option<i32>, mount: String, inner: Option<Box<Host>> },
}

impl Host {
    pub fn name(&self) -> &str {
        match self {
            Host::Terminal => "Terminal",
            Host::Ghostty => "Ghostty",
            Host::ITerm => "iTerm",
            Host::Diri { .. } => "Diri",
            Host::Other { name } | Host::Console { name } | Host::Pty { name, .. } => name,
            Host::Tmux { .. } => "tmux",
            Host::Zellij { .. } => "Zellij",
            Host::Screen { .. } => "screen",
            Host::Neovim { .. } => "Neovim",
            Host::Konsole { .. } => "Konsole",
            Host::WezTerm { .. } => "WezTerm",
            Host::Kitty { .. } => "kitty",
            Host::Wsl { inner: Some(inner), .. } => inner.name(),
            Host::Wsl { .. } => "WSL",
        }
    }

    /// asides can type into sessions there.
    pub fn reachable(&self) -> bool {
        !matches!(
            self,
            Host::Other { .. } | Host::Pty { may_type: false, .. } | Host::Wsl { console: None, inner: None, .. }
        )
    }

    /// The app hosting a terminal on a Mac, from the environment its programs got.
    pub fn of(env: &HashMap<String, String>) -> Host {
        Host::inside(env, None)
    }

    /// [`Host::of`], knowing which multiplexer or editor runs the agent (`innermost`). What takes
    /// text itself comes first (`taking_text`): a pane of tmux is reached in Warp or VS Code too,
    /// which asides can't type into.
    pub fn inside(env: &HashMap<String, String>, innermost: Option<&str>) -> Host {
        if let Some(host) = Host::taking_text(env, innermost) {
            return host;
        }
        match env.get("TERM_PROGRAM").map(String::as_str) {
            Some("Apple_Terminal") => Host::Terminal,
            Some("ghostty") => Host::Ghostty,
            Some("iTerm.app") => Host::ITerm,
            Some("vscode") => Host::Other { name: "VS Code".into() },
            Some("WarpTerminal") => Host::Other { name: "Warp".into() },
            Some(other) if !other.is_empty() => Host::Other { name: other.into() },
            _ => match env.get("DIRIJOR_SESSION_ID") {
                Some(id) if !id.is_empty() => Host::Diri { session: id.clone() },
                // kitty sets no `TERM_PROGRAM`; without its remote control it takes no text.
                _ if env.get("KITTY_WINDOW_ID").is_some_and(|id| !id.is_empty()) => {
                    Host::Other { name: "kitty".into() }
                }
                _ => Host::Other { name: "an unknown terminal".into() },
            },
        }
    }

    /// A terminal that takes text itself, by what its programs were given: a pane of tmux, Zellij or
    /// GNU screen, a terminal of Neovim (`NVIM`), a pane of WezTerm, a window of kitty with its
    /// remote control on (`listen_on`, which sets `KITTY_LISTEN_ON`). A multiplexer's programs got
    /// the variables of the terminal that started it too, so the one running the agent
    /// (`innermost`, one of [`HOSTING`]) comes first, then the multiplexers.
    pub fn taking_text(env: &HashMap<String, String>, innermost: Option<&str>) -> Option<Host> {
        let get = |name: &str| env.get(name).filter(|v| !v.is_empty());
        let tmux = || {
            let (tmux, pane) = (get("TMUX")?, get("TMUX_PANE")?);
            let socket = tmux.split(',').next().unwrap_or_default().to_string();
            Some(Host::Tmux { socket, pane: pane.clone() })
        };
        let zellij = || {
            let (session, pane) = (get("ZELLIJ_SESSION_NAME")?, get("ZELLIJ_PANE_ID")?);
            Some(Host::Zellij { session: session.clone(), pane: pane.clone() })
        };
        let screen = || Some(Host::Screen { session: get("STY")?.clone(), window: get("WINDOW")?.clone() });
        let neovim = || Some(Host::Neovim { server: get("NVIM")?.clone() });
        let by_name = |name: &str| match name {
            "tmux" => tmux(),
            "zellij" => zellij(),
            "screen" => screen(),
            "nvim" => neovim(),
            _ => None,
        };
        let wezterm =
            || Some(Host::WezTerm { pane: get("WEZTERM_PANE")?.clone(), socket: get("WEZTERM_UNIX_SOCKET").cloned() });
        let kitty = || {
            let (listen, window) = (get("KITTY_LISTEN_ON")?, get("KITTY_WINDOW_ID")?);
            Some(Host::Kitty { listen: listen.clone(), window: window.clone() })
        };
        innermost
            .and_then(by_name)
            .or_else(tmux)
            .or_else(zellij)
            .or_else(screen)
            .or_else(neovim)
            .or_else(wezterm)
            .or_else(kitty)
    }

    /// The program that types into this terminal, where the session's own surroundings say which:
    /// tmux's (its server's, the process `TMUX` names), Zellij's, screen's and Neovim's (the one
    /// running the agent, `pid`), WezTerm's command (beside the app's, in `WEZTERM_EXECUTABLE_DIR`),
    /// kitty's `kitten` (beside the process `KITTY_PID` names). A Mac gives apps a bare `PATH`,
    /// where none of them is.
    pub fn tool(&self, pid: i32, env: &HashMap<String, String>, procs: &[Proc]) -> Option<PathBuf> {
        let get = |name: &str| env.get(name).filter(|v| !v.is_empty());
        // With its folder (`has_root`: WSL's paths have no drive, which Windows' `is_absolute` asks for).
        let absolute = |p: &Proc| {
            let named = Path::new(&p.executable).to_path_buf();
            if named.has_root() { Some(named) } else { executable_path(p.pid) }
        };
        let program = |pid: Option<&str>| {
            let pid: i32 = pid?.parse().ok()?;
            absolute(procs.iter().find(|p| p.pid == pid)?)
        };
        let hosting = |name: &str| absolute(innermost(procs, pid).filter(|(n, _)| *n == name)?.1);
        match self {
            Host::Tmux { .. } => program(get("TMUX")?.split(',').nth(1)).or_else(|| hosting("tmux")),
            Host::Zellij { .. } => hosting("zellij"),
            Host::Screen { .. } => hosting("screen"),
            Host::Neovim { .. } => hosting("nvim"),
            Host::WezTerm { .. } => get("WEZTERM_EXECUTABLE_DIR")
                .map(PathBuf::from)
                .or_else(|| Some(Path::new(get("WEZTERM_EXECUTABLE")?).parent()?.to_path_buf()))
                .map(|dir| dir.join("wezterm")),
            Host::Kitty { .. } => Some(program(get("KITTY_PID").map(String::as_str))?.with_file_name("kitten")),
            _ => None,
        }
    }
}

/// Where process `pid`'s program is, when `ps` names it only as it was started (`tmux`, found on
/// the terminal's `PATH`, which asides' own isn't).
#[cfg(target_os = "macos")]
fn executable_path(pid: i32) -> Option<PathBuf> {
    let mut buffer = vec![0u8; 4096];
    // SAFETY: the kernel writes at most `buffer.len()` bytes into it and says how many.
    let len = unsafe { libc::proc_pidpath(pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    let path = std::str::from_utf8(buffer.get(..usize::try_from(len).ok()?)?).ok()?;
    (!path.is_empty()).then(|| PathBuf::from(path))
}

#[cfg(not(target_os = "macos"))]
fn executable_path(_pid: i32) -> Option<PathBuf> {
    None
}

/// The multiplexers and the editor that run terminals of their own, which take text themselves, by
/// their programs' names (GNU screen's server is `SCREEN`).
pub const HOSTING: [&str; 4] = ["tmux", "zellij", "screen", "nvim"];

/// The nearest of [`HOSTING`] above process `pid` (its name, and its process): the one whose
/// terminal it runs in.
pub fn innermost(procs: &[Proc], pid: i32) -> Option<(&'static str, &Proc)> {
    let by_pid: HashMap<i32, &Proc> = procs.iter().map(|p| (p.pid, p)).collect();
    let start = by_pid.get(&pid)?;
    std::iter::successors(by_pid.get(&start.ppid), |q| by_pid.get(&q.ppid)).take(32).find_map(|q| {
        let base = q.executable.rsplit(['/', '\\']).next().unwrap_or(&q.executable).to_ascii_lowercase();
        let name = base.strip_suffix(".exe").unwrap_or(&base);
        HOSTING.iter().find(|h| **h == name).map(|h| (*h, *q))
    })
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
    /// What the agent calls the session, when it says: the title it gave it from what was said
    /// ("Fix the login redirect"), or Claude Code's name for it until then (`asides-6b`).
    #[serde(default)]
    pub name: Option<String>,
    /// asides started it (with [`LAUNCH_VAR`]): the id of that launch.
    #[serde(default)]
    pub launch: Option<String>,
    /// The program that types into its terminal, when the session says which ([`Host::tool`]).
    #[serde(default)]
    pub tool: Option<PathBuf>,
}

impl AgentSession {
    /// The session's id, for as long as its process lives: its process's, after its distribution's
    /// in WSL (`Ubuntu:4242`), whose processes are numbered apart from Windows'.
    pub fn id(&self) -> String {
        match &self.host {
            Host::Wsl { distro, .. } => format!("{distro}:{}", self.pid),
            _ => self.pid.to_string(),
        }
    }

    /// The process of this system showing the session: its own, or in WSL its terminal's `wsl.exe`.
    pub fn local_pid(&self) -> Option<i32> {
        match self.host {
            Host::Wsl { console, .. } => console,
            _ => Some(self.pid),
        }
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

/// The agent sessions among `procs`: agents' processes with a terminal, one per terminal (the
/// first, not processes it started). Their folders and input times come after
/// ([`AgentSession::dir`] is empty here).
pub fn sessions_of(procs: &[Proc]) -> Vec<AgentSession> {
    #[cfg(windows)]
    return sessions_with(procs, |env, _| windows::host_of(env));
    #[cfg(target_os = "linux")]
    return sessions_with(procs, linux::host_inside);
    #[cfg(target_os = "macos")]
    sessions_with(procs, Host::inside)
}

/// [`sessions_of`], with the apps hosting the terminals told by `host_of` (each system's way), from
/// an agent's environment and the nearest of [`HOSTING`] above it.
pub fn sessions_with(
    procs: &[Proc],
    host_of: impl Fn(&HashMap<String, String>, Option<&str>) -> Host,
) -> Vec<AgentSession> {
    let agent = |p: &Proc| {
        let args: Vec<&str> = p.args.iter().map(String::as_str).collect();
        AgentKind::of_process(&p.executable, &args).filter(|&a| a.is_session(&p.args))
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
            host: host_of(&p.env, innermost(procs, p.pid).map(|(name, _)| name)),
            last_input: None,
            name: None,
            launch: p.env.get(LAUNCH_VAR).cloned(),
            tool: None,
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
#[cfg(windows)]
pub fn discover() -> Vec<AgentSession> {
    windows::discover()
}

/// The agent sessions running now, with their folders, last input and names: from `/proc`.
#[cfg(target_os = "linux")]
pub fn discover() -> Vec<AgentSession> {
    linux::discover()
}

/// The agent sessions running now, with their folders, last input and names. Takes a few tens of
/// milliseconds (`ps`, `lsof`).
#[cfg(target_os = "macos")]
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
    let wanted: Vec<i32> =
        procs.iter().filter(|p| p.tty.is_some() && AgentKind::may_be(&p.executable)).map(|p| p.pid).collect();
    if wanted.is_empty() {
        return Vec::new();
    }
    // Each process's arguments, and its environment after them.
    let read = |pids: &[i32]| -> HashMap<i32, (String, HashMap<String, String>)> {
        let list = pids.iter().map(i32::to_string).collect::<Vec<_>>().join(",");
        let by_pid = |out: &str| -> HashMap<i32, String> {
            out.lines()
                .filter_map(|l| {
                    let l = l.trim_start();
                    let (pid, rest) = l.split_once(char::is_whitespace)?;
                    Some((pid.parse().ok()?, rest.trim_start().to_string()))
                })
                .collect()
        };
        let args = by_pid(&run("/bin/ps", &["-ww", "-o", "pid=,command=", "-p", &list]));
        let envs = by_pid(&run("/bin/ps", &["-E", "-ww", "-o", "pid=,command=", "-p", &list]));
        args.into_iter()
            .map(|(pid, line)| {
                let env = envs.get(&pid).map(|e| parse_env(e, &line)).unwrap_or_default();
                (pid, (line, env))
            })
            .collect()
    };
    let read_wanted = read(&wanted);
    for p in procs.iter_mut().filter(|p| wanted.contains(&p.pid)) {
        let (line, env) = read_wanted.get(&p.pid).cloned().unwrap_or_default();
        p.env = env;
        p.args = line.split_whitespace().map(String::from).collect();
    }
    // An agent that titles its process (Pi, Kimi Code) writes over what `ps` reads of its
    // environment: its shell's, which started it in the same terminal, says the same of it. (One a
    // multiplexer started itself has none to ask.)
    let tty_of: HashMap<i32, Option<String>> = procs.iter().map(|p| (p.pid, p.tty.clone())).collect();
    let untitled: Vec<(i32, i32)> = procs
        .iter()
        .filter(|p| p.env.is_empty() && wanted.contains(&p.pid) && tty_of.get(&p.ppid) == Some(&p.tty))
        .filter(|p| {
            AgentKind::of_process(&p.executable, &p.args.iter().map(String::as_str).collect::<Vec<_>>()).is_some()
        })
        .map(|p| (p.pid, p.ppid))
        .collect();
    if !untitled.is_empty() {
        let parents = read(&untitled.iter().map(|(_, ppid)| *ppid).collect::<Vec<_>>());
        for (pid, ppid) in untitled {
            if let (Some(p), Some((_, env))) = (procs.iter_mut().find(|p| p.pid == pid), parents.get(&ppid)) {
                p.env = env.clone();
            }
        }
    }
    let mut sessions = sessions_of(&procs);
    if sessions.is_empty() {
        return sessions;
    }
    for s in &mut sessions {
        s.tool = procs.iter().find(|p| p.pid == s.pid).and_then(|p| s.host.tool(s.pid, &p.env, &procs));
    }
    let list = sessions.iter().map(|s| s.pid.to_string()).collect::<Vec<_>>().join(",");
    let dirs = parse_lsof_cwd(&run("/usr/sbin/lsof", &["-a", "-d", "cwd", "-Fpn", "-p", &list]));
    for s in &mut sessions {
        s.dir = dirs.get(&s.pid).cloned().unwrap_or_default();
    }
    sessions.retain(|s| !s.dir.as_os_str().is_empty());
    // Codex's and OpenCode's sessions are told by when they began.
    let started = match sessions.iter().any(|s| matches!(s.agent, AgentKind::Codex | AgentKind::OpenCode)) {
        true => parse_started(&run("/bin/ps", &["-o", "pid=,etime=", "-p", &list]), SystemTime::now()),
        false => HashMap::new(),
    };
    if let Some(home) = std::env::home_dir() {
        let args: HashMap<i32, &[String]> = procs.iter().map(|p| (p.pid, &p.args[..])).collect();
        let names: Vec<Option<String>> = sessions
            .iter()
            .map(|s| name(&home, s, &sessions, started.get(&s.pid).copied(), args.get(&s.pid).copied().unwrap_or(&[])))
            .collect();
        for (s, name) in sessions.iter_mut().zip(names) {
            s.name = name;
        }
    }
    read_input_times(&mut sessions);
    sessions
}

/// Parses `ps -o pid=,etime=`: when each process started (it has run for `[[dd-]hh:]mm:ss`).
pub fn parse_started(output: &str, now: SystemTime) -> HashMap<i32, SystemTime> {
    output
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let pid = words.next()?.parse().ok()?;
            let elapsed = words.next()?;
            let (days, clock) = match elapsed.split_once('-') {
                Some((days, clock)) => (days.parse::<u64>().ok()?, clock),
                None => (0, elapsed),
            };
            let parts: Vec<u64> = clock.split(':').map(|p| p.parse().ok()).collect::<Option<_>>()?;
            if !(2..=3).contains(&parts.len()) {
                return None;
            }
            let seconds = parts.iter().fold(0, |sum, p| sum * 60 + p);
            Some((pid, now.checked_sub(Duration::from_secs(days * 86_400 + seconds))?))
        })
        .collect()
}

/// Reads when each session's terminal last got input (its device's access time) again: which one
/// was typed into last changes with every key, and reading it takes microseconds, where looking
/// for the sessions takes tens of milliseconds.
pub fn read_input_times(sessions: &mut [AgentSession]) {
    #[cfg(windows)]
    return windows::read_input_times(sessions);
    #[cfg(not(windows))]
    for s in sessions {
        s.last_input = std::fs::metadata(format!("/dev/{}", s.tty)).and_then(|m| m.accessed()).ok();
    }
}

/// Windows: the sessions in the terminal whose window is in front (its process), first.
#[cfg(windows)]
pub fn in_front(sessions: &mut [AgentSession], front: i32) {
    windows::in_front(sessions, front)
}

/// The folders agents write a file in as each of their sessions starts (with whether to look in
/// the folders inside): Claude Code's `~/.claude/sessions/<pid>.json`, Codex's
/// `~/.codex/sessions/<date>/rollout-….jsonl`. A file new there is a session to look for.
pub fn start_folders(home: &Path) -> [(PathBuf, bool); 2] {
    [(home.join(".claude/sessions"), false), (home.join(".codex/sessions"), true)]
}

/// The session files seen in the start folders: one seen for the first time is a session started.
/// Its later writes (the session working) aren't.
#[derive(Debug, Default)]
pub struct Starts {
    seen: HashSet<PathBuf>,
}

impl Starts {
    /// `path` changed: whether it's a session file not seen before.
    pub fn started(&mut self, path: &Path) -> bool {
        let session_file = matches!(path.extension().and_then(|e| e.to_str()), Some("json" | "jsonl"));
        session_file && self.seen.insert(path.to_path_buf())
    }
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
        known.extend(codex_projects(&text));
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

/// The folders Codex's `config.toml` has settings for (`[projects."/Users/me/shop"]`; on Windows
/// `[projects.'C:\Users\me\shop']`). Read line by line when it doesn't parse as a whole.
fn codex_projects(text: &str) -> Vec<PathBuf> {
    if let Ok(config) = text.parse::<toml::Table>() {
        let projects = config.get("projects").and_then(toml::Value::as_table);
        return projects.into_iter().flat_map(|p| p.keys()).map(PathBuf::from).collect();
    }
    text.lines().filter_map(|l| l.trim().strip_prefix("[projects.\"")?.strip_suffix("\"]").map(PathBuf::from)).collect()
}

/// What `session`'s agent calls it ([`AgentSession::name`]). Codex and OpenCode don't say which of
/// their processes runs which session, only the folder it began in: the newest begun there since the
/// process `started`, while it's its agent's only process there.
fn name(
    home: &Path,
    session: &AgentSession,
    all: &[AgentSession],
    started: Option<SystemTime>,
    args: &[String],
) -> Option<String> {
    let alone = || all.iter().filter(|s| s.agent == session.agent && s.dir == session.dir).count() == 1;
    match session.agent {
        AgentKind::Claude => claude_name(home, session.pid),
        AgentKind::Codex if alone() => codex_name(home, &session.dir, started, args),
        AgentKind::OpenCode if alone() => opencode_name(home, &session.dir, started?),
        _ => None,
    }
}

/// A string in `value`, trimmed, unless it's empty.
fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

/// Claude Code's name for its session `pid` (`~/.claude/sessions/<pid>.json` says which it is): the
/// last title in its transcript, or its own name for it until there's one (`asides-6b`).
fn claude_name(home: &Path, pid: i32) -> Option<String> {
    let file = home.join(".claude/sessions").join(format!("{pid}.json"));
    let value: Value = serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()?;
    let title = string(&value, "sessionId")
        .zip(string(&value, "cwd"))
        .and_then(|(id, cwd)| claude_transcript(home, &cwd, &id))
        .and_then(|transcript| transcript_title(&transcript));
    title.or_else(|| string(&value, "name"))
}

/// Claude Code's transcript of session `id`: in the folder of the project it began in (`cwd`, every
/// character but letters and digits a `-`), or else of any project.
fn claude_transcript(home: &Path, cwd: &str, id: &str) -> Option<PathBuf> {
    let (projects, file) = (home.join(".claude/projects"), format!("{id}.jsonl"));
    let project: String = cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let direct = projects.join(project).join(&file);
    if direct.is_file() {
        return Some(direct);
    }
    std::fs::read_dir(&projects).ok()?.flatten().map(|e| e.path().join(&file)).find(|p| p.is_file())
}

/// How far from its end a transcript is first read for a title: Claude Code writes it again as each
/// turn ends, and one turn can write megabytes. Read this much at a time, from the end.
const TITLE_REACH: u64 = 32 << 20;
const TITLE_CHUNK: u64 = 1 << 20;

/// A transcript's length when last read, and its title then.
type Seen = (u64, Option<String>);

/// Each transcript, as last read: only what's added is read again.
static TRANSCRIPTS: std::sync::LazyLock<Mutex<HashMap<PathBuf, Seen>>> = std::sync::LazyLock::new(Mutex::default);

/// The last title in a Claude Code transcript: what the user renamed the session to
/// (`custom-title`), or what Claude Code titled it from what was said (`ai-title`).
fn transcript_title(path: &Path) -> Option<String> {
    let len = std::fs::metadata(path).ok()?.len();
    let known = TRANSCRIPTS.lock().unwrap().get(path).cloned();
    let (from, before) = match known {
        Some((read, title)) if read == len => return title,
        Some((read, title)) if read < len => (read, title),
        // New, or shorter (written again).
        _ => (len.saturating_sub(TITLE_REACH), None),
    };
    let title = last_title(path, from, len).or(before);
    TRANSCRIPTS.lock().unwrap().insert(path.to_path_buf(), (len, title.clone()));
    title
}

/// The last title written between `from` and `to` in a transcript, read backward.
fn last_title(path: &Path, from: u64, to: u64) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let (mut end, mut cut) = (to, Vec::new());
    while end > from {
        let start = end.saturating_sub(TITLE_CHUNK).max(from);
        let mut chunk = vec![0; (end - start) as usize];
        file.seek(SeekFrom::Start(start)).ok()?;
        file.read_exact(&mut chunk).ok()?;
        chunk.append(&mut cut);
        // A line cut at the chunk's start is read with the chunk before it.
        let whole = match start == from {
            true => 0,
            false => chunk.iter().position(|&b| b == b'\n').map_or(chunk.len(), |i| i + 1),
        };
        if let Some(title) = title_in(&String::from_utf8_lossy(&chunk[whole..])) {
            return Some(title);
        }
        chunk.truncate(whole);
        (cut, end) = (chunk, start);
    }
    None
}

/// The last title in `text`, lines of a transcript.
fn title_in(text: &str) -> Option<String> {
    const KINDS: [(&str, &str); 2] = [(r#""type":"custom-title""#, "customTitle"), (r#""type":"ai-title""#, "aiTitle")];
    let mut end = text.len();
    loop {
        let (at, key) = KINDS.iter().filter_map(|(kind, key)| Some((text[..end].rfind(kind)?, *key))).max()?;
        let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
        let stop = text[at..].find('\n').map_or(text.len(), |i| at + i);
        if let Some(title) = serde_json::from_str(&text[start..stop]).ok().and_then(|v: Value| string(&v, key)) {
            return Some(title);
        }
        end = start;
    }
}

/// The name Codex gave the thread a session of its runs (`~/.codex/session_index.jsonl`): the one
/// it resumed (`codex resume <id>`), or the newest begun in its folder from the terminal since it
/// started.
fn codex_name(home: &Path, dir: &Path, started: Option<SystemTime>, args: &[String]) -> Option<String> {
    let uuid = |a: &&String| a.len() == 36 && a.chars().all(|c| c == '-' || c.is_ascii_hexdigit());
    let resumed = args.iter().skip_while(|a| *a != "resume").nth(1).filter(uuid).cloned();
    let id = match resumed {
        Some(id) => id,
        None => codex_thread(&home.join(".codex/sessions"), dir, started?)?,
    };
    let index = std::fs::read_to_string(home.join(".codex/session_index.jsonl")).ok()?;
    index.lines().rev().filter(|line| line.contains(&id)).find_map(|line| {
        let value: Value = serde_json::from_str(line).ok()?;
        (string(&value, "id")? == id).then(|| string(&value, "thread_name")).flatten()
    })
}

/// The newest thread begun in `dir` from Codex's terminal since `since`: each thread's rollout, in
/// `<year>/<month>/<day>/` under `sessions`, says where it began and from what.
fn codex_thread(sessions: &Path, dir: &Path, since: SystemTime) -> Option<String> {
    let since = since.checked_sub(Duration::from_secs(2))?;
    let listed = |dir: &Path| {
        let mut entries: Vec<PathBuf> =
            std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
        entries.sort_by(|a, b| b.cmp(a));
        entries
    };
    let changed = |path: &Path| std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let created = |path: &Path| std::fs::metadata(path).and_then(|m| m.created()).ok();
    // Newest first: a day's folder changes as a thread begins in it, so the days before didn't.
    let days = listed(sessions).into_iter().flat_map(|year| listed(&year)).flat_map(|month| listed(&month));
    let mut rollouts: Vec<(SystemTime, PathBuf)> = days
        .take_while(|day| changed(day).is_some_and(|at| at >= since))
        .flat_map(|day| listed(&day))
        .filter(|file| file.extension().is_some_and(|e| e == "jsonl"))
        .filter_map(|file| Some((created(&file).filter(|at| *at >= since)?, file)))
        .collect();
    rollouts.sort_by_key(|(created, _)| std::cmp::Reverse(*created));
    rollouts.into_iter().find_map(|(_, file)| {
        let mut first = String::new();
        std::io::BufReader::new(std::fs::File::open(file).ok()?).read_line(&mut first).ok()?;
        let value: Value = serde_json::from_str(&first).ok()?;
        let meta = value.get("payload")?;
        let terminal = matches!(meta.get("originator")?.as_str()?, "codex-tui" | "codex_cli_rs");
        (terminal && Path::new(meta.get("cwd")?.as_str()?) == dir).then(|| string(meta, "id")).flatten()
    })
}

/// The title OpenCode gave the session a process of its runs (its database): the newest begun in
/// its folder since the process started, once it has one.
fn opencode_name(home: &Path, dir: &Path, started: SystemTime) -> Option<String> {
    let db = home.join(".local/share/opencode/opencode.db");
    let since = started.checked_sub(Duration::from_secs(2))?.duration_since(UNIX_EPOCH).ok()?.as_millis();
    let dir = dir.to_str()?.replace('\'', "''");
    let sql = format!(
        "SELECT title FROM session WHERE directory = '{dir}' AND parent_id IS NULL AND time_created >= {since} \
         ORDER BY time_updated DESC LIMIT 1"
    );
    let rows = db.is_file().then(|| crate::wispr::sqlite(&db, &sql).ok()).flatten()?;
    let title = rows.first()?.get("title")?.as_str()?.trim();
    // Until it's titled, a session is named for when it began.
    (!title.is_empty() && !title.starts_with("New session - ")).then(|| title.to_string())
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
        let found = sessions_with(&procs, Host::inside);
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
    fn tells_what_takes_text_itself_on_a_mac() {
        let env = |vars: &[(&str, &str)]| -> HashMap<String, String> {
            vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
        };
        // tmux in Warp, which asides can't type into: its pane is reached all the same.
        let in_tmux =
            env(&[("TMUX", "/private/tmp/tmux-501/default,812,0"), ("TMUX_PANE", "%4"), ("TERM_PROGRAM", "tmux")]);
        let tmux = Host::of(&in_tmux);
        assert_eq!(tmux, Host::Tmux { socket: "/private/tmp/tmux-501/default".into(), pane: "%4".into() });
        assert!(tmux.reachable());
        // Its own tmux, the one whose server runs: a Mac's apps have no Homebrew on their `PATH`.
        let procs = [
            proc(812, 1, None, "/opt/homebrew/bin/tmux new -s work", &[]),
            proc(900, 1, None, "/Applications/kitty.app/Contents/MacOS/kitty", &[]),
            proc(901, 1, None, "tmux", &[]),
        ];
        assert_eq!(tmux.tool(0, &in_tmux, &procs), Some(PathBuf::from("/opt/homebrew/bin/tmux")));
        // A server ps names without its folder is left to the `PATH`.
        assert_eq!(tmux.tool(0, &env(&[("TMUX", "/tmp/t,901,0")]), &procs), None);
        let in_wezterm = env(&[
            ("TERM_PROGRAM", "WezTerm"),
            ("WEZTERM_PANE", "3"),
            ("WEZTERM_UNIX_SOCKET", "/Users/me/.local/share/wezterm/gui-sock-77"),
            ("WEZTERM_EXECUTABLE", "/Applications/WezTerm.app/Contents/MacOS/wezterm-gui"),
        ]);
        let wezterm = Host::of(&in_wezterm);
        let socket = Some("/Users/me/.local/share/wezterm/gui-sock-77".to_string());
        assert_eq!(wezterm, Host::WezTerm { pane: "3".into(), socket });
        let command = PathBuf::from("/Applications/WezTerm.app/Contents/MacOS/wezterm");
        assert_eq!(wezterm.tool(0, &in_wezterm, &procs), Some(command));
        // kitty takes text only with its remote control on.
        assert_eq!(
            Host::of(&env(&[("KITTY_WINDOW_ID", "2"), ("KITTY_PID", "900")])),
            Host::Other { name: "kitty".into() }
        );
        let in_kitty =
            env(&[("KITTY_WINDOW_ID", "2"), ("KITTY_PID", "900"), ("KITTY_LISTEN_ON", "unix:/tmp/kitty-900")]);
        let kitty = Host::of(&in_kitty);
        assert_eq!(kitty, Host::Kitty { listen: "unix:/tmp/kitty-900".into(), window: "2".into() });
        assert_eq!(
            kitty.tool(0, &in_kitty, &procs),
            Some(PathBuf::from("/Applications/kitty.app/Contents/MacOS/kitten"))
        );
        assert_eq!(Host::of(&env(&[("TERM_PROGRAM", "iTerm.app")])), Host::ITerm);
        assert!(Host::ITerm.reachable() && !Host::of(&env(&[("TERM_PROGRAM", "vscode")])).reachable());
        assert_eq!(Host::Terminal.tool(0, &in_tmux, &procs), None);
        assert_eq!(serde_json::to_string(&Host::ITerm).unwrap(), r#"{"app":"iterm"}"#);
    }

    #[test]
    fn the_nearest_multiplexer_or_editor_runs_the_agent() {
        let env = |vars: &[(&str, &str)]| -> HashMap<String, String> {
            vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
        };
        let zellij = env(&[("ZELLIJ", "0"), ("ZELLIJ_SESSION_NAME", "work"), ("ZELLIJ_PANE_ID", "2")]);
        assert_eq!(Host::of(&zellij), Host::Zellij { session: "work".into(), pane: "2".into() });
        let screen = env(&[("STY", "812.ttys004.mac"), ("WINDOW", "1"), ("TERM_PROGRAM", "Apple_Terminal")]);
        assert_eq!(Host::of(&screen), Host::Screen { session: "812.ttys004.mac".into(), window: "1".into() });
        // Neovim in Ghostty: its terminal, not Ghostty's, which shows Neovim.
        let nvim = env(&[("NVIM", "/tmp/nvim.me/0"), ("TERM_PROGRAM", "ghostty")]);
        assert_eq!(Host::of(&nvim), Host::Neovim { server: "/tmp/nvim.me/0".into() });
        // Claude Code in Neovim's terminal, Neovim in a pane of tmux: both say so; its processes tell.
        let both =
            env(&[("TMUX", "/private/tmp/tmux-501/default,812,0"), ("TMUX_PANE", "%1"), ("NVIM", "/tmp/nvim.me/0")]);
        let procs = [
            proc(812, 1, None, "/opt/homebrew/bin/tmux", &[]),
            proc(820, 812, Some("ttys004"), "-zsh", &[]),
            proc(830, 820, Some("ttys004"), "/opt/homebrew/bin/nvim", &[]),
            proc(840, 830, Some("ttys005"), "/bin/zsh", &[]),
            proc(850, 840, Some("ttys005"), "claude", &[]),
            // And tmux started in Neovim's terminal: its server isn't Neovim's child.
            proc(860, 812, Some("ttys006"), "codex", &[]),
        ];
        let mut procs = procs.to_vec();
        procs[4].env = both.clone();
        procs[5].env = both.clone();
        let found = sessions_with(&procs, Host::inside);
        assert_eq!(found[0].host, Host::Neovim { server: "/tmp/nvim.me/0".into() });
        assert_eq!(found[1].host, Host::Tmux { socket: "/private/tmp/tmux-501/default".into(), pane: "%1".into() });
        assert_eq!(found[0].host.tool(850, &both, &procs), Some(PathBuf::from("/opt/homebrew/bin/nvim")));
        assert_eq!(innermost(&procs, 860).map(|(name, _)| name), Some("tmux"));
        // GNU screen's server names itself `SCREEN`.
        let procs = [proc(700, 1, None, "SCREEN", &[]), proc(710, 700, Some("ttys008"), "droid", &[])];
        assert_eq!(innermost(&procs, 710).map(|(name, _)| name), Some("screen"));
        assert_eq!(
            serde_json::to_string(&Host::Neovim { server: "s".into() }).unwrap(),
            r#"{"app":"neovim","server":"s"}"#
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
            tool: None,
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
    fn names_a_claude_code_session_by_its_title() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        std::fs::create_dir_all(home.join(".claude/sessions")).unwrap();
        std::fs::write(
            home.join(".claude/sessions/4242.json"),
            r#"{"pid":4242,"sessionId":"abc-1","cwd":"/Users/me/Developer/my.shop","name":"my-shop-c2"}"#,
        )
        .unwrap();
        assert_eq!(claude_name(home, 4242).as_deref(), Some("my-shop-c2"), "no transcript yet: its own name");
        let project = home.join(".claude/projects/-Users-me-Developer-my-shop");
        std::fs::create_dir_all(&project).unwrap();
        let transcript = project.join("abc-1.jsonl");
        let line = |kind: &str, title: &str| match kind {
            "ai" => format!("{{\"type\":\"ai-title\",\"aiTitle\":\"{title}\",\"sessionId\":\"abc-1\"}}\n"),
            _ => format!("{{\"type\":\"custom-title\",\"customTitle\":\"{title}\",\"sessionId\":\"abc-1\"}}\n"),
        };
        // A message quoting a title line isn't one (its quotes are escaped).
        let quoted = r#"{"type":"user","message":{"content":"{\"type\":\"ai-title\",\"aiTitle\":\"Quoted\"}"}}"#;
        let mut text = line("ai", "Fix the login redirect");
        text.push_str(&format!("{quoted}\n"));
        std::fs::write(&transcript, &text).unwrap();
        assert_eq!(claude_name(home, 4242).as_deref(), Some("Fix the login redirect"));
        // A turn writing more than is read at a time, then a new title: only what was added is read.
        let long = format!("{{\"type\":\"assistant\",\"text\":\"{}\"}}\n", "x".repeat(TITLE_CHUNK as usize + 10));
        text.push_str(&long);
        std::fs::write(&transcript, &text).unwrap();
        assert_eq!(transcript_title(&transcript).as_deref(), Some("Fix the login redirect"), "kept while it works");
        text.push_str(&line("custom", "Login"));
        text.push_str(&long);
        std::fs::write(&transcript, &text).unwrap();
        assert_eq!(transcript_title(&transcript).as_deref(), Some("Login"), "renamed");
        // Read for the first time, across chunks.
        let copy = project.join("abc-2.jsonl");
        std::fs::write(&copy, &text).unwrap();
        assert_eq!(last_title(&copy, 0, text.len() as u64).as_deref(), Some("Login"));
    }

    #[test]
    fn names_a_codex_session_by_its_thread() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let started = SystemTime::now() - Duration::from_secs(60);
        let day = home.join(".codex/sessions/2026/10/20");
        std::fs::create_dir_all(&day).unwrap();
        let rollout = |id: &str, cwd: &str, originator: &str| {
            let meta = format!(
                r#"{{"type":"session_meta","payload":{{"id":"{id}","cwd":"{cwd}","originator":"{originator}"}}}}"#
            );
            std::fs::write(day.join(format!("rollout-2026-10-20T11-03-43-{id}.jsonl")), meta + "\n").unwrap();
        };
        let (shop, other) = ("01a15856-7354-78b2-8744-88bf9ff30ea8", "01a15856-0000-78b2-8744-88bf9ff30ea8");
        rollout(shop, "/r/shop", "codex-tui");
        rollout(other, "/r/shop", "codex_exec");
        std::fs::write(
            home.join(".codex/session_index.jsonl"),
            format!(
                "{{\"id\":\"{shop}\",\"thread_name\":\"Reply okay\"}}\n{{\"id\":\"{other}\",\"thread_name\":\"Exec\"}}\n"
            ),
        )
        .unwrap();
        let dir = Path::new("/r/shop");
        assert_eq!(codex_name(home, dir, Some(started), &[]).as_deref(), Some("Reply okay"), "not codex exec's");
        assert_eq!(codex_name(home, Path::new("/r/blog"), Some(started), &[]), None);
        // Begun before the session started: not its thread.
        let later = SystemTime::now() + Duration::from_secs(60);
        assert_eq!(codex_name(home, dir, Some(later), &[]), None);
        let resume = ["codex", "resume", other].map(String::from);
        assert_eq!(codex_name(home, dir, Some(later), &resume).as_deref(), Some("Exec"), "the thread resumed");
    }

    #[test]
    fn parses_how_long_processes_have_run() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let started = parse_started("  98681      05:12\n  12 01:00:00\n 7 2-03:04:05\n 8 junk\n", now);
        let ago = |pid: i32| now.duration_since(started[&pid]).unwrap().as_secs();
        assert_eq!((ago(98681), ago(12), ago(7)), (312, 3600, 2 * 86_400 + 3 * 3600 + 4 * 60 + 5));
        assert_eq!(started.len(), 3);
    }

    #[test]
    fn a_session_file_seen_first_is_a_session_started() {
        let mut starts = Starts::default();
        let claude = Path::new("/h/.claude/sessions/62089.json");
        assert!(starts.started(claude));
        // The session working: its status written again and again.
        assert!(!starts.started(claude));
        assert!(starts.started(Path::new("/h/.claude/sessions/70001.json")));
        assert!(starts.started(Path::new("/h/.codex/sessions/2026/10/19/rollout-2026-10-19T15-11-20-01a1.jsonl")));
        // Not a session's file.
        assert!(!starts.started(Path::new("/h/.claude/sessions/62089.792e0c80.key")));
        assert!(!starts.started(Path::new("/h/.codex/sessions/2026/10/19")));
    }

    #[test]
    fn suggests_folders_from_sessions_recents_and_the_agents_projects() {
        // Suggestions leave out `/tmp`, Linux's temporary folder.
        let tmp =
            if cfg!(target_os = "linux") { tempfile::tempdir_in("/var/tmp") } else { tempfile::tempdir() }.unwrap();
        // macOS's temporary folder is reached through a link (`/var` → `/private/var`).
        let home = if cfg!(windows) { tmp.path().to_path_buf() } else { tmp.path().canonicalize().unwrap() };
        let dir = |name: &str| {
            let d = home.join(name);
            std::fs::create_dir_all(d.join(".git")).unwrap();
            d
        };
        let (shop, blog, api, cli) = (dir("shop"), dir("blog"), dir("api"), dir("cli"));
        std::fs::create_dir_all(shop.join("web")).unwrap();
        let projects = serde_json::json!({"projects": {api.display().to_string(): {}, shop.display().to_string(): {}, "/gone": {}}});
        std::fs::write(home.join(".claude.json"), projects.to_string()).unwrap();
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::write(
            home.join(".codex/config.toml"),
            format!("[projects.{}]\ntrust_level = \"trusted\"\n", toml::Value::from(cli.display().to_string())),
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
            tool: None,
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
