//! The log, `~/Library/Logs/asides/asides.log` (Console lists it under Log Reports): a line for
//! what went wrong and what matters around it, from the app, the overlay and the asides window.
//! Each process's standard error goes there too (`capture_stderr`), so what anything prints,
//! whisper.cpp included, is kept; panics are written with where they happened (`init`). A
//! problem's line has `!` after the process (`problem`): `asides log` lists them, with the crash
//! reports macOS kept (`crash`).

use std::fmt::Display;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Past this the log moves to `asides.log.1`, the one before going.
const MAX_LEN: u64 = 4 << 20;

/// What a problem's line has after the process.
pub const PROBLEM: &str = " ! ";

/// This process in the log: `overlay[812]`.
static NAME: OnceLock<String> = OnceLock::new();

pub fn path() -> PathBuf {
    crate::paths::logs_dir().join("asides.log")
}

/// This process is `name` in the log (`app`, `overlay`, `window`); its panics are written there,
/// with the thread and the backtrace, before they end it.
pub fn init(name: &str) {
    let _ = NAME.set(format!("{name}[{}]", std::process::id()));
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let at = info.location().map(|l| format!(" at {l}")).unwrap_or_default();
        let said = info.payload_as_str().unwrap_or("(no message)");
        let backtrace = std::backtrace::Backtrace::force_capture();
        // What it said on the first line: `asides log` lists only that.
        problem(format_args!("panic on thread {}{at}: {said}\n{backtrace}", thread.name().unwrap_or("unnamed")));
        previous(info);
    }));
}

/// A line in the log.
pub fn write(message: impl Display) {
    add(false, &message.to_string());
}

/// A line in the log for something that went wrong.
pub fn problem(message: impl Display) {
    add(true, &message.to_string());
}

fn add(problem: bool, message: &str) {
    let name = NAME.get().map_or("asides", String::as_str);
    let time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string();
    let _ = append(&path(), entry(&time, name, problem, message).as_bytes(), MAX_LEN);
}

/// `time process message`, `!` between them for a problem; a message's further lines indented.
fn entry(time: &str, name: &str, problem: bool, message: &str) -> String {
    let mut lines = message.lines();
    let mark = if problem { PROBLEM } else { " " };
    let mut text = format!("{time} {name}{mark}{}\n", lines.next().unwrap_or_default());
    for line in lines {
        text += &format!("    {line}\n");
    }
    text
}

fn append(path: &Path, bytes: &[u8], max_len: u64) -> std::io::Result<()> {
    if std::fs::metadata(path).is_ok_and(|m| m.len() > max_len) {
        let _ = std::fs::rename(path, path.with_extension("log.1"));
    }
    let open = || OpenOptions::new().create(true).append(true).open(path);
    let mut file = match open() {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
            open()?
        }
        file => file?,
    };
    // In one write: lines from processes writing at once don't mix.
    file.write_all(bytes)
}

