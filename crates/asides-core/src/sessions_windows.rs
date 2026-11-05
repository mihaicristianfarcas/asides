//! The agent sessions on Windows, found as on a Mac (`sessions.rs`) but in Windows' own process
//! table: every process (a Toolhelp snapshot), then, for the agents', each one's command line,
//! folder and environment, read from its process parameters (the PEB, as Process Explorer reads
//! them). There's no `ps` or `lsof` to ask.
//!
//! A session needs a console to be typed into. Which one it has is told by its environment and its
//! ancestors, without attaching to it: Windows Terminal names each tab (`WT_SESSION`), and a
//! console window's host (`conhost.exe`) runs beside the shell that opened it; other terminals
//! (VS Code's) say who they are (`TERM_PROGRAM`) and keep a console of their own (ConPTY) under the
//! shell they start. asides types into any of them the same way (`deliver.rs`).
//!
//! Windows keeps no time of a console's last input (the Mac's terminal device's access time): a
//! session's last input is when its agent last wrote what was said (Claude Code's transcript), or
//! when it started; the app puts the sessions in the window in front first as asides opens
//! ([`in_front`]).

use std::collections::HashMap;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use windows::Wdk::System::Threading::{NtQueryInformationProcess, ProcessBasicInformation};
use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, HLOCAL, LocalFree};
use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_BASIC_INFORMATION, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_VM_READ, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Shell::CommandLineToArgvW;
use windows::core::{PCWSTR, PWSTR};

use super::{AgentSession, Host, Proc};
use crate::agents::{AgentKind, is_runtime};

/// A process in the snapshot.
#[derive(Debug, Clone)]
pub struct Entry {
    pub pid: i32,
    pub ppid: i32,
    /// Its executable's file name (`claude.exe`).
    pub exe: String,
}

/// Every process running now.
pub fn snapshot() -> Vec<Entry> {
    let mut found = Vec::new();
    // SAFETY: the snapshot's handle is closed below; each entry is a plain struct Windows fills.
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return found };
        let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
        while more {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            found.push(Entry {
                pid: entry.th32ProcessID as i32,
                ppid: entry.th32ParentProcessID as i32,
                exe: String::from_utf16_lossy(&entry.szExeFile[..len]),
            });
            more = Process32NextW(snapshot, &mut entry).is_ok();
        }
        let _ = CloseHandle(snapshot);
    }
    found
}

/// What a process says about itself.
#[derive(Debug, Default)]
struct Details {
    image: String,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    env: HashMap<String, String>,
    started: Option<SystemTime>,
}

/// An open process, closed when dropped.
struct Process(HANDLE);

impl Drop for Process {
    fn drop(&mut self) {
        // SAFETY: the handle is this struct's own, from `OpenProcess`.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

// The 64-bit layout of the process parameters (`RTL_USER_PROCESS_PARAMETERS`), the same for Arm64
// and x64 processes (an x64 process emulated on Arm64 too). 32-bit processes aren't read.
const PEB_PARAMETERS: usize = 0x20;
const PARAMETERS_CURRENT_DIRECTORY: usize = 0x38;
const PARAMETERS_COMMAND_LINE: usize = 0x70;
const PARAMETERS_ENVIRONMENT: usize = 0x80;
const PARAMETERS_ENVIRONMENT_SIZE: usize = 0x3f0;
/// An environment larger than this isn't read (they're a few kilobytes).
const MAX_ENVIRONMENT: usize = 1 << 20;

impl Process {
    fn open(pid: i32) -> Option<Process> {
        // SAFETY: a plain call; the handle is closed on drop.
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, false, pid as u32) }.ok().map(Process)
    }

    fn read(&self, at: usize, buffer: &mut [u8]) -> Option<()> {
        // SAFETY: Windows copies at most `buffer.len()` bytes into it, or fails.
        unsafe { ReadProcessMemory(self.0, at as *const c_void, buffer.as_mut_ptr().cast(), buffer.len(), None).ok() }
    }

    fn read_usize(&self, at: usize) -> Option<usize> {
        let mut bytes = [0; 8];
        self.read(at, &mut bytes)?;
        Some(u64::from_le_bytes(bytes) as usize)
    }

