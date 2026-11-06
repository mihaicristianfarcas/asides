//! Typing asides into agent sessions on Linux (a `deliver::plan`'s steps: tmux's and other terminals'
//! own commands, Konsole's D-Bus, and `asides-keys` for a pty), and starting new sessions in a new
//! window of the user's terminal: a script that runs the agent with the aside as its first message.
//!
//! An app started by the desktop often has a shorter PATH than a shell's (nvm, `~/.local/bin` set in
//! `.bashrc`), so the agents are found through the user's login shell, which has the PATH their
//! terminal has.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use asides_core::agents::AgentKind;
use asides_core::deliver::{self, Step};

use crate::clipboard;

/// How long the login shell may take to say which agents are installed.
const SHELL_TIMEOUT: Duration = Duration::from_secs(15);

/// The app sessions open in: the configured one if it's installed, or the first that is (the
/// desktop's own comes first).
pub fn app(configured: &str) -> String {
    let installed = deliver::installed_terminals();
    match installed.iter().find(|t| t.eq_ignore_ascii_case(configured.trim())) {
        Some(name) => name.to_string(),
        None => installed.first().map_or("xterm".into(), |t| t.to_string()),
    }
}

/// Diri's command: macOS only.
pub fn dirijor() -> PathBuf {
    PathBuf::new()
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
        Step::Run { program, args, stdin } => {
            run(Command::new(program).args(args), stdin.as_deref())
                .with_context(|| format!("could not run {}", program.display()))?;
        }
        Step::Pty { tty, text } => {
            let index = tty.strip_prefix("pts/").with_context(|| format!("{tty} isn't a pty"))?;
            let helper = asides_core::sessions::linux::helper();
            run(Command::new(&helper).arg(index), Some(text))
                .with_context(|| format!("could not type with {}", helper.display()))?;
        }
        Step::Konsole { service, session, text } => {
            let connection = zbus::blocking::Connection::session().context("no session bus")?;
            connection
                .call_method(
                    Some(service.as_str()),
                    session.as_str(),
                    Some("org.kde.konsole.Session"),
                    "sendText",
                    text,
                )
                .context("Konsole didn't take it")?;
        }
        Step::Script(_) => bail!("AppleScript is macOS's"),
        Step::Console { .. } => bail!("consoles are Windows'"),
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
fn run(command: &mut Command, stdin: Option<&str>) -> Result<()> {
    let mut child = command.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn()?;
    if let Some(text) = stdin {
        child.stdin.take().context("no stdin")?.write_all(text.as_bytes())?;
    }
    drop(child.stdin.take());
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!("{}", last_line(&out.stderr, "it failed"));
    }
    Ok(())
}

fn last_line(bytes: &[u8], otherwise: &str) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or(otherwise).trim().to_string()
}

/// Starts `agent` in `dir` in a new window of `terminal`, with `message` as its first message;
/// `readable`: folders it may read without asking (the aside's screenshots). Returns the launch's id
/// (`ASIDES_LAUNCH` in the agent's environment).
pub fn start_session(
    terminal: &str,
    agent: AgentKind,
    dir: &Path,
    message: &str,
    extra: &[String],
    readable: &[PathBuf],
) -> Result<String> {
    let scripts = asides_core::paths::launch_dir();
    fs::create_dir_all(&scripts)?;
    let launch = format!("{}-{}", agent.program(), now_ms());
    let script = scripts.join(format!("{launch}.sh"));
    fs::write(&script, deliver::launch_script(agent, dir, message, extra, &launch, readable))?;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755))?;
    let name = app(terminal);
    let Some(chosen) = deliver::linux_terminal(&name) else {
        let _ = fs::remove_file(&script);
        bail!("asides can't open {name}");
    };
    let program = ["/bin/sh".to_string(), script.to_string_lossy().into_owned()];
    let started = Command::new(chosen.program)
        .args(deliver::linux_terminal_args(&chosen, dir, &program))
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Err(err) = started {
        let _ = fs::remove_file(&script);
        bail!("could not open {name}: {err}");
    }
    Ok(launch)
}

/// What `child` wrote, once it ends (it's waited on, on a thread of its own), or `None` when it
/// takes longer than `timeout`: it's stopped then.
fn finished(child: Child, timeout: Duration) -> Result<Option<Output>> {
    let pid = child.id() as libc::pid_t;
    let (done, ended) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(child.wait_with_output());
    });
    match ended.recv_timeout(timeout) {
        Ok(out) => Ok(Some(out?)),
        Err(_) => {
            // SAFETY: a plain system call, on the child this function was given.
            unsafe { libc::kill(pid, libc::SIGKILL) };
            Ok(None)
        }
    }
}

/// The agents installed for the user: programs on their login shell's PATH.
pub fn installed_agents() -> Vec<AgentKind> {
    let child = match login_shell("printf '%s' \"$PATH\"").stdout(Stdio::piped()).stderr(Stdio::null()).spawn() {
        Ok(child) => child,
        Err(_) => return Vec::new(),
    };
    let Ok(Some(out)) = finished(child, SHELL_TIMEOUT) else { return Vec::new() };
    let path = String::from_utf8_lossy(&out.stdout);
    // Interactive shells may print a banner first: the PATH is the last line.
    let path = path.lines().last().unwrap_or_default();
    AgentKind::ALL.into_iter().filter(|kind| on_path(path, kind.program())).collect()
}

/// Whether `program` is an executable file in one of `path`'s directories.
fn on_path(path: &str, program: &str) -> bool {
    std::env::split_paths(path)
        .any(|dir| fs::metadata(dir.join(program)).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
}

/// `$SHELL -ilc command`: interactive too, since many people set their PATH in `.bashrc`.
fn login_shell(command: &str) -> Command {
    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/bash".into());
    let mut process = Command::new(shell);
    process.args(["-ilc", command]).stdin(Stdio::null());
    process
}

fn now_ms() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_steps_and_says_what_failed() {
        let ok = Step::Run {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "cat >/dev/null".into()],
            stdin: Some("hi".into()),
        };
        run_steps(&[ok, Step::Wait(Duration::from_millis(1))]).unwrap();
        let fail = Step::Run {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "echo nope >&2; exit 3".into()],
            stdin: None,
        };
        assert!(format!("{:#}", run_steps(&[fail]).unwrap_err()).contains("nope"));
    }

    #[test]
    fn finds_programs_on_a_path() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("codex"), "#!/bin/sh\n").unwrap();
        fs::write(bin.join("claude"), "not executable").unwrap();
        fs::set_permissions(bin.join("codex"), fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!("/nonexistent:{}", bin.display());
        assert!(on_path(&path, "codex"));
        assert!(!on_path(&path, "claude"));
    }
}
