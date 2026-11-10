//! How an aside is typed into an agent's session, the way the user would type it, so it shows in the
//! chat as their message: a plan of steps for the app hosting the session's terminal (the app runs
//! them, `sessions.rs` finds the sessions). And how a new session starts with an aside as its first
//! message.
//!
//! Measured on 2026-10-18 with Claude Code, Codex and OpenCode in each app:
//! - **Diri**: `dirijor session send --no-submit` types the text as it is, so it goes between
//!   bracketed-paste markers (typed bare, text longer than a pty's input queue, 1,022 bytes, lost
//!   all but its last part in Claude Code), and `session key` presses keys. A screenshot goes in with the clipboard
//!   and Ctrl+V, which all three agents attach (a pasted path stays text in Claude Code and OpenCode
//!   there).
//! - **Ghostty**: AppleScript's `input text` pastes, `send key` presses keys. Ghostty 1.3 drops Ctrl
//!   keys sent that way when the agent uses the kitty keyboard protocol (all three do), so a
//!   screenshot goes in as its path, pasted on its own: all three agents attach it.
//! - **Terminal**: `do script … in <tab>` types its text and a Return in one burst, which agents take
//!   for a paste (the Return a new line, never sent): the text is wrapped in bracketed-paste markers,
//!   after which the Return sends it. A screenshot goes in as its path, pasted: Codex and OpenCode
//!   attach it as it's pasted, so it goes in the same burst as the text (alone, its Return sent the
//!   screenshot alone); Claude Code attaches it a moment later, so it's pasted on its own first (its
//!   Return finds nothing to send yet) and the text 1.5 s after.
//! - Codex gets Tab instead of Return (`AgentKind::submit_key`): Return joins the turn it's working
//!   on, and it may answer only the newer message.
//! - **Linux**: any terminal, through its pty (`Step::Pty`: `asides-keys` writes what the keyboard
//!   would), where the system lets asides, or what a terminal takes itself, which needs nothing
//!   allowed: tmux's `paste-buffer`, Konsole's D-Bus `sendText`, `wezterm cli send-text`, kitty's
//!   `kitten @ send-text`. As in Terminal, what's typed is a bracketed paste, then the key that sends
//!   it. Screenshots go in as paths.
//! - **A console on Windows** (Windows Terminal, a console window, VS Code's terminal): its input
//!   is written as keys pressed there (`Step::Console`, the app's `WriteConsoleInput`), whichever
//!   app shows it. Text goes between bracketed-paste markers, so its new lines don't send it, then
//!   Return; a screenshot's path is pasted on its own first, as in Terminal. Codex reads keys there
//!   without bracketed paste (an Esc would interrupt it): its text is typed in one burst, which it
//!   takes for a paste, new lines and all.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::agents::{AgentKind, SubmitKey};
use crate::markdown::Part;
use crate::sessions::{AgentSession, Host};

/// One thing to do to type a message.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Runs a program with these arguments, writing `stdin` to it.
    Run {
        program: PathBuf,
        args: Vec<String>,
        stdin: Option<String>,
    },
    /// Runs an AppleScript.
    Script(String),
    /// Puts an image (a PNG's path) on the clipboard, keeping what was there to put back.
    ClipboardImage(PathBuf),
    /// Puts back what the clipboard held before.
    RestoreClipboard,
    Wait(Duration),
    /// Types `text` into the console of process `pid` (Windows), each character a key pressed there:
    /// `\r` is Return, `\t` Tab, `\x1b` Esc.
    Console {
        pid: i32,
        text: String,
    },
    /// Types `text` into a pty on Linux (`pts/3`), as its terminal's keyboard would: `asides-keys`.
    Pty {
        tty: String,
        text: String,
    },
    /// Types `text` into a tab of Konsole, over D-Bus (`org.kde.konsole.Session.sendText`).
    Konsole {
        service: String,
        session: String,
        text: String,
    },
}

/// How long an agent takes to read the clipboard after Ctrl+V, or to attach a pasted path.
const ATTACH: Duration = Duration::from_millis(1500);
/// Between pasting the text and the key that sends it.
const BEFORE_SUBMIT: Duration = Duration::from_millis(500);