    /// A `UNICODE_STRING` (its length in bytes, then a pointer to its characters).
    fn read_unicode(&self, at: usize) -> Option<String> {
        let mut head = [0; 16];
        self.read(at, &mut head)?;
        let len = u16::from_le_bytes([head[0], head[1]]) as usize;
        let pointer = u64::from_le_bytes(head[8..16].try_into().ok()?) as usize;
        if len == 0 || pointer == 0 {
            return Some(String::new());
        }
        let mut text = vec![0; len];
        self.read(pointer, &mut text)?;
        Some(String::from_utf16_lossy(&utf16(&text)))
    }

    fn details(&self) -> Option<Details> {
        // SAFETY: plain data (a pointer among it), filled below.
        let mut info: PROCESS_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        let mut len = 0;
        // SAFETY: Windows fills `info`, whose size it's told.
        let status = unsafe {
            NtQueryInformationProcess(
                self.0,
                ProcessBasicInformation,
                (&mut info as *mut PROCESS_BASIC_INFORMATION).cast(),
                size_of::<PROCESS_BASIC_INFORMATION>() as u32,
                &mut len,
            )
        };
        if status.is_err() || info.PebBaseAddress.is_null() {
            return None;
        }
        let parameters = self.read_usize(info.PebBaseAddress as usize + PEB_PARAMETERS)?;
        let command_line = self.read_unicode(parameters + PARAMETERS_COMMAND_LINE)?;
        let cwd = self.read_unicode(parameters + PARAMETERS_CURRENT_DIRECTORY).map(|dir| {
            // `C:\Users\me\shop\`: without the separator Windows keeps at its end, but not a
            // drive's own (`C:\`).
            let trimmed = if dir.len() > 3 { dir.trim_end_matches('\\') } else { dir.as_str() };
            PathBuf::from(trimmed)
        });
        let environment = self.read_usize(parameters + PARAMETERS_ENVIRONMENT)?;
        let size = self.read_usize(parameters + PARAMETERS_ENVIRONMENT_SIZE).unwrap_or(0).min(MAX_ENVIRONMENT);
        let mut block = vec![0; size];
        let env = match environment != 0 && size > 0 && self.read(environment, &mut block).is_some() {
            true => parse_environment(&utf16(&block)),
            false => HashMap::new(),
        };
        Some(Details {
            image: self.image().unwrap_or_default(),
            args: split_command_line(&command_line),
            cwd,
            env,
            started: self.started(),
        })
    }

    fn image(&self) -> Option<String> {
        let mut path = vec![0u16; 1024];
        let mut len = path.len() as u32;
        // SAFETY: Windows writes at most `len` characters and says how many it wrote.
        unsafe { QueryFullProcessImageNameW(self.0, PROCESS_NAME_WIN32, PWSTR(path.as_mut_ptr()), &mut len) }.ok()?;
        Some(String::from_utf16_lossy(&path[..len as usize]))
    }

    fn started(&self) -> Option<SystemTime> {
        let (mut created, mut exited, mut kernel, mut user) = Default::default();
        // SAFETY: Windows fills the four times.
        unsafe { GetProcessTimes(self.0, &mut created, &mut exited, &mut kernel, &mut user) }.ok()?;
        system_time(created)
    }
}

/// Bytes as UTF-16 characters (little-endian).
fn utf16(bytes: &[u8]) -> Vec<u16> {
    bytes.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).collect()
}

/// A `FILETIME` (100 ns since 1601) as a time.
pub fn system_time(time: FILETIME) -> Option<SystemTime> {
    let ticks = ((time.dwHighDateTime as u64) << 32) | time.dwLowDateTime as u64;
    const UNIX_FROM_1601: u64 = 11_644_473_600 * 10_000_000;
    let since_unix = ticks.checked_sub(UNIX_FROM_1601)?;
    UNIX_EPOCH.checked_add(Duration::from_nanos(since_unix.checked_mul(100)?))
}

/// An environment block: `NAME=value` strings, each ended by a nul, the block by another. Names
/// starting with `=` are the shell's per-drive folders (`=C:=C:\Users`), not variables.
pub fn parse_environment(block: &[u16]) -> HashMap<String, String> {
    block
        .split(|&c| c == 0)
        .take_while(|entry| !entry.is_empty())
        .filter_map(|entry| {
            let entry = String::from_utf16_lossy(entry);
            let (name, value) = entry.split_once('=').filter(|(name, _)| !name.is_empty())?;
            Some((name.to_string(), value.to_string()))
        })
        .collect()
}

