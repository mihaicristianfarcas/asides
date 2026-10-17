//! The coding agents asides works with, in one place: what each is called and how it's started
//! (`asides <program>`, New Agent Session), how it gets the user's asides (watching `asides listen`
//! in the background, messages asides pushes into it, or waiting in a tool call), whether it runs
//! without a window, how long its MCP client lets a tool call run, how that client introduces
//! itself, and the variables it marks the commands it runs with. `launch.rs` builds each one's
//! command line.
//!
//! Any other agent that speaks MCP works too, through `asides mcp` set up in it by hand: it waits
//! for asides in a tool call, under the name its client gives (see [`client_label`]).

use std::time::Duration;

use crate::launch::{REPO_VAR, SESSION_VAR};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentKind {
    Claude,
    Codex,
    OpenCode,
    Gemini,
    Copilot,
    Qwen,
}

/// What asides knows about an agent it starts.
struct Facts {
    kind: AgentKind,
    /// Its program, which is also its `asides` subcommand and its key in `[agents]`.
    program: &'static str,
    name: &'static str,
    /// A word of the name its MCP client gives (`clientInfo.name`, e.g. `gemini-cli-mcp-client`).
    client: &'static str,
    /// How long its MCP client lets a tool call run, unless the server's configuration says.
    tool_timeout: Duration,
    /// Variables it sets for the commands it runs (and only there): they mark a process as part of
    /// its session.
    markers: &'static [&'static str],
}

const MINUTE: Duration = Duration::from_secs(60);

const FACTS: [Facts; 6] = [
    Facts {
        kind: AgentKind::Claude,
        program: "claude",
        name: "Claude Code",
        client: "claude",
        // Hours, but a stdio call that sends nothing for 30 minutes is cut off.
        tool_timeout: Duration::from_secs(30 * 60),
        markers: &["CLAUDECODE"],
    },
    Facts {
        kind: AgentKind::Codex,
        program: "codex",
        name: "Codex",
        client: "codex",
        tool_timeout: MINUTE,
        markers: &[],
    },
    Facts {
        kind: AgentKind::OpenCode,
        program: "opencode",
        name: "OpenCode",
        client: "opencode",
        tool_timeout: MINUTE,
        markers: &["OPENCODE"],
    },
    Facts {
        kind: AgentKind::Gemini,
        program: "gemini",
        name: "Gemini CLI",
        client: "gemini",
        tool_timeout: Duration::from_secs(10 * 60),
        markers: &["GEMINI_CLI"],
    },
    Facts {
        kind: AgentKind::Copilot,
        program: "copilot",
        name: "Copilot CLI",
        client: "copilot",
        tool_timeout: MINUTE,
        markers: &[],
    },
    Facts {
        kind: AgentKind::Qwen,
        program: "qwen",
        name: "Qwen Code",
        client: "qwen",
        tool_timeout: Duration::from_secs(10 * 60),
        markers: &["QWEN_CODE"],
    },
];

/// MCP clients asides doesn't start, by a word of the name they give, and what to call them.
const OTHER_CLIENTS: [(&str, &str); 9] = [
    ("cursor", "Cursor"),
    ("windsurf", "Windsurf"),
    ("zed", "Zed"),
    ("vscode", "VS Code"),
    ("amp", "Amp"),
    ("goose", "Goose"),
    ("cline", "Cline"),
    ("kiro", "Kiro"),
    ("warp", "Warp"),
];

impl AgentKind {
    pub const ALL: [AgentKind; 6] = [
        AgentKind::Claude,
        AgentKind::Codex,
        AgentKind::OpenCode,
        AgentKind::Gemini,
        AgentKind::Copilot,
        AgentKind::Qwen,
    ];

