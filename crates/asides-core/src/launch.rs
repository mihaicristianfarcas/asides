//! Starting a coding agent that listens for asides: Claude Code, Codex or OpenCode, with asides'
//! MCP server given on its command line (no `mcp add` needed), tied to one session, and the listen
//! prompt as its first message. `asides claude` (and the menu's New Session, which runs it in a
//! terminal) builds the command here.

use std::path::Path;

use serde_json::json;

use crate::config::Agents;

/// The variables `asides <agent>` sets for the agent, so the `asides` commands it runs (and an
/// `asides mcp` it was already set up with) know their session and repo, even from a worktree.
pub const SESSION_VAR: &str = "ASIDES_SESSION";
pub const REPO_VAR: &str = "ASIDES_REPO";

/// How long a launched Codex waits in one `wait_for_asides` call, in seconds, with its tool
/// timeout raised to match; other clients' defaults give up after 60 s.
pub const CODEX_WAIT: u64 = 3000;
const CODEX_TOOL_TIMEOUT: u64 = 3600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentKind {
    Claude,
    Codex,
    OpenCode,
}

impl AgentKind {
    pub const ALL: [AgentKind; 3] = [AgentKind::Claude, AgentKind::Codex, AgentKind::OpenCode];

    /// Its program, which is also its `asides` subcommand and its key in `[agents]`.
    pub fn program(self) -> &'static str {
        match self {
            AgentKind::Claude => "claude",
            AgentKind::Codex => "codex",
            AgentKind::OpenCode => "opencode",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            AgentKind::Claude => "Claude Code",
            AgentKind::Codex => "Codex",
            AgentKind::OpenCode => "OpenCode",
        }
    }

    pub fn parse(program: &str) -> Option<AgentKind> {
        AgentKind::ALL.into_iter().find(|k| k.program() == program)
    }

    /// Claude Code runs sessions in the background (`claude --bg`, which `claude attach` opens),
    /// and so does Codex (a thread in its app-server daemon, which `codex resume` opens). OpenCode
    /// would need `opencode run`, which its free tier refuses.
    pub fn runs_in_background(self) -> bool {
        matches!(self, AgentKind::Claude | AgentKind::Codex)
    }

    /// Claude Code makes a git worktree itself when asked; for the others, asides does.
    pub fn makes_worktrees(self) -> bool {
        self == AgentKind::Claude
    }

    /// Asides can hand Codex each aside as a message (`codex queue`), so it doesn't have to wait
    /// for them in a tool call.
    pub fn takes_pushes(self) -> bool {
        self == AgentKind::Codex
    }

    /// Claude Code watches `asides listen` with its Monitor tool; the others wait in a tool call.
    pub fn listens_with_monitor(self) -> bool {
        self == AgentKind::Claude
    }

    pub fn extra_args(self, agents: &Agents) -> &[String] {
        match self {
            AgentKind::Claude => &agents.claude,
            AgentKind::Codex => &agents.codex,
            AgentKind::OpenCode => &agents.opencode,
        }
    }
}

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
    /// Open this Codex thread (`codex resume`), which asides started, instead of a new session.
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
                args.extend(["--worktree".into(), tree.into()]);
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
            server.extend(["--timeout".into(), CODEX_WAIT.to_string()]);
            let set = |key: &str, value: String| ["-c".to_string(), format!("mcp_servers.asides.{key}={value}")];
            args.extend(set("command", toml_string(launch.asides)));
            args.extend(set(
                "args",
                format!("[{}]", server.iter().map(|a| toml_string(a)).collect::<Vec<_>>().join(", ")),
            ));
            args.extend(set("tool_timeout_sec", CODEX_TOOL_TIMEOUT.to_string()));
            args.extend(launch.extra.iter().cloned());
            args.push(launch.prompt.into());
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
    }
    Command { program: launch.kind.program().into(), args, env }
}

/// The configuration a Codex thread asides starts gets: its MCP server, for the session.
pub fn codex_thread_config(asides: &str, repo: &Path, session: &str) -> serde_json::Value {
    let args = ["-C", &repo.to_string_lossy(), "mcp", "--session", session].map(String::from);
    json!({ "mcp_servers": { "asides": { "command": asides, "args": args } } })
}

/// `text` as a TOML string (a JSON string is a valid TOML basic string).
fn toml_string(text: &str) -> String {
    serde_json::Value::from(text).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn the_listen_prompt_says_how_to_listen() {
        let monitor = listen_prompt(&Via::Monitor("asides listen --session s".into()), None);
        assert!(monitor.contains("background monitor") && monitor.contains("\n\nasides listen --session s\n\n"));
        let wait = listen_prompt(&Via::Wait, Some(Path::new("/r/shop")));
        assert!(wait.contains("To listen, call the wait_for_asides tool") && wait.contains("worktree of /r/shop"));
        assert!(wait.ends_with("Keep listening until I tell you to stop."));
    }

    #[test]
    fn names_agents() {
        assert_eq!(AgentKind::parse("codex"), Some(AgentKind::Codex));
        assert_eq!(AgentKind::parse("cursor"), None);
        assert_eq!(AgentKind::OpenCode.name(), "OpenCode");
    }
}
