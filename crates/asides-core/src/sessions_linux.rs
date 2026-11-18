//! The agent sessions on Linux, found as on a Mac (`sessions.rs`) but in `/proc`, with nothing to
//! run: every process's terminal (`stat`), and for the agents' their arguments (`cmdline`),
//! environment (`environ`) and folder (`cwd`). Only the user's own processes can be read, which is
//! all that matters.
//!
//! Which terminal a session is in is told by its environment. Any terminal can be typed into through
//! its pty (`asides-keys`, `Host::Pty`), where the system allows it; those that take text themselves
//! (tmux, Konsole, WezTerm, kitty with remote control on) are asked to, which needs nothing allowed.
//! A terminal device's access time says when it last got input, as on a Mac (`w`'s idle time): the
//! kernel moves it every 8 seconds at most.

use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::procfs::{parse_environ, parse_stat, split_nul};
use super::{AgentSession, Host, Proc};
use crate::agents::{AgentKind, is_runtime};

/// The pty's number from a session's device (`pts/3`).
pub fn pts_index(tty: &str) -> Option<u32> {
    tty.strip_prefix("pts/")?.parse().ok()
}

/// Tells the terminal a session is in from what its programs were given.
pub fn host_of(env: &HashMap<String, String>) -> Host {
    host_with(env, None, helper_may_type())
}

/// [`host_of`], knowing which multiplexer or editor runs the agent (`innermost`).
pub fn host_inside(env: &HashMap<String, String>, innermost: Option<&str>) -> Host {
    host_with(env, innermost, helper_may_type())
}

/// [`host_inside`], knowing whether terminals can be typed into through their ptys.
pub fn host_with(env: &HashMap<String, String>, innermost: Option<&str>, may_type: bool) -> Host {
    let get = |name: &str| env.get(name).filter(|v| !v.is_empty());
    // A multiplexer first: its programs got the variables of the terminal that started it too.
    if let Some(host) = Host::taking_text(env, innermost) {
        return host;
    }
    if let (Some(service), Some(session)) = (get("KONSOLE_DBUS_SERVICE"), get("KONSOLE_DBUS_SESSION")) {
        return Host::Konsole { service: service.clone(), session: session.clone() };
    }
    Host::Pty { name: terminal_name(env), may_type }
}

/// The terminal's name, as far as its programs are told.
fn terminal_name(env: &HashMap<String, String>) -> String {
    let has = |name: &str| env.get(name).is_some_and(|v| !v.is_empty());
    let term = |name: &str| env.get("TERM").is_some_and(|t| t.starts_with(name));
    let names = [
        (has("PTYXIS_VERSION") || has("PTYXIS_PROFILE"), "Ptyxis"),
        (has("GNOME_TERMINAL_SCREEN") || has("GNOME_TERMINAL_SERVICE"), "GNOME Terminal"),
        (has("KITTY_WINDOW_ID"), "kitty"),
        (has("ALACRITTY_WINDOW_ID") || has("ALACRITTY_LOG"), "Alacritty"),
        (has("GHOSTTY_RESOURCES_DIR"), "Ghostty"),
        (has("TILIX_ID"), "Tilix"),
        (has("TERMINATOR_UUID"), "Terminator"),
        (has("XTERM_VERSION"), "xterm"),
        (term("foot"), "foot"),
    ];
    if let Some((_, name)) = names.iter().find(|(there, _)| *there) {
        return (*name).to_string();
    }
    match env.get("TERM_PROGRAM").map(String::as_str) {
        Some("vscode") => "VS Code".into(),
        Some("ghostty") => "Ghostty".into(),
        Some("WarpTerminal") => "Warp".into(),
        Some("tmux") => "tmux".into(),
        Some(other) if !other.is_empty() => other.to_string(),
        _ => "a terminal".into(),
    }
}

/// `asides-keys`, beside this program; `ASIDES_KEYS` overrides it, for development.
pub fn helper() -> PathBuf {
    if let Some(path) = std::env::var_os("ASIDES_KEYS") {
        return PathBuf::from(path);
    }
    let exe = std::env::current_exe().unwrap_or_default();
    exe.canonicalize().unwrap_or(exe).with_file_name(asides_keys::HELPER)
}

/// Whether `asides-keys` can type into the user's terminals here.
pub fn helper_may_type() -> bool {
    asides_keys::may_type(&helper())
}