/// The steps that type `parts` into `session` and send them. `dirijor` is Diri's command.
pub fn plan(session: &AgentSession, parts: &[Part], dirijor: &Path) -> Result<Vec<Step>, String> {
    let parts: Vec<&Part> = parts.iter().filter(|p| !matches!(p, Part::Text(t) if t.trim().is_empty())).collect();
    if parts.is_empty() {
        return Err("There's nothing to send.".into());
    }
    let submit = session.agent.submit_key();
    match &session.host {
        Host::Diri { session: id } => Ok(diri(id, &parts, submit, dirijor)),
        Host::Ghostty => Ok(vec![Step::Script(ghostty(&session.dir, &parts, submit))]),
        Host::Terminal => Ok(terminal(session.agent, &session.tty, &parts, submit)),
        Host::Console { .. } => Ok(console(session.agent, session.pid, &parts, submit)),
        Host::Tmux { socket, pane } => Ok(tmux(socket, pane, session.agent, &parts, submit)),
        Host::Konsole { service, session: tab } => {
            let send = |text: String| Step::Konsole { service: service.clone(), session: tab.clone(), text };
            Ok(typed(session.agent, &parts, submit, send))
        }
        Host::WezTerm { pane, socket } => {
            // Its own mux, by the variable its command reads: a session may be another instance's.
            let mut args: Vec<String> = socket.iter().map(|s| format!("WEZTERM_UNIX_SOCKET={s}")).collect();
            args.extend(["wezterm", "cli", "send-text", "--no-paste", "--pane-id", pane].map(String::from));
            let send = |text: String| Step::Run { program: "env".into(), args: args.clone(), stdin: Some(text) };
            Ok(typed(session.agent, &parts, submit, send))
        }
        Host::Kitty { listen, window } => {
            let args: Vec<String> = ["@", "--to", listen, "send-text", "--match", &format!("id:{window}"), "--stdin"]
                .map(String::from)
                .into();
            let send = |text: String| Step::Run { program: "kitten".into(), args: args.clone(), stdin: Some(text) };
            Ok(typed(session.agent, &parts, submit, send))
        }
        Host::Pty { name, may_type } => {
            if !may_type {
                return Err(format!(
                    "this system doesn't let asides type into {name} yet: allow it in Settings → General, or run the \
                     agent in tmux"
                ));
            }
            let tty = session.tty.clone();
            Ok(typed(session.agent, &parts, submit, |text| Step::Pty { tty: tty.clone(), text }))
        }
        Host::Other { name } => {
            let elsewhere = match cfg!(windows) {
                true => "a console",
                false => "Terminal, Ghostty or Diri",
            };
            Err(format!("asides can't type into {name}. Run the agent in {elsewhere}."))
        }
    }
}

/// The plan for a terminal typed into byte by byte, as its keyboard would (`send`): a bracketed paste
/// and the key that sends it. Claude Code attaches a pasted path a moment after the paste: on its
/// own, ahead of the text. The others attach it as it's pasted.
fn typed(agent: AgentKind, parts: &[&Part], submit: SubmitKey, send: impl Fn(String) -> Step) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut burst = String::new();
    for part in parts {
        match part {
            Part::Image(path) if agent == AgentKind::Claude => {
                steps.push(send(pasted(path)));
                steps.push(Step::Wait(ATTACH));
            }
            Part::Image(path) => burst.push_str(&pasted(path)),
            Part::Text(text) => burst.push_str(&pasted(text)),
        }
    }
    if !burst.is_empty() {
        steps.push(send(burst));
    }
    steps.push(Step::Wait(BEFORE_SUBMIT));
    steps.push(send(if submit == SubmitKey::Tab { "\t" } else { "\r" }.to_string()));
    steps
}

/// Through tmux, which knows whether the program in the pane wants bracketed pastes, and turns new
/// lines into Returns: `paste-buffer -p`.
fn tmux(socket: &str, pane: &str, agent: AgentKind, parts: &[&Part], submit: SubmitKey) -> Vec<Step> {
    let run = |args: &[&str], stdin: Option<String>| Step::Run {
        program: "tmux".into(),
        args: ["-S", socket].iter().chain(args).map(|a| a.to_string()).collect(),
        stdin,
    };
    let paste = |text: &str, steps: &mut Vec<Step>| {
        steps.push(run(&["load-buffer", "-b", "asides", "-"], Some(text.to_string())));
        steps.push(run(&["paste-buffer", "-p", "-d", "-b", "asides", "-t", pane], None));
    };
    let mut steps = Vec::new();
    let mut burst: Vec<&str> = Vec::new();
    for part in parts {
        match part {
            Part::Image(path) if agent == AgentKind::Claude => {
                paste(path, &mut steps);
                steps.push(Step::Wait(ATTACH));
            }
            Part::Image(path) | Part::Text(path) => burst.push(path),
        }
    }
    if !burst.is_empty() {
        paste(&burst.join("\n\n"), &mut steps);
    }
    steps.push(Step::Wait(BEFORE_SUBMIT));
    let key = if submit == SubmitKey::Tab { "Tab" } else { "Enter" };
    steps.push(run(&["send-keys", "-t", pane, key], None));
    steps
}