/// A command line split into its words, as the C runtime splits it for `main` (Windows' own rules,
/// `CommandLineToArgvW`).
pub fn split_command_line(line: &str) -> Vec<String> {
    if line.trim().is_empty() {
        return Vec::new();
    }
    let wide: Vec<u16> = line.encode_utf16().chain([0]).collect();
    let mut count = 0;
    // SAFETY: the line is nul-terminated; the array Windows returns is freed with `LocalFree`.
    unsafe {
        let words = CommandLineToArgvW(PCWSTR(wide.as_ptr()), &mut count);
        if words.is_null() {
            return Vec::new();
        }
        let found = (0..count as usize).map(|i| (*words.add(i)).to_string().unwrap_or_default()).collect();
        let _ = LocalFree(Some(HLOCAL(words.cast())));
        found
    }
}

/// Programs that host consoles: Windows' console window (`conhost.exe`) and Windows Terminal's
/// (`OpenConsole.exe`).
fn console_host(exe: &str) -> bool {
    exe.eq_ignore_ascii_case("conhost.exe") || exe.eq_ignore_ascii_case("openconsole.exe")
}

/// Whether a process's file name may be an agent's: an agent's own, a script runtime's (npm's
/// installs), or a version's (Claude Code's own installs keep each version as `versions\2.1.303`).
fn may_be_agent(exe: &str) -> bool {
    let stem = exe.strip_suffix(".exe").or_else(|| exe.strip_suffix(".EXE")).unwrap_or(exe);
    AgentKind::of_process(exe, &[]).is_some()
        || is_runtime(exe)
        || (!stem.is_empty() && stem.chars().all(|c| c.is_ascii_digit() || c == '.'))
}

/// Which console a process has, from its environment and its ancestors: Windows Terminal's tab
/// (`WT_SESSION`), or the shell a console host runs beside, or else, in a terminal that says who it
/// is (`TERM_PROGRAM`), the shell it started. `None` for a process without one.
pub fn console_of(pid: i32, env: &HashMap<String, String>, entries: &[Entry]) -> Option<String> {
    if let Some(tab) = env.get("WT_SESSION").filter(|t| !t.is_empty()) {
        return Some(format!("wt-{tab}"));
    }
    let by_pid: HashMap<i32, &Entry> = entries.iter().map(|e| (e.pid, e)).collect();
    let hosts: Vec<i32> = entries.iter().filter(|e| console_host(&e.exe)).map(|e| e.ppid).collect();
    let mut ancestors = std::iter::successors(by_pid.get(&pid).copied(), |e| by_pid.get(&e.ppid).copied())
        .take(16)
        // A parent's id can be taken again by a newer process: the chain stops at a loop.
        .scan(Vec::new(), |seen: &mut Vec<i32>, e| {
            (!seen.contains(&e.pid)).then(|| {
                seen.push(e.pid);
                e
            })
        });
    let terminal = env.get("TERM_PROGRAM").is_some_and(|t| !t.is_empty());
    for entry in ancestors.by_ref() {
        if hosts.contains(&entry.pid) {
            return Some(format!("console-{}", entry.pid));
        }
        // The shell a terminal started: the last ancestor before the terminal's own process.
        if terminal && by_pid.get(&entry.ppid).is_some_and(|p| !may_be_shell(&p.exe) && !may_be_agent(&p.exe)) {
            return Some(format!("term-{}", entry.pid));
        }
    }
    None
}

/// Programs that run in a console between a terminal and an agent: shells, and the launchers
/// they start agents through.
fn may_be_shell(exe: &str) -> bool {
    const SHELLS: &[&str] =
        &["cmd.exe", "powershell.exe", "pwsh.exe", "bash.exe", "sh.exe", "zsh.exe", "fish.exe", "nu.exe", "wsl.exe"];
    SHELLS.iter().any(|s| exe.eq_ignore_ascii_case(s))
}

