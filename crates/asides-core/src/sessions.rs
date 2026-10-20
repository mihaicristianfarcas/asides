//! The agent sessions on this Mac, found in the process table: each coding agent (`agents.rs`)
//! running in a terminal, the folder it works in, its terminal device, and the app hosting that
//! terminal, which is how an aside reaches it (`deliver.rs`). Nothing is installed in the agents and
//! nothing has to listen.
//!
//! The app hosting a session is told by the environment the agent was started with: Terminal and
//! Ghostty set `TERM_PROGRAM`; Diri sets `DIRIJOR_SESSION_ID` (and no `TERM_PROGRAM`, so a terminal's
//! own wins over one inherited from Diri). The session the user last typed into is the one whose
//! terminal device was last read from: its access time, which is how `w` tells idle time.
//!
//! A session is named as its agent titles it from what was said in it (or as the user renamed it):
//! Claude Code in its transcript, Codex in its thread index, OpenCode in its database.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    /// What the agent calls the session, when it says: the title it gave it from what was said
    /// ("Fix the login redirect"), or Claude Code's name for it until then (`asides-6b`).
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
    for s in sessions {
        s.last_input = std::fs::metadata(format!("/dev/{}", s.tty)).and_then(|m| m.accessed()).ok();
    }
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