/// The agent sessions running now, with their folders, last input and names.
pub fn discover() -> Vec<AgentSession> {
    // SAFETY: a plain query.
    let uid = unsafe { libc::getuid() };
    let may_type = helper_may_type();
    let mut starts: HashMap<i32, u64> = HashMap::new();
    let mut procs: Vec<Proc> = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else { return Vec::new() };
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<i32>().ok()) else { continue };
        if !entry.metadata().is_ok_and(|m| m.uid() == uid) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else { continue };
        let Some(stat) = parse_stat(&text) else { continue };
        let executable = std::fs::read_link(format!("/proc/{pid}/exe"))
            .map(|p| p.to_string_lossy().trim_end_matches(" (deleted)").to_string())
            .unwrap_or_default();
        let mut proc = Proc { pid, ppid: stat.ppid, tty: stat.tty, executable, ..Proc::default() };
        // Arguments and environments only for agents' processes (and scripts', which may be agents).
        if proc.tty.is_some() {
            let args = std::fs::read(format!("/proc/{pid}/cmdline")).map(|b| split_nul(&b)).unwrap_or_default();
            let words: Vec<&str> = args.iter().map(String::as_str).collect();
            if is_runtime(&proc.executable) || AgentKind::of_process(&proc.executable, &words).is_some() {
                proc.env = std::fs::read(format!("/proc/{pid}/environ")).map(|b| parse_environ(&b)).unwrap_or_default();
                proc.args = args;
                starts.insert(pid, stat.start_ticks);
            }
        }
        procs.push(proc);
    }
    let mut sessions = super::sessions_with(&procs, |env, innermost| host_with(env, innermost, may_type));
    for s in &mut sessions {
        s.tool = procs.iter().find(|p| p.pid == s.pid).and_then(|p| s.host.tool(s.pid, &p.env, &procs));
    }
    for s in &mut sessions {
        s.dir = std::fs::read_link(format!("/proc/{}/cwd", s.pid)).unwrap_or_default();
    }
    sessions.retain(|s| !s.dir.as_os_str().is_empty());
    let boot = boot_time();
    if let Some(home) = std::env::home_dir() {
        let args: HashMap<i32, &[String]> = procs.iter().map(|p| (p.pid, &p.args[..])).collect();
        let names: Vec<Option<String>> = sessions
            .iter()
            .map(|s| {
                let started = starts.get(&s.pid).and_then(|&ticks| started_at(boot?, ticks));
                super::name(&home, s, &sessions, started, args.get(&s.pid).copied().unwrap_or(&[]))
            })
            .collect();
        for (s, name) in sessions.iter_mut().zip(names) {
            s.name = name;
        }
    }
    super::read_input_times(&mut sessions);
    sessions
}

/// When the computer started (`/proc/stat`'s `btime`).
fn boot_time() -> Option<SystemTime> {
    let text = std::fs::read_to_string("/proc/stat").ok()?;
    let seconds: u64 = text.lines().find_map(|l| l.strip_prefix("btime "))?.trim().parse().ok()?;
    Some(UNIX_EPOCH + Duration::from_secs(seconds))
}

/// When a process started, from its clock ticks since the computer did.
fn started_at(boot: SystemTime, ticks: u64) -> Option<SystemTime> {
    // SAFETY: a plain query.
    let per_second = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    (per_second > 0).then(|| boot + Duration::from_secs_f64(ticks as f64 / per_second as f64))
}

/// A program on `PATH` (or, given as a path, that file).
pub fn find_program(path: &std::ffi::OsStr, name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        return Path::new(name).is_file().then(|| PathBuf::from(name));
    }
    std::env::split_paths(path)
        .map(|dir| dir.join(name))
        .find(|candidate| std::fs::metadata(candidate).is_ok_and(|m| m.is_file() && m.mode() & 0o111 != 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(vars: &[(&str, &str)]) -> HashMap<String, String> {
        vars.iter().map(|(n, v)| (n.to_string(), v.to_string())).collect()
    }

    #[test]
    fn numbers_ptys() {
        assert_eq!(pts_index("pts/257"), Some(257));
        assert_eq!(pts_index("ttys003"), None);
    }

    #[test]
    fn tells_terminals_and_multiplexers_apart() {
        let pty = |name: &str, may_type| Host::Pty { name: name.into(), may_type };
        assert_eq!(
            host_with(
                &env(&[("GNOME_TERMINAL_SCREEN", "/org/gnome/Terminal/screen/1"), ("VTE_VERSION", "7600")]),
                None,
                true
            ),
            pty("GNOME Terminal", true)
        );
        assert_eq!(
            host_with(&env(&[("PTYXIS_VERSION", "47"), ("VTE_VERSION", "7800")]), None, false),
            pty("Ptyxis", false)
        );
        assert_eq!(host_with(&env(&[("TERM_PROGRAM", "vscode")]), None, true), pty("VS Code", true));
        assert_eq!(host_with(&env(&[("TERM", "foot")]), None, true), pty("foot", true));
        assert_eq!(host_with(&env(&[]), None, true), pty("a terminal", true));
        // In tmux, whatever terminal started it.
        assert_eq!(
            host_with(
                &env(&[
                    ("TMUX", "/tmp/tmux-1000/default,1234,0"),
                    ("TMUX_PANE", "%3"),
                    ("GNOME_TERMINAL_SCREEN", "/x"),
                    ("TERM_PROGRAM", "tmux"),
                ]),
                None,
                false
            ),
            Host::Tmux { socket: "/tmp/tmux-1000/default".into(), pane: "%3".into() }
        );
        assert_eq!(
            host_with(&env(&[("KONSOLE_DBUS_SERVICE", ":1.42"), ("KONSOLE_DBUS_SESSION", "/Sessions/2")]), None, false),
            Host::Konsole { service: ":1.42".into(), session: "/Sessions/2".into() }
        );
        assert_eq!(
            host_with(&env(&[("WEZTERM_PANE", "7")]), None, false),
            Host::WezTerm { pane: "7".into(), socket: None }
        );
        // kitty answers only with remote control on: its pty otherwise.
        assert_eq!(host_with(&env(&[("KITTY_WINDOW_ID", "1")]), None, true), pty("kitty", true));
        assert_eq!(
            host_with(&env(&[("KITTY_WINDOW_ID", "1"), ("KITTY_LISTEN_ON", "unix:/tmp/k")]), None, false),
            Host::Kitty { listen: "unix:/tmp/k".into(), window: "1".into() }
        );
    }
}