    fn facts(self) -> &'static Facts {
        FACTS.iter().find(|f| f.kind == self).expect("every agent has its facts")
    }

    /// Its program, which is also its `asides` subcommand and its key in `[agents]`.
    pub fn program(self) -> &'static str {
        self.facts().program
    }

    pub fn name(self) -> &'static str {
        self.facts().name
    }

    pub fn parse(program: &str) -> Option<AgentKind> {
        AgentKind::ALL.into_iter().find(|k| k.program() == program)
    }

    /// The agent a session record names (its [`name`](Self::name)).
    pub fn named(name: &str) -> Option<AgentKind> {
        AgentKind::ALL.into_iter().find(|k| k.name() == name)
    }

    /// The agent whose MCP client introduced itself as `client` (`clientInfo.name`).
    pub fn of_client(client: &str) -> Option<AgentKind> {
        let words = words(client);
        AgentKind::ALL.into_iter().find(|k| words.iter().any(|w| w == k.facts().client))
    }

    /// Claude Code makes a git worktree itself when asked; for the others, asides does.
    pub fn makes_worktrees(self) -> bool {
        self == AgentKind::Claude
    }

    /// Asides can hand Codex and OpenCode each aside as a message (a turn in Codex's daemon, a
    /// prompt to OpenCode's service), so they don't have to wait for them in a tool call.
    pub fn takes_pushes(self) -> bool {
        matches!(self, AgentKind::Codex | AgentKind::OpenCode)
    }

    /// It can run with no window open: Claude Code's background sessions (`claude --bg`), and
    /// the background processes Codex and OpenCode run their sessions in.
    pub fn runs_in_background(self) -> bool {
        matches!(self, AgentKind::Claude | AgentKind::Codex | AgentKind::OpenCode)
    }

    /// Claude Code watches `asides listen` with its Monitor tool; the others wait in a tool call
    /// (or get messages).
    pub fn listens_with_monitor(self) -> bool {
        self == AgentKind::Claude
    }

    /// How long `wait_for_asides` waits before answering that nothing came, for this agent's
    /// client, when the server isn't told: inside its tool calls' time limit, with room to spare.
    pub fn wait(self) -> Duration {
        match self.facts().tool_timeout {
            limit if limit >= Duration::from_secs(30 * 60) => Duration::from_secs(25 * 60),
            limit if limit > MINUTE => limit - MINUTE,
            _ => Duration::from_secs(50),
        }
    }

    /// Whether asides raises its client's tool-call time limit for a session it starts, so that
    /// `wait_for_asides` waits long (Claude Code needs it raised, and the others wait in messages).
    pub fn waits_long_when_started(self) -> bool {
        matches!(self, AgentKind::Codex | AgentKind::Gemini | AgentKind::Copilot | AgentKind::Qwen)
    }
}

/// The lowercase words of a client's name.
fn words(name: &str) -> Vec<String> {
    name.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}

/// A friendlier name for an MCP client: `claude-code` → "Claude Code", `gemini-cli-mcp-client` →
/// "Gemini CLI", `Visual Studio Code` → "VS Code".
pub fn client_label(name: &str) -> String {
    if let Some(kind) = AgentKind::of_client(name) {
        return kind.name().into();
    }
    let lower = name.to_ascii_lowercase();
    let words = words(name);
    if lower.contains("visual studio code") {
        return "VS Code".into();
    }
    if let Some((_, label)) = OTHER_CLIENTS.iter().find(|(word, _)| words.iter().any(|w| w == word)) {
        return (*label).into();
    }
    match name.trim() {
        "" => "Agent".into(),
        name => name.to_string(),
    }
}

/// Which of the variables `names` mark a process as part of an agent's session: those Claude Code
/// (`CLAUDECODE`, `CLAUDE_CODE_SESSION_ID`, its pid and effort…), OpenCode (`OPENCODE`, `AGENT`…),
/// Gemini CLI (`GEMINI_CLI`) and Qwen Code (`QWEN_CODE`) set for the commands they run, and asides'
/// own. An agent's process can start the app (its `asides mcp` opens a link), which would hand them
/// to every terminal and agent the app starts: Claude Code took a session started from the menu for
/// a child of the first one (and turned off saving its transcript), and `asides` in that terminal
/// meant the first session's repo. An agent's settings (`CLAUDE_CODE_USE_BEDROCK`) count only
/// inside its session, where they're set again: from its settings, or the login shell.
pub fn session_markers(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let names: Vec<String> = names.into_iter().collect();
    let claude = names.iter().any(|n| n == "CLAUDECODE");
    let opencode = names.iter().any(|n| n == "OPENCODE");
    let marker = |name: &str| FACTS.iter().any(|f| f.markers.contains(&name));
    names
        .into_iter()
        .filter(|name| match name.as_str() {
            SESSION_VAR | REPO_VAR | "AI_AGENT" => true,
            n if marker(n) => true,
            "CLAUDE_PID" | "CLAUDE_EFFORT" | "CLAUDE_PROJECT_DIR" => claude,
            n if n.starts_with("CLAUDE_CODE_") => claude,
            "AGENT" | "OPENCODE_SESSION_ID" | "OPENCODE_TERMINAL" | "OPENCODE_PTY_HANDOFF" => opencode,
            _ => false,
        })
        .collect()
}