fn diri(id: &str, parts: &[&Part], submit: SubmitKey, dirijor: &Path) -> Vec<Step> {
    let run = |args: &[&str], stdin: Option<String>| Step::Run {
        program: dirijor.to_path_buf(),
        args: args.iter().map(|a| a.to_string()).collect(),
        stdin,
    };
    let mut steps = Vec::new();
    let mut clipboard = false;
    for (i, part) in parts.iter().enumerate() {
        match part {
            // Apart from the screenshots around it by a blank line, in the paste itself (Diri
            // doesn't send text that's only whitespace). A bracketed paste: Diri types `--no-submit`
            // text as it is, which reaches the agent in reads of a pty's input queue (1,022 bytes
            // on macOS), and unbracketed, Claude Code kept only the last.
            Part::Text(text) => {
                let before = if i > 0 { "\n\n" } else { "" };
                let after = if i + 1 < parts.len() { "\n\n" } else { "" };
                let paste = pasted(&format!("{before}{text}{after}"));
                steps.push(run(&["session", "send", id, "--no-submit"], Some(paste)));
            }
            Part::Image(path) => {
                clipboard = true;
                steps.push(Step::ClipboardImage(PathBuf::from(path)));
                steps.push(run(&["session", "key", id, "v", "--ctrl"], None));
                steps.push(Step::Wait(ATTACH));
            }
        }
    }
    if clipboard {
        steps.push(Step::RestoreClipboard);
    }
    steps.push(Step::Wait(BEFORE_SUBMIT));
    let key = if submit == SubmitKey::Tab { "tab" } else { "enter" };
    steps.push(run(&["session", "key", id, key], None));
    steps
}

/// An AppleScript string expression for `text` (lines joined with linefeeds).
pub fn applescript_string(text: &str) -> String {
    let literal = |line: &str| format!("\"{}\"", line.replace('\\', "\\\\").replace('"', "\\\""));
    text.split('\n').map(literal).collect::<Vec<_>>().join(" & linefeed & ")
}

fn ghostty(dir: &Path, parts: &[&Part], submit: SubmitKey) -> String {
    let mut body = String::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            body.push_str("    input text linefeed to t\n");
        }
        match part {
            Part::Text(text) => body.push_str(&format!("    input text {} to t\n", applescript_string(text))),
            Part::Image(path) => {
                body.push_str(&format!("    input text {} to t\n", applescript_string(path)));
                body.push_str(&format!("    delay {}\n", ATTACH.as_secs_f32()));
            }
        }
    }
    let key = if submit == SubmitKey::Tab { "tab" } else { "enter" };
    format!(
        "tell application id \"com.mitchellh.ghostty\"\n    set found to (every terminal whose working directory is {dir})\n    \
         if (count of found) is 0 then error \"Couldn't find its terminal in Ghostty.\"\n    set t to item 1 of found\n{body}    \
         delay {wait}\n    send key \"{key}\" to t\nend tell",
        dir = applescript_string(&dir.to_string_lossy()),
        wait = BEFORE_SUBMIT.as_secs_f32(),
    )
}

/// `do script` in Terminal's tab on `tty`.
fn in_terminal_tab(tty: &str, what: &str) -> String {
    format!(
        "tell application \"Terminal\"\n    repeat with w in windows\n        repeat with t in tabs of w\n            if tty of t is \
         {tty} then\n                do script ({what}) in t\n                return\n            end if\n        end repeat\n    end \
         repeat\n    error \"Couldn't find its tab in Terminal.\"\nend tell",
        tty = applescript_string(&format!("/dev/{tty}")),
    )
}

fn bracketed(text: &str) -> String {
    format!("(character id 27) & \"[200~\" & {} & (character id 27) & \"[201~\"", applescript_string(text))
}

fn terminal(agent: AgentKind, tty: &str, parts: &[&Part], submit: SubmitKey) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut burst: Vec<String> = Vec::new();
    for part in parts {
        match part {
            // Claude Code attaches a pasted path after the paste: on its own, ahead of the text.
            Part::Image(path) if agent == AgentKind::Claude => {
                steps.push(Step::Script(in_terminal_tab(tty, &bracketed(path))));
                steps.push(Step::Wait(ATTACH));
            }
            Part::Image(path) => burst.push(bracketed(path)),
            Part::Text(text) => burst.push(bracketed(text)),
        }
    }
    let mut what = burst.join(" & ");
    if submit == SubmitKey::Tab {
        // Tab queues it; the Return after it finds an empty prompt.
        what.push_str(" & (character id 9)");
    }
    steps.push(Step::Script(in_terminal_tab(tty, &what)));
    steps
}