/// The terminal app showing a console (Windows Terminal, VS Code…), named for the sessions' list.
pub fn host_of(env: &HashMap<String, String>) -> Host {
    if env.get("WT_SESSION").is_some_and(|t| !t.is_empty()) {
        return Host::Console { name: "Windows Terminal".into() };
    }
    let name = match env.get("TERM_PROGRAM").map(String::as_str) {
        Some("vscode") => "VS Code",
        Some("WezTerm") => "WezTerm",
        Some("Tabby") => "Tabby",
        Some(other) if !other.is_empty() => other,
        _ => "Console",
    };
    Host::Console { name: name.into() }
}

/// The agent sessions running now, with their folders, last input and names.
pub fn discover() -> Vec<AgentSession> {
    let entries = snapshot();
    let mut started = HashMap::new();
    let procs: Vec<Proc> = entries
        .iter()
        .map(|e| {
            let mut proc = Proc { pid: e.pid, ppid: e.ppid, executable: e.exe.clone(), ..Proc::default() };
            if !may_be_agent(&e.exe) {
                return proc;
            }
            let Some(details) = Process::open(e.pid).and_then(|p| p.details()) else { return proc };
            proc.tty = console_of(e.pid, &details.env, &entries);
            if !details.image.is_empty() {
                proc.executable = details.image;
            }
            proc.args = details.args;
            proc.env = details.env;
            if let Some(cwd) = details.cwd {
                proc.env.insert(CWD.into(), cwd.to_string_lossy().into_owned());
            }
            if let Some(at) = details.started {
                started.insert(e.pid, at);
            }
            proc
        })
        .collect();
    let mut sessions = super::sessions_with(&procs, host_of);
    let dirs: HashMap<i32, PathBuf> =
        procs.iter().filter_map(|p| Some((p.pid, PathBuf::from(p.env.get(CWD)?)))).collect();
    for s in &mut sessions {
        s.dir = dirs.get(&s.pid).cloned().unwrap_or_default();
    }
    sessions.retain(|s| !s.dir.as_os_str().is_empty());
    if let Some(home) = std::env::home_dir() {
        let args: HashMap<i32, &[String]> = procs.iter().map(|p| (p.pid, &p.args[..])).collect();
        let names: Vec<Option<String>> = sessions
            .iter()
            .map(|s| {
                let args = args.get(&s.pid).copied().unwrap_or(&[]);
                super::name(&home, s, &sessions, started.get(&s.pid).copied(), args)
            })
            .collect();
        for (s, name) in sessions.iter_mut().zip(names) {
            s.name = name;
        }
    }
    STARTED.lock().unwrap().extend(started);
    read_input_times(&mut sessions);
    sessions
}

/// Where the folder a process works in is kept among its variables while sessions are found.
const CWD: &str = "\0cwd";

/// When each process was started, as last found.
static STARTED: std::sync::LazyLock<std::sync::Mutex<HashMap<i32, SystemTime>>> =
    std::sync::LazyLock::new(Default::default);

/// When each session last got input, as near as Windows tells: when its agent last wrote what was
/// said (Claude Code's transcript and session file), or else when it started.
pub fn read_input_times(sessions: &mut [AgentSession]) {
    let home = std::env::home_dir().unwrap_or_default();
    let started = STARTED.lock().unwrap();
    for s in sessions {
        let written = match s.agent {
            AgentKind::Claude => claude_written(&home, s.pid),
            _ => None,
        };
        s.last_input = written.or_else(|| started.get(&s.pid).copied());
    }
}

/// When Claude Code last wrote session `pid`'s files: its session file, and its transcript.
fn claude_written(home: &Path, pid: i32) -> Option<SystemTime> {
    let file = home.join(".claude").join("sessions").join(format!("{pid}.json"));
    let modified = |path: &Path| std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let value: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).ok()?).ok()?;
    let transcript = value
        .get("sessionId")
        .and_then(|v| v.as_str())
        .zip(value.get("cwd").and_then(|v| v.as_str()))
        .and_then(|(id, cwd)| super::claude_transcript(home, cwd, id));
    [modified(&file), transcript.as_deref().and_then(modified)].into_iter().flatten().max()
}