/// Sends this process's standard error into the log, a line at a time, from a thread of its own;
/// the processes it starts afterwards write there too. Not when it's a terminal (in development,
/// someone reads it there).
pub fn capture_stderr() -> std::io::Result<()> {
    use std::io::IsTerminal;
    use std::os::fd::AsRawFd;
    if std::io::stderr().is_terminal() {
        return Ok(());
    }
    let (reader, writer) = std::io::pipe()?;
    // SAFETY: both descriptors are this process's; standard error is replaced in one step.
    if unsafe { libc::dup2(writer.as_raw_fd(), libc::STDERR_FILENO) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    drop(writer);
    std::thread::Builder::new().name("asides-log".into()).spawn(move || relay(reader, |line| write(line)))?;
    Ok(())
}

/// Each line read, to `line`, until the writers are gone.
fn relay(reader: impl Read, line: impl Fn(&str)) {
    let mut reader = BufReader::new(reader);
    let mut text = Vec::new();
    loop {
        text.clear();
        match reader.read_until(b'\n', &mut text) {
            Ok(0) => return,
            Ok(_) => line(String::from_utf8_lossy(&text).trim_end()),
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => {
                // Kept open: writing to standard error must never fail (`eprintln!` panics then).
                std::mem::forget(reader);
                return;
            }
        }
    }
}

/// The problems in the log (and the one before it) from `days` back, oldest first.
pub fn recent_problems(days: i64) -> Vec<String> {
    let since = (chrono::Local::now() - chrono::Duration::days(days)).format("%Y-%m-%d %H:%M").to_string();
    let path = path();
    [path.with_extension("log.1"), path]
        .iter()
        .flat_map(|p| {
            let text = std::fs::read_to_string(p).unwrap_or_default();
            problems(&text, &since).into_iter().map(str::to_string).collect::<Vec<_>>()
        })
        .collect()
}

/// The problems in the log (`text`) at or after `since` (`2026-10-23 16:02`, compared as text).
pub fn problems<'a>(text: &'a str, since: &str) -> Vec<&'a str> {
    let mut problems = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let first = line.split(PROBLEM).next().unwrap_or_default();
        // The time and the process, then the mark: not a `!` in a message.
        let marked = line.contains(PROBLEM) && first.split(' ').count() == 3;
        if !marked || line.get(..since.len()).is_none_or(|time| time < since) {
            continue;
        }
        problems.push(line);
        // Not its further lines (a panic's backtrace).
        while lines.peek().is_some_and(|l| l.starts_with("    ")) {
            lines.next();
        }
    }
    problems
}

/// What a crash report says, in short.
#[derive(Clone, Debug, PartialEq)]
pub struct Crash {
    pub process: String,
    pub version: String,
    /// `2026-10-19 16:26:59.00 +0300`
    pub time: String,
    /// `EXC_CRASH SIGABRT: abort() called`
    pub reason: String,
    /// The crashed thread's first function of asides' own (past the standard library's).
    pub crashed_in: Option<String>,
    /// The main thread's, when another thread crashed (a hang the watchdog ended: what it was
    /// stuck in).
    pub main_in: Option<String>,
}

