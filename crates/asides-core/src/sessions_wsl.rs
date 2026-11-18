//! Agents in WSL's Linux distributions, found from Windows. Windows' process table shows only the
//! `wsl.exe` a terminal runs: what runs inside is listed by a short script ([`script`]), given on
//! the input of `sh` in each distribution already running (`wsl.exe -d <name> --exec sh`), which
//! never starts one. It reads `/proc` as Linux's sessions are found (`sessions_linux.rs`).
//!
//! An agent there is typed into through the console of its `wsl.exe`, which hands what's typed to
//! Linux's terminal (`deliver.rs`). Which `wsl.exe` is told by Windows Terminal's tab (`WT_SESSION`,
//! which it hands into WSL too), or else by when each started: the `wsl.exe` that started as the
//! agent's terminal did. In tmux, Zellij, screen or Neovim there, their own command types in, run
//! through `wsl.exe`. Screenshots are given by their paths in WSL (`/mnt/c/…`).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::procfs::{parse_environ, parse_stat, split_nul};
use super::{AgentSession, Host, Proc, innermost};
use crate::agents::AgentKind;

/// The script listing a distribution's processes (the user's): for each, its `stat` and program,
/// and for an agent's (or a runtime's, which may run one) with a terminal, also its terminal, folder,
/// terminal's last input, arguments and environment. Then when the system started, its clock's ticks
/// per second, and where Windows' `C:` is. Fields are separated by `0x1f`, processes by `0x1e`.
/// About 90 ms in a distribution that runs.
pub fn script() -> String {
    let mut names: Vec<&str> = AgentKind::executables();
    names.extend(["node", "nodejs", "bun", "python*", "[0-9]*"]);
    format!(
        r#"cd /proc || exit 0
for p in [0-9]*; do
  [ -O "$p" ] || continue
  read -r s < "$p/stat" 2>/dev/null || continue
  e=$(readlink "$p/exe" 2>/dev/null)
  printf '\036%s\037%s' "$s" "$e"
  set -- ${{s##*) }}
  [ "$5" = 0 ] && continue
  case "${{e##*/}}" in {}) ;; *) continue ;; esac
  t=$(readlink "$p/fd/0" 2>/dev/null)
  printf '\037%s\037%s\037%s\037' "$t" "$(readlink "$p/cwd" 2>/dev/null)" "$(stat -c %X "$t" 2>/dev/null)"
  cat "$p/cmdline" 2>/dev/null
  printf '\037'
  cat "$p/environ" 2>/dev/null
done
printf '\036\037%s\037%s\037%s' "$(sed -n 's/^btime //p' /proc/stat)" "$(getconf CLK_TCK)" "$(wslpath -u 'C:\' 2>/dev/null)"
"#,
        names.join("|")
    )
}

/// What the script found in a distribution.
#[derive(Debug, Default)]
pub struct Listing {
    pub procs: Vec<Proc>,
    /// When each process started, in clock ticks since the system did.
    starts: HashMap<i32, u64>,
    dirs: HashMap<i32, String>,
    /// When each agent's terminal last got input (its access time, in seconds).
    typed: HashMap<i32, u64>,
    /// When the system started (seconds since 1970), and its clock's ticks per second.
    boot: Option<u64>,
    ticks: u64,
    /// Where Windows' drives are (`/mnt/`).
    pub mount: String,
}

impl Listing {
    fn started(&self, pid: i32) -> Option<SystemTime> {
        let ticks = *self.starts.get(&pid)?;
        Some(UNIX_EPOCH + Duration::from_secs(self.boot?) + Duration::from_millis(ticks * 1000 / self.ticks.max(1)))
    }
}

