//! Starting agent sessions from the menu: in a terminal window running a script (no Automation
//! permission), or in the background through the user's login shell. Either way the work is
//! `asides <agent>`'s, the CLI in the bundle.
//!
//! Terminal and iTerm run a `.command` script they're asked to open, as when it's double-clicked.
//! Ghostty asks before it runs a script it's asked to open, so it's started as its own process
//! with the script as its initial command instead, and quits when that window closes.
//!
//! The app is launched by Launch Services with a bare PATH, so the agents (installed with npm,
//! Homebrew, their own installers) are found and run through the user's login shell, which has
//! the PATH their terminal has.

use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use asides_core::launch::AgentKind;

/// Terminal apps tried, in order, when `[agents] terminal` doesn't name one. All of them run
/// `.command` scripts they're asked to open.
const TERMINALS: &[&str] = &["Ghostty", "iTerm", "Terminal"];
/// How long the login shell may take to say which agents are installed.
const SHELL_TIMEOUT: Duration = Duration::from_secs(15);

/// The `asides` CLI that ships with this app: in the bundle's Helpers, or beside the binary in a
/// build folder.
pub fn cli() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join("../Helpers/asides"), dir.join("asides")]
        .into_iter()
        .find(|p| p.is_file())
        .and_then(|p| p.canonicalize().ok())
}

/// `command` (which runs the bundled CLI) with its links sent back to this copy of the app: Launch
/// Services could otherwise pick another copy with the same bundle ID (a release and a build).
pub fn from_this_app(command: &str) -> String {
    let bundle = std::env::current_exe().ok().and_then(|exe| exe.ancestors().nth(3).map(Path::to_path_buf));
    match bundle.filter(|b| b.extension().is_some_and(|e| e == "app")) {
        Some(bundle) => format!("ASIDES_APP={} {command}", quote(&bundle.to_string_lossy())),
        None => command.to_string(),
    }
}

/// The terminal app sessions open in: the configured one, or the first installed of [`TERMINALS`].
pub fn app(configured: &str) -> String {
    if !configured.trim().is_empty() {
        return configured.trim().to_string();
    }
    let home = std::env::home_dir().unwrap_or_default();
    let installed = |name: &str| {
        [PathBuf::from("/Applications"), home.join("Applications"), "/System/Applications/Utilities".into()]
            .iter()
            .any(|dir| dir.join(format!("{name}.app")).exists())
    };
    TERMINALS.iter().find(|name| installed(name)).unwrap_or(&"Terminal").to_string()
}

/// Opens a window of `terminal` in `dir` running `command` (a shell command line) in the user's
/// login shell. When it ends, the window keeps a shell there.
pub fn open(terminal: &str, dir: &Path, command: &str) -> Result<()> {
    let scripts = std::env::temp_dir().join("asides-sessions");
    fs::create_dir_all(&scripts)?;
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let script = scripts.join(format!("{}-{}.command", safe(&name), now_ms()));
    // Ghostty runs it directly, not through the login shell: the shell runs the command, for PATH.
    let then = format!("{command}; exec \"${{SHELL:-/bin/zsh}}\" -l");
    let body = format!(
        "#!/bin/sh\n# Started by Asides: {command}\nrm -f \"$0\"\ncd {} || exit 1\nexec \"${{SHELL:-/bin/zsh}}\" -ilc {}\n",
        quote(&dir.to_string_lossy()),
        quote(&then)
    );
    fs::write(&script, body)?;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755))?;
    let mut open = Command::new("open");
    if terminal.eq_ignore_ascii_case("ghostty") {
        // As an option, not a bare path: AppKit would also open a path argument as a document.
        let initial = format!("--initial-command={}", script.display());
        open.args(["-na", terminal, "--args", "--quit-after-last-window-closed=true", &initial]);
    } else {
        open.arg("-a").arg(terminal).arg(&script);
    }
    let out = open.output()?;
    if !out.status.success() {
        let _ = fs::remove_file(&script);
        bail!(
            "{terminal} didn't open ({}). Set [agents] terminal in the settings",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// Runs `command` in the user's login shell and returns its output, or what went wrong.
pub fn run(command: &str) -> Result<String> {
    let out = login_shell(command).output().context("could not start the login shell")?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("it failed");
        bail!("{}", last.trim().trim_start_matches("asides: "));
    }
    Ok(text)
}

/// The agents installed for the user: programs on their login shell's PATH (`command -v` would
/// report an alias, which `asides <agent>` doesn't run).
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

/// `text` as one shell word.
pub fn quote(text: &str) -> String {
    if !text.is_empty() && text.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+@%=:,".contains(c)) {
        text.to_string()
    } else {
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}

fn safe(name: &str) -> String {
    let name: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).collect();
    if name.is_empty() { "session".into() } else { name }
}

fn now_ms() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_and_names_safely() {
        assert_eq!(quote("/Users/sam/My Repo"), "'/Users/sam/My Repo'");
        assert_eq!(quote("plain/path"), "plain/path");
        assert_eq!(safe("my shop!"), "my-shop-");
        assert_eq!(safe(""), "session");
        assert_eq!(app(" Warp "), "Warp");
    }

    /// Opens a real terminal window, as New Agent Session does: run with `--ignored`, with
    /// ASIDES_TERMINAL_DIR and ASIDES_TERMINAL_COMMAND set (and ASIDES_TERMINAL to pick the app).
    #[test]
    #[ignore]
    fn opens_a_terminal() {
        let (Ok(dir), Ok(command)) = (std::env::var("ASIDES_TERMINAL_DIR"), std::env::var("ASIDES_TERMINAL_COMMAND"))
        else {
            return;
        };
        let terminal = app(&std::env::var("ASIDES_TERMINAL").unwrap_or_default());
        open(&terminal, Path::new(&dir), &command).unwrap();
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