/// Reads a crash report (`.ips`: a line of JSON about it, then the report's JSON).
pub fn crash(text: &str) -> Option<Crash> {
    use serde_json::Value;
    let (head, body) = text.split_once('\n')?;
    let head: Value = serde_json::from_str(head).ok()?;
    let body: Value = serde_json::from_str(body).ok()?;
    let text = |v: &Value| v.as_str().unwrap_or_default().to_string();
    let exception = &body["exception"];
    let mut reason =
        [&exception["type"], &exception["signal"]].into_iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ");
    let said = body["asi"].as_object().into_iter().flat_map(|o| o.values()).filter_map(Value::as_array).flatten();
    if let Some(said) = said.filter_map(Value::as_str).next() {
        reason = format!("{reason}: {said}");
    }
    let images: Vec<String> =
        body["usedImages"].as_array().into_iter().flatten().map(|image| text(&image["name"])).collect();
    let own = |thread: &Value| {
        thread["frames"].as_array().into_iter().flatten().find_map(|frame| {
            let image = images.get(frame["imageIndex"].as_u64()? as usize)?;
            let symbol = format!("{:#}", rustc_demangle::demangle(frame["symbol"].as_str()?));
            let library = ["std::", "core::", "alloc::", "__rust", "rust_panic", "<std::", "<core::", "<alloc::"];
            (image.to_lowercase().starts_with("asides") && !library.iter().any(|l| symbol.starts_with(l)))
                .then_some(symbol)
        })
    };
    let threads = body["threads"].as_array();
    let crashed = threads.into_iter().flatten().position(|t| t["triggered"] == true);
    let thread = |i: usize| threads.and_then(|t| t.get(i));
    Some(Crash {
        process: text(&head["app_name"]),
        version: text(&head["app_version"]),
        time: text(&head["timestamp"]),
        reason,
        crashed_in: crashed.and_then(thread).and_then(own),
        main_in: crashed.filter(|&i| i != 0).and(thread(0)).and_then(own),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_mark_problems_and_indent_further_lines() {
        let time = "2026-10-23 16:02:11.123";
        assert_eq!(entry(time, "app[1]", false, "opened"), "2026-10-23 16:02:11.123 app[1] opened\n");
        assert_eq!(
            entry(time, "overlay[2]", true, "panic on thread main: at x.rs\nboom"),
            "2026-10-23 16:02:11.123 overlay[2] ! panic on thread main: at x.rs\n    boom\n"
        );
    }

    #[test]
    fn the_log_moves_aside_when_it_grows_too_long() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs/asides.log");
        append(&path, b"012345678\n", 10).unwrap();
        append(&path, b"second\n", 10).unwrap();
        // Too long now: the next line starts a new one.
        append(&path, b"third\n", 10).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "third\n");
        assert_eq!(std::fs::read_to_string(path.with_extension("log.1")).unwrap(), "012345678\nsecond\n");
    }

    #[test]
    fn relays_each_line() {
        let lines = std::cell::RefCell::new(Vec::new());
        relay(&b"ggml_metal: error\nnot UTF-8: \xff\r\nlast, unfinished"[..], |l| {
            lines.borrow_mut().push(l.to_string())
        });
        assert_eq!(lines.into_inner(), ["ggml_metal: error", "not UTF-8: \u{fffd}", "last, unfinished"]);
    }

    #[test]
    fn lists_problems_since_a_time() {
        let log = "\
2026-10-22 09:00:00.000 app[1] ! could not write state.json
2026-10-23 16:00:00.000 overlay[2] Space held 300 ms in pin 1's card: talking
2026-10-23 16:00:01.000 overlay[2] said \"hi ! there\"
2026-10-23 16:02:11.000 overlay[2] ! panic on thread main: boom
    0: std::backtrace
2026-10-23 16:03:00.000 app[1] ! the overlay ended unexpectedly (signal 6)
";
        assert_eq!(
            problems(log, "2026-10-23"),
            [
                "2026-10-23 16:02:11.000 overlay[2] ! panic on thread main: boom",
                "2026-10-23 16:03:00.000 app[1] ! the overlay ended unexpectedly (signal 6)"
            ]
        );
        assert_eq!(problems(log, "").len(), 3);
    }

    #[test]
    fn reads_a_crash_report() {
        let head = r#"{"app_name":"asides-overlay","timestamp":"2026-10-23 16:05:00.00 +0300","app_version":"1.0.8"}"#;
        let body = serde_json::json!({
            "exception": {"type": "EXC_CRASH", "signal": "SIGABRT"},
            "asi": {"libsystem_c.dylib": ["abort() called"]},
            "usedImages": [{"name": "libsystem_kernel.dylib"}, {"name": "asides-overlay"}],
            "threads": [
                {"frames": [
                    {"imageIndex": 0, "symbol": "mach_msg2_trap"},
                    {"imageIndex": 1, "symbol": "_RNvMs_Csevsns4j2Y0r_10asides_appNtB4_3App6handle"},
                ]},
                {"triggered": true, "frames": [
                    {"imageIndex": 0, "symbol": "__pthread_kill"},
                    {"imageIndex": 1, "symbol": "_RNvNtCs1SSuMAdGM58_3std7process5abort"},
                    {"imageIndex": 1, "symbol": "_RNvNtCsh2FAxYNBLCa_11asides_core7history6create"},
                ]},
            ],
        });
        let crash = crash(&format!("{head}\n{body}")).unwrap();
        assert_eq!(
            crash,
            Crash {
                process: "asides-overlay".into(),
                version: "1.0.8".into(),
                time: "2026-10-23 16:05:00.00 +0300".into(),
                reason: "EXC_CRASH SIGABRT: abort() called".into(),
                crashed_in: Some("asides_core::history::create".into()),
                main_in: Some("<asides_app::App>::handle".into()),
            }
        );
        assert_eq!(super::crash("not a report"), None);
    }
}
