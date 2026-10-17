//! Starting a coding agent that listens for asides (see `agents.rs` for the agents), with asides'
//! MCP server given on its command line (no `mcp add` needed), tied to one session, and the listen
//! prompt as its first message. `asides claude` (and the menu's New Session, which runs it in a
//! terminal) builds the command here.

use std::path::Path;

use serde_json::{Value, json};

use crate::agents::AgentKind;

/// The variables `asides <agent>` sets for the agent, so the `asides` commands it runs (and an
/// `asides mcp` it was already set up with) know their session and repo, even from a worktree.
pub const SESSION_VAR: &str = "ASIDES_SESSION";
pub const REPO_VAR: &str = "ASIDES_REPO";

/// How long a launched agent that waits for asides in a tool call (Codex without its daemon,
/// Gemini CLI, Copilot CLI, Qwen Code) waits in one `wait_for_asides` call, in seconds, with its
/// client's tool timeout raised to match; their defaults give up after a minute or ten.
pub const LONG_WAIT: u64 = 3000;
const LONG_TOOL_TIMEOUT: u64 = 3600;
/// Claude Code settings for a session's worktree: branch from the current HEAD.
const WORKTREE_FROM_HEAD: &str = r#"{"worktree":{"baseRef":"head"}}"#;

/// How an agent listens.
#[derive(Debug, Clone, PartialEq)]
pub enum Via {
    /// Watching this `asides listen` command in the background (Claude Code's Monitor tool).
    Monitor(String),
    /// Waiting in `wait_for_asides` calls.
    Wait,
    /// Asides sends each aside as a message.
    Messages,
}

/// The listen prompt: what the agent is asked first, or gets from `/asides:listen`. `worktree` is
/// the repo when the agent works in a worktree of it.
pub fn listen_prompt(via: &Via, worktree: Option<&Path>) -> String {
    let wait = "call the wait_for_asides tool; whenever it returns asides, list them for me in one short line \
                each (#N and a few words, since I can't see tool results), act on each one, then call \
                wait_for_asides again. If it returns none, call it again straight away.";
    let mut text = "Listen to my asides: things I say while I test, transcribed on my Mac. Each one is a request \
                    from me."
        .to_string();
    if let Some(repo) = worktree {
        text.push_str(&format!(
            " You work in a git worktree of {}, so your changes don't collide with another session's; my notes \
             are in that repo's .asides folder.",
            repo.display()
        ));
    }
    match via {
        Via::Monitor(command) => text.push_str(&format!(
            "\n\nStart a background monitor (the Monitor tool) with the description \"asides\" and timeout_ms \
             1800000 that runs:\n\n{command}\n\nEach event is an aside or a batch of them, which I see in the chat \
             too: act on it, no need to repeat it. Between asides, you're free: I may ask you other things. When \
             the monitor's last event says it stopped, start a new monitor with the command it gives, to keep \
             listening. If you don't have the Monitor tool, {wait}"
        )),
        Via::Wait => text.push_str(&format!("\n\nTo listen, {wait}")),
        Via::Messages => text.push_str(
            "\n\nAsides sends you each aside as a message as soon as it's transcribed (or a batch, when I send \
             them): act on it when it arrives. There's nothing to wait for or poll in between; I may ask you \
             other things. For now, say in a sentence that you're ready.",
        ),
    }
    text.push_str(
        "\n\nWhen you've handled an aside, call resolve_aside with its number and a short summary of what you \
         did (dismiss_aside with a reason if you won't act on it, reply_to_aside to ask me something); I see \
         those in the Asides menu.\n\nKeep listening until I tell you to stop.",
    );
    text
}

/// A session to start.
pub struct Launch<'a> {
    pub kind: AgentKind,
    /// The `asides` program, as the agent should run it.
    pub asides: &'a str,
    pub repo: &'a Path,
    pub session: &'a str,
    /// Shown in Claude Code's prompt box, its terminal title, and `claude agents`.
    pub name: &'a str,
    pub prompt: &'a str,
    /// The listen command's start, to pre-approve it (Claude Code).
    pub listen_prefix: Option<&'a str>,
    /// Have the agent make a git worktree, with this name where it takes one.
    pub worktree: Option<&'a str>,
    /// Start a Claude Code background session.
    pub background: bool,
    /// Open this session asides started (a Codex thread, `codex resume`; an OpenCode session,
    /// `opencode --session`) instead of a new one.
    pub thread: Option<&'a str>,
    /// OpenCode before 2.0: servers sit directly under `mcp`, and there's no shared background
    /// service to stay away from.
    pub opencode_1: bool,
    pub extra: &'a [String],
}