/// Reads what [`script`] printed.
pub fn parse(output: &[u8]) -> Listing {
    let mut listing = Listing { ticks: 100, mount: "/mnt/".into(), ..Listing::default() };
    for record in output.split(|&b| b == 0x1e).skip(1) {
        let fields: Vec<&[u8]> = record.splitn(7, |&b| b == 0x1f).collect();
        let text = |i: usize| fields.get(i).map(|f| String::from_utf8_lossy(f).trim().to_string()).unwrap_or_default();
        if fields.first().is_some_and(|f| f.is_empty()) {
            listing.boot = text(1).parse().ok();
            listing.ticks = text(2).parse().ok().filter(|&t| t > 0).unwrap_or(100);
            if let Some(mount) = text(3).strip_suffix("c/").filter(|m| m.starts_with('/')) {
                listing.mount = mount.to_string();
            }
            continue;
        }
        let stat = text(0);
        let (Some(pid), Some(parsed)) =
            (stat.split_whitespace().next().and_then(|p| p.parse().ok()), parse_stat(&stat))
        else {
            continue;
        };
        let executable = text(1).trim_end_matches(" (deleted)").to_string();
        let mut proc = Proc { pid, ppid: parsed.ppid, tty: parsed.tty, executable, ..Proc::default() };
        listing.starts.insert(pid, parsed.start_ticks);
        if fields.len() == 7 {
            listing.dirs.insert(pid, text(3));
            if let Ok(at) = text(4).parse() {
                listing.typed.insert(pid, at);
            }
            proc.args = split_nul(fields[5]);
            proc.env = parse_environ(fields[6]);
        }
        listing.procs.push(proc);
    }
    listing
}

