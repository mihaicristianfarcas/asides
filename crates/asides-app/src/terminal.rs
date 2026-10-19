//! Typing asides into agent sessions (a `deliver::plan`'s steps: `dirijor`, AppleScript, the
//! clipboard), and starting new sessions: a script that runs the agent with the aside as its first
//! message, in a new window of Terminal (the default) or Ghostty, typed into the user's own shell, or
//! as a session of Diri. Always off the main thread: an AppleScript waits for the user to say whether
//! asides may control that app (the first time), up to two minutes.
//!
//! The app is launched by Launch Services with a bare PATH, so the agents (installed with npm,
//! Homebrew, their own installers) are found through the user's login shell, which has the PATH
//! their terminal has.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use asides_core::agents::AgentKind;
use asides_core::deliver::{self, Step};
use asides_core::sessions::Host;

use crate::clipboard;

/// How long the login shell may take to say which agents are installed.
const SHELL_TIMEOUT: Duration = Duration::from_secs(15);
/// How long an AppleScript may run: the first, the user answers whether asides may control the app.
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(150);

/// The app sessions open in: the configured one, or Terminal.
pub fn app(configured: &str) -> String {
    match configured.trim() {
        "" => "Terminal".into(),
        name => deliver::TERMINALS
            .iter()
            .find(|t| t.eq_ignore_ascii_case(name))
            .map_or("Terminal".into(), |t| t.to_string()),
    }
}

/// Diri's command (`dirijor`), in its bundle.
pub fn dirijor() -> PathBuf {
    deliver::app_bundle("Diri")
        .map(|app| app.join("Contents/Resources/bin/dirijor"))
        .unwrap_or_else(|| "dirijor".into())
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
            let mut child = Command::new(program)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .with_context(|| format!("could not run {}", program.display()))?;
            if let Some(text) = stdin {
                child.stdin.take().context("no stdin")?.write_all(text.as_bytes())?;
            }
            drop(child.stdin.take());
            let out = child.wait_with_output()?;
            if !out.status.success() {
                bail!("{}", last_line(&out.stderr, "it failed"));
            }
        }
        Step::Script(script) => applescript(script)?,
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

/// Runs an AppleScript (its source on standard input), and says what went wrong.
fn applescript(script: &str) -> Result<()> {
    let mut child = Command::new("/usr/bin/osascript")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("could not run osascript")?;
    child.stdin.take().context("no stdin")?.write_all(script.as_bytes())?;
    let started = Instant::now();
    while child.try_wait()?.is_none() {
        if started.elapsed() > SCRIPT_TIMEOUT {
            let _ = child.kill();
            bail!(
                "macOS didn't let asides control the app in time (System Settings → Privacy & Security → Automation)"
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        if err.contains("-1743") || err.contains("Not authorized") {
            bail!("asides may not control that app: allow it in System Settings → Privacy & Security → Automation");
        }
        // `… execution error: Couldn't find its tab in Terminal. (-2700)`
        let line = last_line(&out.stderr, "the AppleScript failed");
        let message = line.split("execution error: ").nth(1).unwrap_or(&line);
        bail!("{}", message.split(" (-").next().unwrap_or(message).trim());
    }
    Ok(())
}

fn last_line(bytes: &[u8], otherwise: &str) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or(otherwise).trim().to_string()
}

/// Starts `agent` in `dir` in `terminal` ("Terminal", "Ghostty" or "Diri") with `message` as its
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
    let scripts = asides_core::paths::launch_dir();
    fs::create_dir_all(&scripts)?;
    let launch = format!("{}-{}", agent.program(), now_ms());
    let script = scripts.join(format!("{launch}.sh"));
    fs::write(&script, deliver::launch_script(agent, dir, message, extra, &launch, readable))?;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755))?;
    let line = deliver::launch_line(&script);
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match app(terminal).as_str() {
        "Diri" => {
            let out = Command::new(dirijor())
                .args(["session", "run", "--title", &format!("{} ({name})", agent.name()), "--cwd"])
                .arg(dir)
                .args(["--", "/bin/sh"])
                .arg(&script)
                .output()
                .context("could not run Diri's dirijor")?;
            if !out.status.success() {
                let _ = fs::remove_file(&script);
                bail!("Diri didn't start the session: {}", last_line(&out.stderr, "it failed"));
            }
            let _ = Command::new("open").args(["-a", "Diri"]).status();
        }
        other => {
            let host = if other == "Ghostty" { Host::Ghostty } else { Host::Terminal };
            let script_source = deliver::open_terminal_script(&host, dir, &line).context("no way to open it")?;
            if let Err(err) = applescript(&script_source) {
                let _ = fs::remove_file(&script);
                return Err(err);
            }
            let _ = Command::new("open").args(["-a", other]).status();
        }
    }
    Ok(launch)
}

/// The agents installed for the user: programs on their login shell's PATH.
pub fn installed_agents() -> Vec<AgentKind> {
    let mut child = match login_shell("printf '%s' \"$PATH\"").stdout(Stdio::piped()).stderr(Stdio::null()).spawn() {
        Ok(child) => child,
        Err(_) => return Vec::new(),
    };
    let started = Instant::now();
    while child.try_wait().ok().flatten().is_none() {
        if started.elapsed() > SHELL_TIMEOUT {
            let _ = child.kill();
            return Vec::new();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut path = String::new();
    let _ = child.stdout.take().map(|mut out| out.read_to_string(&mut path));
    // Interactive shells may print a banner first: the PATH is the last line.
    let path = path.lines().last().unwrap_or_default();
    AgentKind::ALL.into_iter().filter(|kind| on_path(path, kind.program())).collect()
}

/// Whether `program` is an executable file in one of `path`'s directories.
fn on_path(path: &str, program: &str) -> bool {
    std::env::split_paths(path)
        .any(|dir| fs::metadata(dir.join(program)).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
}

/// `$SHELL -ilc command`: interactive too, since many people set their PATH in `.zshrc`.
fn login_shell(command: &str) -> Command {
    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/zsh".into());
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
    fn names_the_terminal_apps() {
        assert_eq!(app(" ghostty "), "Ghostty");
        assert_eq!(app(""), "Terminal");
        assert_eq!(app("Warp"), "Terminal", "only apps asides can type into");
        assert!(deliver::installed_terminals().contains(&"Terminal"));
    }

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
        assert_eq!(run_steps(&[fail]).unwrap_err().to_string(), "nope");
    }

    /// Types an aside into a Diri session of your own, as the app does: `ASIDES_DELIVER=<aside folder>
    /// ASIDES_DELIVER_TO=<Diri session id> cargo test -p asides-app types_into_diri -- --ignored`.
    #[test]
    #[ignore]
    fn types_into_diri() {
        let aside = asides_core::history::read(Path::new(&std::env::var("ASIDES_DELIVER").unwrap())).unwrap();
        let session = asides_core::sessions::AgentSession {
            pid: 0,
            agent: AgentKind::Claude,
            dir: "/".into(),
            tty: String::new(),
            host: Host::Diri { session: std::env::var("ASIDES_DELIVER_TO").unwrap() },
            last_input: None,
            name: None,
            launch: None,
        };
        run_steps(&deliver::plan(&session, &aside.parts(), &dirijor()).unwrap()).unwrap();
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
        assert!(!on_path(&path, "opencode"));
    }
}