#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// The agent's command line, with asides' MCP server set up for the session.
pub fn command(launch: &Launch) -> Command {
    let repo = launch.repo.to_string_lossy().into_owned();
    let mut server = vec!["-C".to_string(), repo.clone(), "mcp".into(), "--session".into(), launch.session.into()];
    let mut env = vec![(SESSION_VAR.to_string(), launch.session.to_string()), (REPO_VAR.to_string(), repo.clone())];
    let mut args: Vec<String> = Vec::new();
    if launch.kind.waits_long_when_started() {
        server.extend(["--timeout".into(), LONG_WAIT.to_string()]);
    }
    // How Copilot CLI and Qwen Code describe a local server: they take the same shape.
    let local = |server: &[String], fields: Value| {
        let mut config = json!({ "command": launch.asides, "args": server, "timeout": LONG_TOOL_TIMEOUT * 1000 });
        if let (Some(config), Value::Object(fields)) = (config.as_object_mut(), fields) {
            config.extend(fields);
        }
        config
    };
    match launch.kind {
        AgentKind::Claude => {
            let config =
                json!({ "mcpServers": { "asides": { "type": "stdio", "command": launch.asides, "args": server } } });
            // The prompt goes first: --mcp-config and --allowedTools take every value after them.
            args.push(launch.prompt.into());
            args.extend(["--mcp-config".into(), config.to_string(), "--allowedTools".into(), "mcp__asides".into()]);
            if let Some(prefix) = launch.listen_prefix {
                args.push(format!("Bash({prefix} *)"));
            }
            args.extend(["--name".into(), launch.name.into()]);
            if let Some(tree) = launch.worktree {
                // From the user's HEAD, as asides' own worktrees: by default Claude Code branches
                // from origin's default branch, without the branch the user is testing.
                args.extend(["--worktree".into(), tree.into(), "--settings".into(), WORKTREE_FROM_HEAD.into()]);
            }
            if launch.background {
                args.push("--bg".into());
            }
            args.extend(launch.extra.iter().cloned());
        }
        // A thread asides started in Codex's daemon already has its MCP server and first prompt.
        // It opens without `-c`: with configuration of its own, Codex runs a server of its own,
        // which the daemon's thread is locked against ("open in another app").
        AgentKind::Codex if launch.thread.is_some() => {
            args.extend(launch.extra.iter().cloned());
            args.extend(["resume".into(), launch.thread.unwrap_or_default().into()]);
        }
        AgentKind::Codex => {
            let set = |key: &str, value: String| ["-c".to_string(), format!("mcp_servers.asides.{key}={value}")];
            args.extend(set("command", toml_string(launch.asides)));
            args.extend(set(
                "args",
                format!("[{}]", server.iter().map(|a| toml_string(a)).collect::<Vec<_>>().join(", ")),
            ));
            args.extend(set("tool_timeout_sec", LONG_TOOL_TIMEOUT.to_string()));
            args.extend(launch.extra.iter().cloned());
            args.push(launch.prompt.into());
        }
        // A session asides started in OpenCode's service already has its MCP server and first
        // prompt; the TUI opens it there.
        AgentKind::OpenCode if launch.thread.is_some() => {
            args.extend(launch.extra.iter().cloned());
            args.extend(["--session".into(), launch.thread.unwrap_or_default().into()]);
        }
        AgentKind::OpenCode => {
            let mut command = vec![launch.asides.to_string()];
            command.extend(server);
            let server = json!({ "asides": { "type": "local", "command": command, "enabled": true } });
            let config =
                if launch.opencode_1 { json!({ "mcp": server }) } else { json!({ "mcp": { "servers": server } }) };
            env.push(("OPENCODE_CONFIG_CONTENT".into(), config.to_string()));
            // OpenCode 2 runs agents in a shared background service, which doesn't see this
            // configuration: the session gets a server of its own.
            if !launch.opencode_1 {
                args.push("--standalone".into());
            }
            args.extend(["--prompt".into(), launch.prompt.into()]);
            args.extend(launch.extra.iter().cloned());
        }
        // Gemini CLI takes servers only from settings files, and a system one of a session's own
        // only from a folder root owns: it uses the user's `asides` server (see `gemini_server`),
        // which is the session's through the variables above, since Gemini passes its environment
        // to its servers.
        AgentKind::Gemini => {
            args.extend(["--prompt-interactive".into(), launch.prompt.into()]);
            args.extend(launch.extra.iter().cloned());
        }
        // Copilot CLI adds servers for one session, and approves a server's tools by its name.
        // `--allow-tool` takes every value after it unless given with `=`.
        AgentKind::Copilot => {
            let config =
                json!({ "mcpServers": { "asides": local(&server, json!({ "type": "local", "tools": ["*"] })) } });
            args.extend(["--additional-mcp-config".into(), config.to_string(), "--allow-tool=asides".into()]);
            args.extend(["--name".into(), launch.name.into()]);
            args.extend(launch.extra.iter().cloned());
            args.extend(["--interactive".into(), launch.prompt.into()]);
        }
        // Qwen Code (from Gemini CLI) takes servers on its command line; trusted, as above.
        AgentKind::Qwen => {
            let config = json!({ "mcpServers": { "asides": local(&server, json!({ "trust": true })) } });
            args.extend(["--mcp-config".into(), config.to_string()]);
            args.extend(launch.extra.iter().cloned());
            args.extend(["--prompt-interactive".into(), launch.prompt.into()]);
        }
    }
    Command { program: launch.kind.program().into(), args, env }
}