/// Bracketed-paste markers around `text`; its new lines as Return (what a terminal sends for them).
fn pasted(text: &str) -> String {
    format!("\x1b[200~{}\x1b[201~", text.replace("\r\n", "\r").replace('\n', "\r"))
}

fn console(agent: AgentKind, pid: i32, parts: &[&Part], submit: SubmitKey) -> Vec<Step> {
    // Codex on Windows reads keys without bracketed paste: a burst of keys is its paste.
    let paste = |text: &str| match agent {
        AgentKind::Codex => text.replace("\r\n", "\r").replace('\n', "\r"),
        _ => pasted(text),
    };
    let mut steps = Vec::new();
    let mut texts: Vec<&str> = Vec::new();
    for part in parts {
        match part {
            // Attached a moment after it's pasted: on its own, ahead of the text.
            Part::Image(path) => {
                steps.push(Step::Console { pid, text: paste(path) });
                steps.push(Step::Wait(ATTACH));
            }
            Part::Text(text) => texts.push(text),
        }
    }
    if !texts.is_empty() {
        steps.push(Step::Console { pid, text: paste(&texts.join("\n\n")) });
    }
    steps.push(Step::Wait(BEFORE_SUBMIT));
    let key = if submit == SubmitKey::Tab { "\t" } else { "\r" };
    steps.push(Step::Console { pid, text: key.into() });
    steps
}

/// The apps new sessions open in, which asides can also type into, as Settings offers them. Terminal
/// is the default.
#[cfg(target_os = "macos")]
pub const TERMINALS: &[&str] = &["Terminal", "Ghostty", "Diri"];

/// The terminals new sessions open in on Linux (`linux_terminal` says how): the desktop's own comes
/// first where it's installed (`preferred_terminal`), then these, in order.
#[cfg(target_os = "linux")]
pub const TERMINALS: &[&str] = &[
    "Ptyxis",
    "GNOME Terminal",
    "Console",
    "Konsole",
    "Ghostty",
    "kitty",
    "WezTerm",
    "Alacritty",
    "foot",
    "Tilix",
    "Terminator",
    "Xfce Terminal",
    "MATE Terminal",
    "COSMIC Terminal",
    "xterm",
];

/// How a terminal on Linux is started, in a folder, running a program: its command, then the arguments
/// before the folder's, whether the folder goes in one with `=`, and what comes before the program.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinuxTerminal {
    pub program: &'static str,
    pub folder: &'static [&'static str],
    pub run: &'static [&'static str],
}

/// [`TERMINALS`]' commands.
#[cfg(target_os = "linux")]
pub fn linux_terminal(name: &str) -> Option<LinuxTerminal> {
    let (program, folder, run): (&str, &[&str], &[&str]) = match name {
        "Ptyxis" => ("ptyxis", &["--new-window", "--working-directory"], &["--"]),
        "GNOME Terminal" => ("gnome-terminal", &["--working-directory"], &["--"]),
        "Console" => ("kgx", &["--working-directory"], &["--"]),
        "Konsole" => ("konsole", &["--workdir"], &["-e"]),
        "Ghostty" => ("ghostty", &["--working-directory"], &["-e"]),
        "kitty" => ("kitty", &["--directory"], &[]),
        "WezTerm" => ("wezterm", &["start", "--cwd"], &["--"]),
        "Alacritty" => ("alacritty", &["--working-directory"], &["-e"]),
        "foot" => ("foot", &["--working-directory"], &[]),
        "Tilix" => ("tilix", &["--working-directory"], &["-e"]),
        "Terminator" => ("terminator", &["--working-directory"], &["-x"]),
        "Xfce Terminal" => ("xfce4-terminal", &["--working-directory"], &["-x"]),
        "MATE Terminal" => ("mate-terminal", &["--working-directory"], &["-x"]),
        "COSMIC Terminal" => ("cosmic-term", &["--working-directory"], &["--"]),
        "xterm" => ("xterm", &[], &["-e"]),
        _ => return None,
    };
    Some(LinuxTerminal { program, folder, run })
}

