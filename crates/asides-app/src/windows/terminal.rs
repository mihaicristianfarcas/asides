//! Typing asides into agent sessions on Windows (a `deliver::plan`'s steps), and starting new
//! sessions in Windows Terminal or a console window of their own.
//!
//! A console is typed into by the `asides` command (`asides type-into <pid>`), not the app: it
//! attaches to the session's console to write keys into it, and a process can be attached to one
//! console at a time, which ends it when that console closes. The command runs without a console of
//! its own, once per step.
//!
//! A new session runs `asides launch <file>` in the terminal: the file says which agent, where, and
//! the aside as its first message, which is handed to the agent as an argument by Windows' own rules
//! (no shell between, whatever the aside's quotes and new lines).

use std::fs;
use std::io::Write;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use asides_core::agents::AgentKind;
use asides_core::deliver::{self, Step};

use crate::clipboard;

/// `DETACHED_PROCESS`: no console (it attaches to the session's); `CREATE_NEW_CONSOLE`: a console
/// window of its own; `CREATE_NO_WINDOW`: a console nobody sees.
const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The app sessions open in: the configured one, or Windows Terminal when it's there.
pub fn app(configured: &str) -> String {
    let installed = deliver::installed_terminals();
    match installed.iter().find(|t| t.eq_ignore_ascii_case(configured.trim())) {
        Some(name) => name.to_string(),
        None => installed.first().map_or("Console".into(), |t| t.to_string()),
    }
}

/// Diri's command: macOS only.
pub fn dirijor() -> PathBuf {
    PathBuf::new()
}

/// The `asides` command, beside the app (`ASIDES_CLI` overrides it, for development).
pub fn cli() -> PathBuf {
    if let Some(path) = std::env::var_os("ASIDES_CLI") {
        return PathBuf::from(path);
    }
    std::env::current_exe().unwrap_or_default().with_file_name("asides.exe")
}

/// Runs a delivery plan's steps, in order. The clipboard, if a step used it, is put back even when a
/// step fails.
pub fn run_steps(steps: &[Step]) -> Result<()> {
    let mut kept: Option<clipboard::Snapshot> = None;
    let result = steps.iter().try_for_each(|step| run_step(step, &mut kept));
    if let Some(snapshot) = kept {
        clipboard::restore(&snapshot);
    }
    result
}

fn run_step(step: &Step, kept: &mut Option<clipboard::Snapshot>) -> Result<()> {
    match step {
        Step::Run { program, args, stdin } => run(Command::new(program).args(args), stdin.as_deref(), CREATE_NO_WINDOW)
            .with_context(|| format!("could not run {}", program.display()))?,
        Step::Console { pid, text } => {
            run(Command::new(cli()).arg("type-into").arg(pid.to_string()), Some(text), DETACHED_PROCESS)?
        }
        Step::Script(_) => bail!("AppleScript is macOS's"),
        Step::Pty { .. } | Step::Konsole { .. } => bail!("that's how Linux's terminals are typed into"),
        Step::ClipboardImage(path) => {
            if kept.is_none() {
                *kept = Some(clipboard::snapshot());
            }
            clipboard::put_png(&fs::read(path).with_context(|| format!("could not read {}", path.display()))?);
        }
        Step::RestoreClipboard => {
            if let Some(snapshot) = kept.take() {
                clipboard::restore(&snapshot);
            }
        }
        Step::Wait(duration) => std::thread::sleep(*duration),
    }
    Ok(())
}

/// Runs `command` with `stdin`, and says what it said went wrong.
fn run(command: &mut Command, stdin: Option<&str>, flags: u32) -> Result<()> {
    let mut child =
        command.creation_flags(flags).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn()?;
    if let Some(text) = stdin {
        child.stdin.take().context("no stdin")?.write_all(text.as_bytes())?;
    }
    drop(child.stdin.take());
    let out = child.wait_with_output()?;
    if !out.status.success() {
        let text = String::from_utf8_lossy(&out.stderr);
        let line = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("it failed");
        bail!("{}", line.trim().trim_start_matches("asides: "));
    }
    Ok(())
}

/// Starts `agent` in `dir` in `terminal` ("Windows Terminal" or "Console") with `message` as its
/// first message; `readable`: folders it may read without asking (the aside's screenshots).
/// Returns the launch's id (`ASIDES_LAUNCH` in the agent's environment).
pub fn start_session(
    terminal: &str,
    agent: AgentKind,
    dir: &Path,
    message: &str,
    extra: &[String],
    readable: &[PathBuf],
) -> Result<String> {
    let launches = asides_core::paths::launch_dir();
    fs::create_dir_all(&launches)?;
    let launch = format!("{}-{}", agent.program(), now_ms());
    let file = launches.join(format!("{launch}.json"));
    let words = deliver::launch_words(agent, message, extra, readable);
    let spec = serde_json::json!({ "words": words, "dir": dir, "launch": launch });
    fs::write(&file, spec.to_string())?;
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let title = format!("{} ({name})", agent.name());
    let started = match app(terminal).as_str() {
        "Windows Terminal" => Command::new("wt.exe")
            .args(["-w", "new", "new-tab", "--title"])
            .arg(wt_arg(&title))
            .arg("-d")
            .arg(wt_arg(&dir.to_string_lossy()))
            .arg(wt_arg(&cli().to_string_lossy()))
            .arg("launch")
            .arg(wt_arg(&file.to_string_lossy()))
            .spawn()
            .map(drop),
        _ => Command::new(cli())
            .arg("launch")
            .arg(&file)
            .current_dir(dir)
            .creation_flags(CREATE_NEW_CONSOLE)
            .spawn()
            .map(drop),
    };
    if let Err(err) = started {
        let _ = fs::remove_file(&file);
        bail!("could not open {}: {err}", app(terminal));
    }
    Ok(launch)
}

/// An argument for `wt.exe`, which splits its command line at each `;` that isn't escaped.
fn wt_arg(text: &str) -> String {
    text.replace(';', r"\;")
}

/// The agents installed for the user: programs on their PATH, as a new terminal has it (the
/// registry's, which may have changed since asides started).
pub fn installed_agents() -> Vec<AgentKind> {
    let path = asides_core::launch::user_path();
    AgentKind::ALL
        .into_iter()
        .filter(|kind| asides_core::launch::find_program(&path, kind.program()).is_some())
        .collect()
}

fn now_ms() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_windows_terminal_arguments() {
        assert_eq!(wt_arg(r"C:\a;b"), r"C:\a\;b");
    }
}