/// The sessions in the terminal whose window is in front (its process, `front`), first: of them,
/// the one whose agent wrote last gets the latest input. Windows Terminal's tabs share its window.
pub fn in_front(sessions: &mut [AgentSession], front: i32) {
    let entries = snapshot();
    let by_pid: HashMap<i32, &Entry> = entries.iter().map(|e| (e.pid, e)).collect();
    // A console window is its host's (`conhost.exe`), which runs beside the shell.
    let front_shell = by_pid.get(&front).filter(|e| console_host(&e.exe)).map(|e| e.ppid);
    let shows = |pid: i32| {
        std::iter::successors(by_pid.get(&pid).copied(), |e| by_pid.get(&e.ppid).copied())
            .take(16)
            .any(|e| e.pid == front || Some(e.pid) == front_shell)
    };
    let latest =
        sessions.iter().enumerate().filter(|(_, s)| shows(s.pid)).max_by_key(|(_, s)| s.last_input).map(|(i, _)| i);
    if let Some(i) = latest {
        sessions[i].last_input = Some(SystemTime::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_an_environment_block() {
        let block: Vec<u16> = "=C:=C:\\Users\0WT_SESSION=4f1e\0Path=C:\\a;C:\\b\0\0junk".encode_utf16().collect();
        let env = parse_environment(&block);
        assert_eq!(env.get("WT_SESSION").map(String::as_str), Some("4f1e"));
        assert_eq!(env.get("Path").map(String::as_str), Some("C:\\a;C:\\b"));
        assert_eq!(env.len(), 2, "{env:?}");
    }

    #[test]
    fn splits_command_lines_as_windows_does() {
        let words = split_command_line(r#""C:\Program Files\nodejs\node.exe" C:\x\cli.js -- "fix the \"login\"""#);
        assert_eq!(words, [r"C:\Program Files\nodejs\node.exe", r"C:\x\cli.js", "--", r#"fix the "login""#]);
        assert!(split_command_line("  ").is_empty());
    }

    #[test]
    fn tells_which_console_a_process_has() {
        let entry = |pid, ppid, exe: &str| Entry { pid, ppid, exe: exe.into() };
        let entries = [
            entry(1, 0, "explorer.exe"),
            // A console window: cmd.exe with its host beside it, and an agent in it.
            entry(10, 1, "cmd.exe"),
            entry(11, 10, "conhost.exe"),
            entry(12, 10, "claude.exe"),
            entry(13, 12, "node.exe"),
            // VS Code's terminal: its shell, an agent.
            entry(20, 1, "Code.exe"),
            entry(21, 20, "pwsh.exe"),
            entry(22, 21, "codex.exe"),
            // Started without a console.
            entry(30, 1, "claude.exe"),
        ];
        let none = HashMap::new();
        let wt = HashMap::from([("WT_SESSION".to_string(), "4f1e".to_string())]);
        let vscode = HashMap::from([("TERM_PROGRAM".to_string(), "vscode".to_string())]);
        assert_eq!(console_of(12, &wt, &entries).as_deref(), Some("wt-4f1e"));
        assert_eq!(console_of(12, &none, &entries).as_deref(), Some("console-10"));
        assert_eq!(console_of(13, &none, &entries), console_of(12, &none, &entries), "one console, one session");
        assert_eq!(console_of(22, &vscode, &entries).as_deref(), Some("term-21"));
        assert_eq!(console_of(30, &none, &entries), None);
        assert_eq!(host_of(&wt), Host::Console { name: "Windows Terminal".into() });
        assert_eq!(host_of(&vscode), Host::Console { name: "VS Code".into() });
        assert_eq!(host_of(&none), Host::Console { name: "Console".into() });
    }

    #[test]
    fn takes_versions_and_runtimes_for_agents() {
        assert!(may_be_agent("claude.exe") && may_be_agent("2.1.303.exe") && may_be_agent("node.exe"));
        assert!(!may_be_agent("pwsh.exe") && !may_be_agent("explorer.exe"));
    }

    #[test]
    fn reads_windows_times() {
        let at = FILETIME { dwLowDateTime: 0xd53e_8000, dwHighDateTime: 0x019d_b1de };
        assert_eq!(system_time(at), Some(UNIX_EPOCH));
    }

    /// Lists the sessions running on this PC: `cargo test -p asides-core discovers_on_windows --
    /// --ignored --nocapture`.
    #[test]
    #[ignore = "looks at the processes running on this PC"]
    fn discovers_on_windows() {
        for s in discover() {
            println!("{} {:?} {} {} {:?} {:?}", s.pid, s.agent, s.host.name(), s.dir.display(), s.tty, s.name);
        }
    }
}