/// The arguments that start `terminal` in `dir`, running `program`.
#[cfg(target_os = "linux")]
pub fn linux_terminal_args(terminal: &LinuxTerminal, dir: &Path, program: &[String]) -> Vec<String> {
    let mut args: Vec<String> = terminal.folder.iter().map(|a| a.to_string()).collect();
    match args.last_mut() {
        // Ghostty takes only `--option=value`, and all but WezTerm's and Konsole's take it too.
        Some(flag) if !matches!(terminal.program, "wezterm" | "konsole" | "kitty") => {
            flag.push('=');
            flag.push_str(&dir.to_string_lossy());
        }
        Some(_) => args.push(dir.to_string_lossy().into_owned()),
        None => {}
    }
    args.extend(terminal.run.iter().map(|a| a.to_string()));
    args.extend(program.iter().cloned());
    args
}

/// The terminals of [`TERMINALS`] that come with a desktop (`XDG_CURRENT_DESKTOP`).
#[cfg(target_os = "linux")]
pub fn preferred_terminal(desktop: &str) -> &'static [&'static str] {
    let desktop = desktop.to_ascii_uppercase();
    let has = |name: &str| desktop.split(':').any(|d| d == name);
    if has("KDE") {
        &["Konsole"]
    } else if has("XFCE") {
        &["Xfce Terminal"]
    } else if has("MATE") {
        &["MATE Terminal"]
    } else if has("COSMIC") {
        &["COSMIC Terminal"]
    } else if has("GNOME") || has("UBUNTU") || has("X-CINNAMON") || has("CINNAMON") {
        &["Ptyxis", "GNOME Terminal", "Console"]
    } else {
        &[]
    }
}

/// The apps new sessions open in on Windows: Windows Terminal (the default where it's installed),
/// or a console window of their own.
#[cfg(windows)]
pub const TERMINALS: &[&str] = &["Windows Terminal", "Console"];

/// The bundle ID of the terminal app `name`, one of [`TERMINALS`].
pub fn bundle_id(name: &str) -> Option<&'static str> {
    match name.to_ascii_lowercase().as_str() {
        "terminal" => Some("com.apple.Terminal"),
        "ghostty" => Some("com.mitchellh.ghostty"),
        "diri" => Some("com.dirijor.diri"),
        _ => None,
    }
}

/// The bundle ID of the terminal app `name` asides types into with Apple events (Automation): none
/// for Diri, which needs no permission.
#[cfg(not(target_os = "linux"))]
pub fn automated_bundle(name: &str) -> Option<&'static str> {
    bundle_id(name).filter(|_| !name.eq_ignore_ascii_case("diri"))
}

/// What must be allowed to type into the terminal `name`: on Linux the helper (`asides-keys`), but
/// for the terminals that take text from outside themselves, by their own commands.
#[cfg(target_os = "linux")]
pub fn automated_bundle(name: &str) -> Option<&'static str> {
    (!matches!(name, "Konsole" | "WezTerm")).then_some("asides-keys")
}

/// Where the app `name` is installed, if it is (names match ignoring case, as the file system does).
pub fn app_bundle(name: &str) -> Option<PathBuf> {
    let home = std::env::home_dir().unwrap_or_default();
    [PathBuf::from("/Applications"), home.join("Applications"), "/System/Applications/Utilities".into()]
        .into_iter()
        .map(|dir| dir.join(format!("{name}.app")))
        .find(|app| app.exists())
}

/// The apps of [`TERMINALS`] that are installed.
#[cfg(target_os = "macos")]
pub fn installed_terminals() -> Vec<&'static str> {
    TERMINALS.iter().copied().filter(|name| app_bundle(name).is_some()).collect()
}

/// The terminals of [`TERMINALS`] that are installed, the desktop's own first.
#[cfg(target_os = "linux")]
pub fn installed_terminals() -> Vec<&'static str> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let preferred = preferred_terminal(&desktop);
    let mut found: Vec<&'static str> = TERMINALS
        .iter()
        .copied()
        .filter(|name| {
            linux_terminal(name).is_some_and(|t| crate::sessions::linux::find_program(&path, t.program).is_some())
        })
        .collect();
    found.sort_by_key(|name| preferred.iter().position(|p| p == name).unwrap_or(preferred.len()));
    found
}

/// The apps of [`TERMINALS`] that are installed: Windows Terminal (its `wt` command), and the
/// console window, always there.
#[cfg(windows)]
pub fn installed_terminals() -> Vec<&'static str> {
    let terminal = crate::launch::find_program(&crate::launch::user_path(), "wt").is_some();
    TERMINALS.iter().copied().filter(|name| *name != "Windows Terminal" || terminal).collect()
}