/// [`session_markers`] of this process's environment.
pub fn inherited_markers() -> Vec<String> {
    session_markers(std::env::vars_os().filter_map(|(name, _)| name.into_string().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_agents() {
        assert_eq!(AgentKind::parse("codex"), Some(AgentKind::Codex));
        assert_eq!(AgentKind::parse("gemini"), Some(AgentKind::Gemini));
        assert_eq!(AgentKind::parse("cursor"), None);
        assert_eq!(AgentKind::OpenCode.name(), "OpenCode");
        assert_eq!(AgentKind::named("Copilot CLI"), Some(AgentKind::Copilot));
        assert!(AgentKind::OpenCode.takes_pushes() && !AgentKind::Claude.takes_pushes());
        assert!(AgentKind::Codex.runs_in_background() && !AgentKind::Qwen.runs_in_background());
        // Every agent's facts are there, once.
        for kind in AgentKind::ALL {
            assert_eq!(FACTS.iter().filter(|f| f.kind == kind).count(), 1, "{kind:?}");
            assert_eq!(AgentKind::parse(kind.program()), Some(kind));
        }
    }

    #[test]
    fn labels_clients() {
        assert_eq!(client_label("claude-code"), "Claude Code");
        assert_eq!(client_label("codex-mcp-client"), "Codex");
        assert_eq!(client_label("opencode"), "OpenCode");
        assert_eq!(client_label("gemini-cli-mcp-client"), "Gemini CLI");
        assert_eq!(client_label("copilot-cli"), "Copilot CLI");
        assert_eq!(client_label("qwen-code"), "Qwen Code");
        assert_eq!(client_label("cursor-vscode"), "Cursor");
        assert_eq!(client_label("Visual Studio Code"), "VS Code");
        assert_eq!(client_label("example-client"), "example-client", "no word is amp");
        assert_eq!(client_label(""), "Agent");
        assert_eq!(client_label("Zed"), "Zed");
    }

    #[test]
    fn waits_inside_each_clients_time_limit() {
        assert_eq!(AgentKind::Claude.wait(), Duration::from_secs(25 * 60));
        assert_eq!(AgentKind::Gemini.wait(), Duration::from_secs(9 * 60));
        assert_eq!(AgentKind::Codex.wait(), Duration::from_secs(50));
        assert_eq!(AgentKind::of_client("qwen-cli-mcp-client"), Some(AgentKind::Qwen));
    }

    #[test]
    fn finds_the_markers_of_an_agents_session() {
        let inside = [
            "PATH",
            "CLAUDECODE",
            "CLAUDE_CODE_SESSION_ID",
            "CLAUDE_CODE_CHILD_SESSION",
            "CLAUDE_EFFORT",
            "AI_AGENT",
            "ASIDES_REPO",
            "OPENCODE",
            "AGENT",
            "OPENCODE_SESSION_ID",
            "OPENCODE_PASSWORD",
            "CODEX_HOME",
            "GEMINI_CLI",
            "QWEN_CODE",
        ];
        assert_eq!(
            session_markers(inside.map(String::from)),
            [
                "CLAUDECODE",
                "CLAUDE_CODE_SESSION_ID",
                "CLAUDE_CODE_CHILD_SESSION",
                "CLAUDE_EFFORT",
                "AI_AGENT",
                "ASIDES_REPO",
                "OPENCODE",
                "AGENT",
                "OPENCODE_SESSION_ID",
                "GEMINI_CLI",
                "QWEN_CODE"
            ]
        );
        // Outside an agent, settings with the same prefixes stay.
        let outside = ["PATH", "CLAUDE_CODE_USE_BEDROCK", "CLAUDE_EFFORT", "AGENT", "OPENCODE_SESSION_ID"];
        assert!(session_markers(outside.map(String::from)).is_empty());
    }
}
