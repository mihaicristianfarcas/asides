//! How an aside is typed into an agent's session, the way the user would type it, so it shows in the
//! chat as their message: a plan of steps for the app hosting the session's terminal (the app runs
//! them, `sessions.rs` finds the sessions). And how a new session starts with an aside as its first
//! message.
//!
//! Measured on 2026-10-18 with Claude Code, Codex and OpenCode in each app:
//! - **Diri**: `dirijor session send --no-submit` pastes the text (a bracketed paste), and
//!   `session key` presses keys. A screenshot goes in with the clipboard and Ctrl+V, which all three
//!   agents attach (a pasted path stays text in Claude Code and OpenCode there).
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
        Host::Other { name } => {
            Err(format!("asides can't type into {name}. Run the agent in Terminal, Ghostty or Diri."))
        }
    }
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
            // doesn't send text that's only whitespace).
            Part::Text(text) => {
                let before = if i > 0 { "\n\n" } else { "" };
                let after = if i + 1 < parts.len() { "\n\n" } else { "" };
                steps.push(run(&["session", "send", id, "--no-submit"], Some(format!("{before}{text}{after}"))));
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

/// The apps new sessions open in, which asides can also type into, as Settings offers them. Terminal
/// is the default.
pub const TERMINALS: &[&str] = &["Terminal", "Ghostty", "Diri"];

/// Where the app `name` is installed, if it is (names match ignoring case, as the file system does).
pub fn app_bundle(name: &str) -> Option<PathBuf> {
    let home = std::env::home_dir().unwrap_or_default();
    [PathBuf::from("/Applications"), home.join("Applications"), "/System/Applications/Utilities".into()]
        .into_iter()
        .map(|dir| dir.join(format!("{name}.app")))
        .find(|app| app.exists())
}

/// The apps of [`TERMINALS`] that are installed.
pub fn installed_terminals() -> Vec<&'static str> {
    TERMINALS.iter().copied().filter(|name| app_bundle(name).is_some()).collect()
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
    let mut words = agent.command(message, extra);
    if agent == AgentKind::Claude {
        // Its screenshots are outside the repo: readable without asking.
        for dir in readable {
            words.splice(1..1, ["--add-dir".to_string(), dir.to_string_lossy().into_owned()]);
        }
    }
    let command = words.iter().map(|w| sh_quote(w)).collect::<Vec<_>>().join(" ");
    format!(
        "#!/bin/sh\n# Started by asides: {name} in {dir}.\nrm -f -- \"$0\"\nlogin_path=$(\"${{SHELL:-/bin/zsh}}\" -ilc 'printf %s \"$PATH\"' \
         2>/dev/null </dev/null) && [ -n \"$login_path\" ] && PATH=$login_path\nexport PATH\ncd {qdir} || exit 1\nexport {var}={launch}\nexec \
         {command}\n",
        name = agent.name(),
        dir = dir.display(),
        qdir = sh_quote(&dir.to_string_lossy()),
        var = crate::agents::LAUNCH_VAR,
        launch = sh_quote(launch),
    )
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
                send("The \"Pay\" button is too low.\n\n"),
                Step::ClipboardImage("/h/1.png".into()),
                key(&["v", "--ctrl"]),
                Step::Wait(ATTACH),
                send("\n\n### 1. button"),
                Step::RestoreClipboard,
                Step::Wait(BEFORE_SUBMIT),
                key(&["tab"]),
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
        assert!(script.ends_with("exec 'claude' '--add-dir' '/h' 'It'\\''s broken'\n"), "{script}");
        assert_eq!(launch_line(Path::new("/tmp/a b.sh")), "'/tmp/a b.sh'");
        let ghostty = open_terminal_script(&Host::Ghostty, Path::new("/r"), "'/t/x.sh'").unwrap();
        assert!(ghostty.contains("set initial input of c to \"'/t/x.sh'\" & linefeed"), "{ghostty}");
    }
}