/// The command that registers asides' MCP server with Gemini CLI for the user (trusted: its tools
/// run without asking), unless `settings` (their `~/.gemini/settings.json`, or the project's) has
/// one: Gemini CLI can't be given one on its command line.
pub fn gemini_server(asides: &str, settings: &[&str]) -> Option<Vec<String>> {
    // Gemini's settings may have comments; the server's key, quoted, is enough to tell.
    let has = |text: &&str| match serde_json::from_str::<Value>(text) {
        Ok(json) => json.pointer("/mcpServers/asides").is_some(),
        Err(_) => text.contains("\"asides\""),
    };
    (!settings.iter().any(has))
        .then(|| ["mcp", "add", "--scope", "user", "--trust", "asides", asides, "mcp"].map(String::from).to_vec())
}

/// A Claude Code background session, as `claude agents --json` lists it.
#[derive(Debug, PartialEq, Eq)]
pub struct Background {
    /// What `claude stop` takes (`claude attach` also takes the name).
    pub id: String,
    pub pid: Option<u32>,
}

/// The session named `name` in `claude agents --json`'s output (after whatever a login shell
/// printed first).
pub fn claude_background(agents_json: &str, name: &str) -> Option<Background> {
    let list = &agents_json[agents_json.find('[')?..=agents_json.rfind(']')?];
    let list: serde_json::Value = serde_json::from_str(list).ok()?;
    let found = list.as_array()?.iter().find(|s| s["name"].as_str() == Some(name))?;
    Some(Background {
        id: found["id"].as_str()?.to_string(),
        pid: found["pid"].as_u64().and_then(|pid| u32::try_from(pid).ok()),
    })
}

/// The configuration a Codex thread asides starts gets: its MCP server, for the session.
pub fn codex_thread_config(asides: &str, repo: &Path, session: &str) -> serde_json::Value {
    let args = server_args(repo, session);
    json!({ "mcp_servers": { "asides": { "command": asides, "args": args } } })
}

/// The command line of the MCP server an OpenCode session asides starts gets.
pub fn opencode_server(asides: &str, repo: &Path, session: &str) -> Vec<String> {
    let mut command = vec![asides.to_string()];
    command.extend(server_args(repo, session));
    command
}