/// The distributions `wsl.exe --list --quiet` names (UTF-16, as Windows' consoles get it, or UTF-8
/// with `WSL_UTF8`).
pub fn parse_list(output: &[u8]) -> Vec<String> {
    let utf16 =
        output.len() >= 2 && output.iter().skip(1).step_by(2).filter(|&&b| b == 0).count() * 2 >= output.len() / 2;
    let text = if utf16 {
        let units: Vec<u16> = output.as_chunks::<2>().0.iter().map(|&pair| u16::from_le_bytes(pair)).collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(output).into_owned()
    };
    text.lines()
        .map(|l| l.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}'))
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

/// A Windows process that may hold a WSL terminal's console: a `wsl.exe`, or the program a tab of
/// Windows Terminal runs.
#[derive(Debug, Clone, PartialEq)]
pub struct Console {
    pub pid: i32,
    pub started: Option<SystemTime>,
    /// Its tab of Windows Terminal (`WT_SESSION`).
    pub tab: Option<String>,
    /// The distribution it runs, for a `wsl.exe` (others are told only by their tab).
    pub distro: Option<String>,
}

/// The distribution `wsl.exe` runs with these arguments: the one `-d` names, or the default.
pub fn distro_of(args: &[String], default: &str) -> String {
    args.windows(2)
        .find(|w| matches!(w[0].as_str(), "-d" | "--distribution"))
        .map_or_else(|| default.to_string(), |w| w[1].clone())
}

/// How far apart a terminal's `wsl.exe` and its first process in Linux may have started: the
/// distribution may have had to start first.
const STARTED_APART: Duration = Duration::from_secs(30);

/// The agent sessions in distribution `distro`, as `listing` found them, each with the console it's
/// typed into through, from `consoles` (each taken once, across distributions: `taken`).
pub fn sessions(distro: &str, listing: &Listing, consoles: &[Console], taken: &mut HashSet<i32>) -> Vec<AgentSession> {
    let procs = &listing.procs;
    let mut found = super::sessions_with(procs, |env, innermost| {
        let inner = Host::taking_text(env, innermost).filter(|h| {
            matches!(h, Host::Tmux { .. } | Host::Zellij { .. } | Host::Screen { .. } | Host::Neovim { .. })
        });
        Host::Wsl { distro: distro.into(), console: None, mount: listing.mount.clone(), inner: inner.map(Box::new) }
    });
    let by_pid: HashMap<i32, &Proc> = procs.iter().map(|p| (p.pid, p)).collect();
    for s in &mut found {
        s.dir = listing.dirs.get(&s.pid).map(PathBuf::from).unwrap_or_default();
        s.last_input = listing.typed.get(&s.pid).map(|&at| UNIX_EPOCH + Duration::from_secs(at));
        if let Host::Wsl { inner: Some(_), .. } = s.host {
            // Their own command, where it is in the distribution (its `PATH` there may not have it).
            s.tool =
                innermost(procs, s.pid).map(|(_, p)| &p.executable).filter(|e| e.starts_with('/')).map(PathBuf::from);
        }
    }
    found.retain(|s| !s.dir.as_os_str().is_empty());
    // Windows Terminal's tabs say which console is which.
    for s in &mut found {
        let tab = by_pid.get(&s.pid).and_then(|p| p.env.get("WT_SESSION")).filter(|t| !t.is_empty());
        let Host::Wsl { console: console @ None, inner: None, .. } = &mut s.host else { continue };
        if let Some(c) = consoles.iter().find(|c| !taken.contains(&c.pid) && c.tab.is_some() && c.tab.as_ref() == tab) {
            *console = Some(c.pid);
            taken.insert(c.pid);
        }
    }
    // The others by when they started: the terminal's first process in Linux (below WSL's own,
    // which aren't the user's), and the `wsl.exe` nearest in time.
    let first =
        |pid: i32| std::iter::successors(by_pid.get(&pid), |p| by_pid.get(&p.ppid)).take(64).last().map(|p| p.pid);
    let mut pairs: Vec<(Duration, usize, i32)> = Vec::new();
    for (i, s) in found.iter().enumerate() {
        let Host::Wsl { console: None, inner: None, .. } = s.host else { continue };
        let Some(at) = first(s.pid).and_then(|pid| listing.started(pid)) else { continue };
        for c in consoles.iter().filter(|c| c.distro.as_deref() == Some(distro) && !taken.contains(&c.pid)) {
            let Some(started) = c.started else { continue };
            let apart = at.duration_since(started).or_else(|_| started.duration_since(at)).unwrap_or_default();
            if apart <= STARTED_APART {
                pairs.push((apart, i, c.pid));
            }
        }
    }
    pairs.sort();
    for (_, i, pid) in pairs {
        if let Host::Wsl { console: console @ None, .. } = &mut found[i].host
            && taken.insert(pid)
        {
            *console = Some(pid);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the script prints of an agent's process: its terminal, folder, last input, arguments and
    /// environment.
    type Agent<'a> = (&'a str, &'a str, &'a str, &'a [&'a str], &'a [&'a str]);

    /// One process as the script prints it.
    fn record(stat: &str, exe: &str, extra: Option<Agent>) -> Vec<u8> {
        let mut out = format!("\x1e{stat}\x1f{exe}").into_bytes();
        if let Some((tty, cwd, at, args, env)) = extra {
            out.extend(format!("\x1f{tty}\x1f{cwd}\x1f{at}\x1f").bytes());
            args.iter().for_each(|a| out.extend(format!("{a}\0").bytes()));
            out.push(0x1f);
            env.iter().for_each(|e| out.extend(format!("{e}\0").bytes()));
        }
        out
    }

    fn stat(pid: i32, comm: &str, ppid: i32, tty_nr: u32, start: u64) -> String {
        format!("{pid} ({comm}) S {ppid} {pid} {pid} {tty_nr} {pid} 0 0 0 0 0 0 0 0 0 20 0 1 0 {start} 1 2")
    }

    const BOOT: u64 = 1_794_993_962;

    /// A distribution with a shell started by Windows Terminal's `wsl.exe` (Claude Code in it), one
    /// started by another `wsl.exe` (Codex), and tmux running Gemini CLI.
    fn listing() -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(record(&stat(441, "zsh", 367, 34816, 1000), "/usr/bin/zsh", None));
        out.extend(record(
            &stat(500, "claude", 441, 34816, 2000),
            "/home/m/.local/share/claude/versions/2.1.327",
            Some((
                "/dev/pts/0",
                "/home/m/shop",
                "1794995000",
                &["claude"],
                &["WT_SESSION=tab-1", "TERM=xterm-256color"],
            )),
        ));
        out.extend(record(&stat(600, "bash", 599, 34817, 50_000), "/usr/bin/bash", None));
        out.extend(record(
            &stat(610, "node", 600, 34817, 50_100),
            "/usr/bin/node",
            Some(("/dev/pts/1", "/home/m/api", "1794995100", &["node", "/usr/bin/codex"], &["TERM=xterm"])),
        ));
        out.extend(record(&stat(700, "tmux: server", 1, 0, 60_000), "/usr/bin/tmux", None));
        out.extend(record(&stat(701, "bash", 700, 34818, 60_010), "/usr/bin/bash", None));
        out.extend(record(
            &stat(702, "node", 701, 34818, 60_020),
            "/usr/bin/node",
            Some((
                "/dev/pts/2",
                "/home/m/web",
                "1794995200",
                &["node", "/usr/local/bin/gemini"],
                &["TMUX=/tmp/tmux-1000/default,700,0", "TMUX_PANE=%3", "WT_SESSION=tab-9"],
            )),
        ));
        out.extend(format!("\x1e\x1f{BOOT}\n\x1f100\x1f/mnt/c/").bytes());
        out
    }

    fn at(seconds: u64) -> Option<SystemTime> {
        Some(UNIX_EPOCH + Duration::from_secs(seconds))
    }

    #[test]
    fn reads_what_the_script_printed() {
        let listing = parse(&listing());
        assert_eq!(listing.procs.len(), 7);
        let claude = listing.procs.iter().find(|p| p.pid == 500).unwrap();
        assert_eq!((claude.ppid, claude.tty.as_deref()), (441, Some("pts/0")));
        assert_eq!(claude.args, ["claude"]);
        assert_eq!(claude.env.get("WT_SESSION").map(String::as_str), Some("tab-1"));
        assert_eq!(listing.started(441), at(BOOT + 10));
        assert_eq!(listing.mount, "/mnt/");
        assert!(listing.procs.iter().find(|p| p.pid == 441).unwrap().env.is_empty());
    }

    #[test]
    fn finds_each_agents_console() {
        let listing = parse(&listing());
        let consoles = [
            // Another distribution's, started as Codex's shell did.
            Console { pid: 8, started: at(BOOT + 500), tab: None, distro: Some("Debian".into()) },
            Console { pid: 12, started: at(BOOT + 501), tab: None, distro: Some("Ubuntu".into()) },
            Console { pid: 16, started: at(BOOT + 3), tab: Some("tab-1".into()), distro: Some("Ubuntu".into()) },
        ];
        let mut taken = HashSet::new();
        let found = sessions("Ubuntu", &listing, &consoles, &mut taken);
        let host = |pid: i32| found.iter().find(|s| s.pid == pid).map(|s| s.host.clone()).unwrap();
        let wsl = |console: Option<i32>, inner: Option<Host>| Host::Wsl {
            distro: "Ubuntu".into(),
            console,
            mount: "/mnt/".into(),
            inner: inner.map(Box::new),
        };
        assert_eq!(host(500), wsl(Some(16), None), "by its tab");
        assert_eq!(host(610), wsl(Some(12), None), "by when it started");
        let tmux = Host::Tmux { socket: "/tmp/tmux-1000/default".into(), pane: "%3".into() };
        assert_eq!(host(702), wsl(None, Some(tmux)), "tmux's own command types in");
        let gemini = found.iter().find(|s| s.pid == 702).unwrap();
        assert_eq!(gemini.tool, Some(PathBuf::from("/usr/bin/tmux")));
        assert_eq!(gemini.id(), "Ubuntu:702");
        assert_eq!(found.iter().find(|s| s.pid == 500).unwrap().dir, PathBuf::from("/home/m/shop"));
        assert_eq!(found.iter().find(|s| s.pid == 500).unwrap().last_input, at(1_794_995_000));
        assert_eq!(taken, HashSet::from([12, 16]));
        // A console taken isn't another's.
        let again = sessions("Ubuntu", &listing, &consoles, &mut taken);
        assert!(again.iter().all(|s| matches!(s.host, Host::Wsl { console: None, .. })), "{again:?}");
        assert!(!again.iter().find(|s| s.pid == 500).unwrap().host.reachable());
    }

    #[test]
    fn names_the_distributions_running() {
        let utf16: Vec<u8> =
            "\u{feff}Ubuntu\r\ndocker-desktop\r\n\r\n".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(parse_list(&utf16), ["Ubuntu", "docker-desktop"]);
        assert_eq!(parse_list(b"Debian\n"), ["Debian"]);
        let args = |words: &[&str]| words.iter().map(|w| w.to_string()).collect::<Vec<_>>();
        assert_eq!(distro_of(&args(&["wsl.exe", "-d", "Debian", "--cd", "~"]), "Ubuntu"), "Debian");
        assert_eq!(distro_of(&args(&["C:\\Windows\\system32\\wsl.exe"]), "Ubuntu"), "Ubuntu");
    }

    #[test]
    fn lists_agents_programs_in_the_script() {
        let script = script();
        assert!(script.contains("claude|") && script.contains("|cursor-agent|") && script.contains("|python*|"));
        assert!(!script.contains("{{"), "{script}");
    }
}