/// The first message of a new session, as text: its screenshots as paths the agent can read.
pub fn first_message(parts: &[Part]) -> String {
    let mut n = 0;
    parts
        .iter()
        .map(|part| match part {
            Part::Text(text) => text.trim().to_string(),
            Part::Image(path) => {
                n += 1;
                format!("[Screenshot {n}: {path}]")
            }
        })
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// `text` quoted for `sh`.
pub fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// The `/bin/sh` script that starts `agent` in `dir` with `message` as its first message (and the
/// user's extra arguments). It finds the agent through the login shell's PATH (an app's is bare),
/// and marks it with `launch` (`ASIDES_LAUNCH`), so asides can tell the session it started.
pub fn launch_script(
    agent: AgentKind,
    dir: &Path,
    message: &str,
    extra: &[String],
    launch: &str,
    readable: &[PathBuf],
) -> String {
    let words = launch_words(agent, message, extra, readable);
    let command = words.iter().map(|w| sh_quote(w)).collect::<Vec<_>>().join(" ");
    format!(
        "#!/bin/sh\n# Started by asides: {name} in {dir}.\nrm -f -- \"$0\"\nlogin_path=$(\"${{SHELL:-{shell}}}\" -ilc 'printf %s \"$PATH\"' \
         2>/dev/null </dev/null) && [ -n \"$login_path\" ] && PATH=$login_path\nexport PATH\ncd {qdir} || exit 1\nexport {var}={launch}\nexec \
         {command}\n",
        name = agent.name(),
        dir = dir.display(),
        shell = if cfg!(target_os = "macos") { "/bin/zsh" } else { "/bin/bash" },
        qdir = sh_quote(&dir.to_string_lossy()),
        var = crate::agents::LAUNCH_VAR,
        launch = sh_quote(launch),
    )
}

/// The command line that starts `agent` with `message` as its first message (and the user's extra
/// arguments): its program, then its arguments. Claude Code may read `readable` without asking.
pub fn launch_words(agent: AgentKind, message: &str, extra: &[String], readable: &[PathBuf]) -> Vec<String> {
    let mut words = agent.command(message, extra);
    if agent == AgentKind::Claude {
        // Its screenshots are outside the repo: readable without asking.
        for dir in readable {
            words.splice(1..1, ["--add-dir".to_string(), dir.to_string_lossy().into_owned()]);
        }
    }
    words
}

/// The line typed into the user's shell (bash, zsh or fish alike) to run a launch script.
pub fn launch_line(script: &Path) -> String {
    sh_quote(&script.to_string_lossy())
}

/// The AppleScript that opens a new window running `line` in `app`'s user shell, in `dir`.
pub fn open_terminal_script(app: &Host, dir: &Path, line: &str) -> Option<String> {
    match app {
        Host::Terminal => Some(format!("tell application \"Terminal\" to do script {}", applescript_string(line))),
        Host::Ghostty => Some(format!(
            "tell application id \"com.mitchellh.ghostty\"\n    set c to new surface configuration\n    set initial working directory of c \
             to {}\n    set initial input of c to {} & linefeed\n    new window with configuration c\nend tell",
            applescript_string(&dir.to_string_lossy()),
            applescript_string(line),
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(agent: AgentKind, host: Host) -> AgentSession {
        AgentSession {
            pid: 42,
            agent,
            dir: PathBuf::from("/Users/me/shop"),
            tty: "ttys004".into(),
            host,
            last_input: None,
            name: None,
            launch: None,
        }
    }

    fn parts() -> Vec<Part> {
        vec![
            Part::Text("The \"Pay\" button is too low.".into()),
            Part::Image("/h/1.png".into()),
            Part::Text("### 1. button".into()),
        ]
    }

    #[test]
    fn types_into_a_pty_as_its_keyboard_would() {
        let host = Host::Pty { name: "GNOME Terminal".into(), may_type: true };
        let steps = plan(&session(AgentKind::Claude, host), &parts(), Path::new("")).unwrap();
        let pty = |text: &str| Step::Pty { tty: "ttys004".into(), text: text.into() };
        assert_eq!(
            steps,
            [
                pty("\x1b[200~/h/1.png\x1b[201~"),
                Step::Wait(ATTACH),
                pty("\x1b[200~The \"Pay\" button is too low.\x1b[201~\x1b[200~### 1. button\x1b[201~"),
                Step::Wait(BEFORE_SUBMIT),
                pty("\r"),
            ]
        );
        // Codex attaches a path as it's pasted, and Tab queues what's typed.
        let host = Host::Pty { name: "foot".into(), may_type: true };
        let steps = plan(&session(AgentKind::Codex, host), &parts(), Path::new("")).unwrap();
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[2], pty("\t"));
    }

    #[test]
    fn says_when_the_system_doesnt_let_it_type() {
        let host = Host::Pty { name: "GNOME Terminal".into(), may_type: false };
        let session = session(AgentKind::Claude, host);
        assert!(!session.host.reachable());
        assert!(plan(&session, &parts(), Path::new("")).unwrap_err().contains("GNOME Terminal"));
    }

    #[test]
    fn pastes_into_tmux_through_its_buffer() {
        let host = Host::Tmux { socket: "/tmp/tmux-1000/default".into(), pane: "%3".into() };
        let steps = plan(&session(AgentKind::Codex, host), &parts(), Path::new("")).unwrap();
        let run = |args: &[&str], stdin: Option<&str>| Step::Run {
            program: "tmux".into(),
            args: ["-S", "/tmp/tmux-1000/default"].iter().chain(args).map(|a| a.to_string()).collect(),
            stdin: stdin.map(String::from),
        };
        assert_eq!(
            steps,
            [
                run(
                    &["load-buffer", "-b", "asides", "-"],
                    Some("The \"Pay\" button is too low.\n\n/h/1.png\n\n### 1. button")
                ),
                run(&["paste-buffer", "-p", "-d", "-b", "asides", "-t", "%3"], None),
                Step::Wait(BEFORE_SUBMIT),
                run(&["send-keys", "-t", "%3", "Tab"], None),
            ]
        );
    }

    #[test]
    fn asks_konsole_over_dbus() {
        let host = Host::Konsole { service: ":1.9".into(), session: "/Sessions/2".into() };
        let steps = plan(&session(AgentKind::OpenCode, host), &parts(), Path::new("")).unwrap();
        assert!(matches!(&steps[0], Step::Konsole { service, session, text }
            if service == ":1.9" && session == "/Sessions/2" && text.starts_with("\x1b[200~")));
        assert!(matches!(&steps[2], Step::Konsole { text, .. } if text == "\r"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn starts_linux_terminals_in_a_folder() {
        let dir = Path::new("/home/me/my shop");
        let program = ["/bin/sh".to_string(), "/tmp/launch.sh".to_string()];
        let args = |name: &str| linux_terminal_args(&linux_terminal(name).unwrap(), dir, &program).join(" ");
        assert_eq!(args("GNOME Terminal"), "--working-directory=/home/me/my shop -- /bin/sh /tmp/launch.sh");
        assert_eq!(args("Ptyxis"), "--new-window --working-directory=/home/me/my shop -- /bin/sh /tmp/launch.sh");
        assert_eq!(args("Konsole"), "--workdir /home/me/my shop -e /bin/sh /tmp/launch.sh");
        assert_eq!(args("WezTerm"), "start --cwd /home/me/my shop -- /bin/sh /tmp/launch.sh");
        assert_eq!(args("kitty"), "--directory /home/me/my shop /bin/sh /tmp/launch.sh");
        assert_eq!(args("xterm"), "-e /bin/sh /tmp/launch.sh");
        for name in TERMINALS {
            assert!(linux_terminal(name).is_some(), "{name}");
        }
        assert_eq!(preferred_terminal("KDE"), ["Konsole"]);
        assert_eq!(preferred_terminal("ubuntu:GNOME")[0], "Ptyxis");
    }

    #[test]
    fn types_into_diri_with_ctrl_v_for_screenshots() {
        let dirijor = Path::new("/d/dirijor");
        let steps = plan(&session(AgentKind::Codex, Host::Diri { session: "s_1".into() }), &parts(), dirijor).unwrap();
        let send = |text: &str| Step::Run {
            program: dirijor.into(),
            args: ["session", "send", "s_1", "--no-submit"].map(String::from).to_vec(),
            stdin: Some(text.into()),
        };
        let key = |args: &[&str]| Step::Run {
            program: dirijor.into(),
            args: ["session", "key", "s_1"].iter().chain(args).map(|a| a.to_string()).collect(),
            stdin: None,
        };
        assert_eq!(
            steps,
            [
                send("\x1b[200~The \"Pay\" button is too low.\r\r\x1b[201~"),
                Step::ClipboardImage("/h/1.png".into()),
                key(&["v", "--ctrl"]),
                Step::Wait(ATTACH),
                send("\x1b[200~\r\r### 1. button\x1b[201~"),
                Step::RestoreClipboard,
                Step::Wait(BEFORE_SUBMIT),
                key(&["tab"]),
            ]
        );
    }

    #[test]
    fn types_into_a_windows_console_as_keys() {
        let into =
            |agent| plan(&session(agent, Host::Console { name: "Windows Terminal".into() }), &parts(), Path::new(""));
        let keys = |text: &str| Step::Console { pid: 42, text: text.into() };
        assert_eq!(
            into(AgentKind::Claude).unwrap(),
            [
                keys("\x1b[200~/h/1.png\x1b[201~"),
                Step::Wait(ATTACH),
                keys("\x1b[200~The \"Pay\" button is too low.\r\r### 1. button\x1b[201~"),
                Step::Wait(BEFORE_SUBMIT),
                keys("\r"),
            ]
        );
        // Codex: a burst of keys, no markers (an Esc would interrupt it); Tab queues it.
        assert_eq!(
            into(AgentKind::Codex).unwrap(),
            [
                keys("/h/1.png"),
                Step::Wait(ATTACH),
                keys("The \"Pay\" button is too low.\r\r### 1. button"),
                Step::Wait(BEFORE_SUBMIT),
                keys("\t"),
            ]
        );
    }

    #[test]
    fn types_into_ghostty_with_screenshots_as_paths() {
        let steps = plan(&session(AgentKind::Claude, Host::Ghostty), &parts(), Path::new("dirijor")).unwrap();
        let [Step::Script(script)] = steps.as_slice() else { panic!("{steps:?}") };
        assert!(script.contains("every terminal whose working directory is \"/Users/me/shop\""), "{script}");
        assert!(script.contains("input text \"The \\\"Pay\\\" button is too low.\" to t"), "{script}");
        assert!(script.contains("input text \"/h/1.png\" to t\n    delay 1.5"), "{script}");
        assert!(script.trim_end().ends_with("send key \"enter\" to t\nend tell"), "{script}");
    }

    #[test]
    fn types_into_terminal_as_a_bracketed_paste() {
        let steps = plan(&session(AgentKind::Claude, Host::Terminal), &parts(), Path::new("dirijor")).unwrap();
        assert_eq!(steps.len(), 3, "{steps:?}");
        let Step::Script(image) = &steps[0] else { panic!() };
        assert!(image.contains("if tty of t is \"/dev/ttys004\""), "{image}");
        assert!(image.contains("\"[200~\" & \"/h/1.png\" & (character id 27) & \"[201~\""), "{image}");
        assert_eq!(steps[1], Step::Wait(ATTACH));
        let Step::Script(text) = &steps[2] else { panic!() };
        assert!(text.contains("\"The \\\"Pay\\\" button is too low.\" & (character id 27)"), "{text}");
        assert!(text.contains("\"### 1. button\""), "{text}");
        // Codex and OpenCode attach as it's pasted: one burst, in order (Codex sent with Tab).
        let codex = plan(&session(AgentKind::Codex, Host::Terminal), &parts(), Path::new("d")).unwrap();
        let [Step::Script(text)] = codex.as_slice() else { panic!("{codex:?}") };
        let (pay, path, button) =
            (text.find("Pay").unwrap(), text.find("/h/1.png").unwrap(), text.find("### 1.").unwrap());
        assert!(pay < path && path < button, "{text}");
        assert!(text.contains("\"[201~\" & (character id 9)) in t"), "{text}");
    }

    #[test]
    fn says_which_apps_it_cant_type_into() {
        let other = session(AgentKind::Claude, Host::Other { name: "Warp".into() });
        assert!(plan(&other, &parts(), Path::new("d")).unwrap_err().contains("Warp"));
        assert!(plan(&session(AgentKind::Claude, Host::Terminal), &[Part::Text(" ".into())], Path::new("d")).is_err());
    }

    #[test]
    fn starts_new_sessions_with_the_aside() {
        assert_eq!(
            first_message(&parts()),
            "The \"Pay\" button is too low.\n\n[Screenshot 1: /h/1.png]\n\n### 1. button"
        );
        let script =
            launch_script(AgentKind::Claude, Path::new("/r/my shop"), "It's broken", &[], "l1", &[PathBuf::from("/h")]);
        assert!(script.contains("cd '/r/my shop' || exit 1"), "{script}");
        assert!(script.contains("export ASIDES_LAUNCH='l1'"), "{script}");
        assert!(script.ends_with("exec 'claude' '--add-dir' '/h' '--' 'It'\\''s broken'\n"), "{script}");
        assert_eq!(launch_line(Path::new("/tmp/a b.sh")), "'/tmp/a b.sh'");
        let ghostty = open_terminal_script(&Host::Ghostty, Path::new("/r"), "'/t/x.sh'").unwrap();
        assert!(ghostty.contains("set initial input of c to \"'/t/x.sh'\" & linefeed"), "{ghostty}");
    }
}