fn server_args(repo: &Path, session: &str) -> Vec<String> {
    ["-C", &repo.to_string_lossy(), "mcp", "--session", session].map(String::from).to_vec()
}

/// `text` as a TOML string (a JSON string is a valid TOML basic string).
fn toml_string(text: &str) -> String {
    serde_json::Value::from(text).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_background_session_by_name() {
        let list = r#"[
            {"pid": 51, "id": "0a1b2c3d", "kind": "interactive", "name": "other"},
            {"pid": 77543, "id": "79133e0b", "kind": "background", "name": "my shop · asides e7e073ed"}
        ]"#;
        let found = claude_background(list, "my shop · asides e7e073ed");
        assert_eq!(found, Some(Background { id: "79133e0b".into(), pid: Some(77543) }));
        assert_eq!(claude_background(list, "my shop · asides 00000000"), None);
        assert_eq!(claude_background("not json", "other"), None);
        let after_a_banner = format!("Welcome back!\n{list}");
        assert_eq!(claude_background(&after_a_banner, "other").map(|s| s.id), Some("0a1b2c3d".into()));
    }

    fn launch(kind: AgentKind) -> Launch<'static> {
        Launch {
            kind,
            asides: "/Applications/Asides.app/Contents/Helpers/asides",
            repo: Path::new("/Users/sam/code/my shop"),
            session: "a1b2c3d4",
            name: "my shop · asides a1b2c3d4",
            prompt: "Listen to my asides.",
            listen_prefix: Some("asides listen"),
            worktree: None,
            background: false,
            thread: None,
            opencode_1: false,
            extra: &[],
        }
    }

    #[test]
    fn claude_gets_the_server_on_its_command_line_and_its_tools_approved() {
        let extra = ["--model".to_string(), "opus".into()];
        let command =
            command(&Launch { worktree: Some("asides-a1b2c3d4"), extra: &extra, ..launch(AgentKind::Claude) });
        assert_eq!(command.program, "claude");
        assert_eq!(command.args[0], "Listen to my asides.");
        let config: serde_json::Value = serde_json::from_str(&command.args[2]).unwrap();
        assert_eq!(
            config.pointer("/mcpServers/asides/args").unwrap(),
            &json!(["-C", "/Users/sam/code/my shop", "mcp", "--session", "a1b2c3d4"])
        );
        assert_eq!(
            command.args[3..],
            [
                "--allowedTools",
                "mcp__asides",
                "Bash(asides listen *)",
                "--name",
                "my shop · asides a1b2c3d4",
                "--worktree",
                "asides-a1b2c3d4",
                "--settings",
                r#"{"worktree":{"baseRef":"head"}}"#,
                "--model",
                "opus"
            ]
        );
        assert!(command.env.contains(&(SESSION_VAR.into(), "a1b2c3d4".into())));
        let background = super::command(&Launch { background: true, ..launch(AgentKind::Claude) });
        assert_eq!(background.args.last().map(String::as_str), Some("--bg"));
    }

    #[test]
    fn codex_opens_the_thread_asides_started() {
        let command = command(&Launch { thread: Some("01a1-thread"), ..launch(AgentKind::Codex) });
        assert_eq!(command.args, ["resume", "01a1-thread"], "no -c: it would lock Codex out of the thread");
        let config = codex_thread_config("asides", Path::new("/r/shop"), "s1");
        assert_eq!(config.pointer("/mcp_servers/asides/args/4").unwrap(), "s1");
        assert!(listen_prompt(&Via::Messages, None).contains("as a message"));
    }

    #[test]
    fn codex_gets_config_overrides_and_long_waits() {
        let command = command(&launch(AgentKind::Codex));
        assert_eq!(
            command.args,
            [
                "-c",
                "mcp_servers.asides.command=\"/Applications/Asides.app/Contents/Helpers/asides\"",
                "-c",
                "mcp_servers.asides.args=[\"-C\", \"/Users/sam/code/my shop\", \"mcp\", \"--session\", \"a1b2c3d4\", \
                 \"--timeout\", \"3000\"]",
                "-c",
                "mcp_servers.asides.tool_timeout_sec=3600",
                "Listen to my asides."
            ]
        );
    }

    #[test]
    fn opencode_gets_its_config_in_the_environment() {
        let command = command(&launch(AgentKind::OpenCode));
        assert_eq!(command.args, ["--standalone", "--prompt", "Listen to my asides."]);
        let config = |command: &Command| -> serde_json::Value {
            let (_, config) = command.env.iter().find(|(k, _)| k == "OPENCODE_CONFIG_CONTENT").unwrap();
            serde_json::from_str(config).unwrap()
        };
        assert_eq!(config(&command).pointer("/mcp/servers/asides/command/3").unwrap(), "mcp");
        let old = super::command(&Launch { opencode_1: true, ..launch(AgentKind::OpenCode) });
        assert_eq!(old.args, ["--prompt", "Listen to my asides."]);
        assert_eq!(config(&old).pointer("/mcp/asides/command/4").unwrap(), "--session");
    }

    #[test]
    fn opencode_opens_the_session_asides_started() {
        let command = command(&Launch { thread: Some("ses_1"), ..launch(AgentKind::OpenCode) });
        assert_eq!(command.args, ["--session", "ses_1"], "no --standalone: the session is in the service");
        assert!(!command.env.iter().any(|(k, _)| k == "OPENCODE_CONFIG_CONTENT"));
        assert_eq!(
            opencode_server("asides", Path::new("/r/shop"), "s1"),
            ["asides", "-C", "/r/shop", "mcp", "--session", "s1"]
        );
    }

    #[test]
    fn gemini_uses_the_users_server_as_the_session() {
        let command = command(&launch(AgentKind::Gemini));
        assert_eq!(command.args, ["--prompt-interactive", "Listen to my asides."]);
        assert!(command.env.contains(&(SESSION_VAR.into(), "a1b2c3d4".into())));
        let add = gemini_server("/bin/asides", &[r#"{"mcpServers": {"github": {}}}"#]).unwrap();
        assert_eq!(add, ["mcp", "add", "--scope", "user", "--trust", "asides", "/bin/asides", "mcp"]);
        assert_eq!(gemini_server("asides", &["{}", r#"{"mcpServers": {"asides": {"command": "asides"}}}"#]), None);
        assert_eq!(gemini_server("asides", &["// mine\n{\"mcpServers\": {\"asides\": {}}}"]), None, "with comments");
    }

    #[test]
    fn copilot_and_qwen_get_the_server_on_their_command_lines() {
        let command = command(&launch(AgentKind::Copilot));
        assert_eq!(command.args[0], "--additional-mcp-config");
        let config: Value = serde_json::from_str(&command.args[1]).unwrap();
        assert_eq!(config["mcpServers"]["asides"]["type"], "local");
        assert_eq!(config["mcpServers"]["asides"]["tools"], json!(["*"]));
        assert_eq!(config.pointer("/mcpServers/asides/args/4").unwrap(), "a1b2c3d4");
        assert_eq!(
            command.args[2..],
            ["--allow-tool=asides", "--name", "my shop · asides a1b2c3d4", "--interactive", "Listen to my asides."]
        );
        let command = super::command(&launch(AgentKind::Qwen));
        assert_eq!(
            (command.args[0].as_str(), command.args[2..].to_vec()),
            ("--mcp-config", vec!["--prompt-interactive".to_string(), "Listen to my asides.".into()])
        );
        let config: Value = serde_json::from_str(&command.args[1]).unwrap();
        assert_eq!(config["mcpServers"]["asides"]["trust"], true);
        assert_eq!(config["mcpServers"]["asides"]["timeout"], 3_600_000);
    }

    #[test]
    fn the_listen_prompt_says_how_to_listen() {
        let monitor = listen_prompt(&Via::Monitor("asides listen --session s".into()), None);
        assert!(monitor.contains("background monitor") && monitor.contains("\n\nasides listen --session s\n\n"));
        let wait = listen_prompt(&Via::Wait, Some(Path::new("/r/shop")));
        assert!(wait.contains("To listen, call the wait_for_asides tool") && wait.contains("worktree of /r/shop"));
        assert!(wait.ends_with("Keep listening until I tell you to stop."));
    }
}
