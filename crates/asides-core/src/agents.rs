//! The coding agents asides sends asides to, in one place: what each is called, how its process is
//! recognized (`sessions.rs` finds them in the process table), how it's started in a new session,
//! which key sends a message typed into it, and the variables it marks the commands it runs with.

/// The variable a launch script gives the agent it starts, so the session can be told apart.
pub const LAUNCH_VAR: &str = "ASIDES_LAUNCH";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    Claude,
    Codex,
    OpenCode,
    Gemini,
    Copilot,
    Qwen,
}

/// What asides knows about an agent.
struct Facts {
    kind: AgentKind,
    /// Its program, which is also its key in `[agents]`.
    program: &'static str,
    name: &'static str,
    /// Its process: the executable's name, or for agents run by Node, a word of its script's path.
    process: Process,
    /// Variables it sets for the commands it runs (and only there): they mark a process as part of
    /// its session.
    markers: &'static [&'static str],
}

enum Process {
    /// A native executable with this name.
    Executable(&'static str),
    /// `node …/<word>…`: a Node script whose path has this word.
    Node(&'static str),
}

const FACTS: [Facts; 6] = [
    Facts {
        kind: AgentKind::Claude,
        program: "claude",
        name: "Claude Code",
        process: Process::Executable("claude"),
        markers: &["CLAUDECODE"],
    },
    Facts {
        kind: AgentKind::Codex,
        program: "codex",
        name: "Codex",
        process: Process::Executable("codex"),
        markers: &[],
    },
    Facts {
        kind: AgentKind::OpenCode,
        program: "opencode",
        name: "OpenCode",
        process: Process::Executable("opencode"),
        markers: &["OPENCODE"],
    },
    Facts {
        kind: AgentKind::Gemini,
        program: "gemini",
        name: "Gemini CLI",
        process: Process::Node("gemini"),
        markers: &["GEMINI_CLI"],
    },
    Facts {
        kind: AgentKind::Copilot,
        program: "copilot",
        name: "Copilot CLI",
        process: Process::Node("copilot"),
        markers: &[],
    },
    Facts {
        kind: AgentKind::Qwen,
        program: "qwen",
        name: "Qwen Code",
        process: Process::Node("qwen"),
        markers: &["QWEN_CODE"],
    },
];

/// The key that sends what's typed into an agent's prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitKey {
    Enter,
    /// Codex: Return while it's working joins the turn in progress ("steers"), and it may then
    /// answer only the newer message; Tab queues it for after the turn (and sends it at once when
    /// Codex is idle).
    Tab,
}

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

    /// Its program, which is also its key in `[agents]`.
    pub fn program(self) -> &'static str {
        self.facts().program
    }

    pub fn name(self) -> &'static str {
        self.facts().name
    }

    pub fn parse(program: &str) -> Option<AgentKind> {
        AgentKind::ALL.into_iter().find(|k| k.program() == program)
    }

    /// The agent a process is, from its executable's name (`comm`) and its arguments: `claude`, or
    /// `node /opt/homebrew/bin/gemini`.
    pub fn of_process(executable: &str, args: &[&str]) -> Option<AgentKind> {
        let base = |path: &str| path.rsplit('/').next().unwrap_or(path).to_string();
        let executable = base(executable);
        AgentKind::ALL.into_iter().find(|k| match k.facts().process {
            Process::Executable(name) => executable == name,
            Process::Node(word) => {
                executable == "node" && args.get(1).is_some_and(|script| script.to_ascii_lowercase().contains(word))
            }
        })
    }

    pub fn submit_key(self) -> SubmitKey {
        if self == AgentKind::Codex { SubmitKey::Tab } else { SubmitKey::Enter }
    }

    /// Its command line for a new session in a terminal, starting with `prompt` as its first
    /// message, plus the user's extra arguments (`[agents]`). Words are for `sh`.
    pub fn command(self, prompt: &str, extra: &[String]) -> Vec<String> {
        let mut words = vec![self.program().to_string()];
        words.extend(extra.iter().cloned());
        match self {
            // `--prompt` starts the TUI with it; a bare argument would be a project directory.
            AgentKind::OpenCode => words.extend(["--prompt".into(), prompt.into()]),
            // `-i` runs it, then stays interactive.
            AgentKind::Gemini | AgentKind::Qwen | AgentKind::Copilot => words.extend(["-i".into(), prompt.into()]),
            AgentKind::Claude | AgentKind::Codex => words.push(prompt.into()),
        }
        words
    }
}

/// Which of the variables `names` mark a process as part of an agent's session (or of another app
/// hosting one): those Claude Code (`CLAUDECODE`, `CLAUDE_CODE_SESSION_ID`, its pid and effort…),
/// OpenCode (`OPENCODE`, `AGENT`…), Gemini CLI (`GEMINI_CLI`), Qwen Code (`QWEN_CODE`) and Diri
/// (`DIRIJOR_SESSION_ID`…) set for the commands they run, and asides' own. An agent's process can
/// start the app, which would hand them to every terminal and agent the app starts: Claude Code
/// took a session started that way for a child of the first one (and turned off saving its
/// transcript), and a session started in Terminal looked like Diri's. An agent's settings
/// (`CLAUDE_CODE_USE_BEDROCK`) count only inside its session, where they're set again: from its
/// settings, or the login shell.
pub fn session_markers(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let names: Vec<String> = names.into_iter().collect();
    let claude = names.iter().any(|n| n == "CLAUDECODE");
    let opencode = names.iter().any(|n| n == "OPENCODE");
    let marker = |name: &str| FACTS.iter().any(|f| f.markers.contains(&name));
    names
        .into_iter()
        .filter(|name| match name.as_str() {
            LAUNCH_VAR | "AI_AGENT" => true,
            n if marker(n) => true,
            n if n.starts_with("DIRIJOR_") => true,
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
        // Every agent's facts are there, once.
        for kind in AgentKind::ALL {
            assert_eq!(FACTS.iter().filter(|f| f.kind == kind).count(), 1, "{kind:?}");
            assert_eq!(AgentKind::parse(kind.program()), Some(kind));
        }
    }

    #[test]
    fn recognizes_agent_processes() {
        assert_eq!(AgentKind::of_process("claude", &["claude"]), Some(AgentKind::Claude));
        assert_eq!(
            AgentKind::of_process("/opt/homebrew/Caskroom/codex/0.163.0/bin/codex", &["codex", "resume"]),
            Some(AgentKind::Codex)
        );
        assert_eq!(AgentKind::of_process("opencode", &[]), Some(AgentKind::OpenCode));
        assert_eq!(
            AgentKind::of_process("node", &["node", "/opt/homebrew/lib/node_modules/@google/gemini-cli/dist/index.js"]),
            Some(AgentKind::Gemini)
        );
        assert_eq!(AgentKind::of_process("node", &["node", "server.js"]), None);
        assert_eq!(AgentKind::of_process("zsh", &["-zsh"]), None);
    }

    #[test]
    fn codex_queues_with_tab() {
        assert_eq!(AgentKind::Codex.submit_key(), SubmitKey::Tab);
        assert_eq!(AgentKind::Claude.submit_key(), SubmitKey::Enter);
    }

    #[test]
    fn builds_command_lines_with_the_first_message() {
        assert_eq!(
            AgentKind::Claude.command("Fix it", &["--model".into(), "opus".into()]),
            ["claude", "--model", "opus", "Fix it"]
        );
        assert_eq!(AgentKind::OpenCode.command("Fix it", &[]), ["opencode", "--prompt", "Fix it"]);
        assert_eq!(AgentKind::Gemini.command("Fix it", &[]), ["gemini", "-i", "Fix it"]);
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
            "ASIDES_LAUNCH",
            "OPENCODE",
            "AGENT",
            "OPENCODE_SESSION_ID",
            "OPENCODE_PASSWORD",
            "CODEX_HOME",
            "GEMINI_CLI",
            "QWEN_CODE",
            "DIRIJOR_SESSION_ID",
        ];
        assert_eq!(
            session_markers(inside.map(String::from)),
            [
                "CLAUDECODE",
                "CLAUDE_CODE_SESSION_ID",
                "CLAUDE_CODE_CHILD_SESSION",
                "CLAUDE_EFFORT",
                "AI_AGENT",
                "ASIDES_LAUNCH",
                "OPENCODE",
                "AGENT",
                "OPENCODE_SESSION_ID",
                "GEMINI_CLI",
                "QWEN_CODE",
                "DIRIJOR_SESSION_ID"
            ]
        );
        // Outside an agent, settings with the same prefixes stay.
        let outside = ["PATH", "CLAUDE_CODE_USE_BEDROCK", "CLAUDE_EFFORT", "AGENT", "OPENCODE_SESSION_ID"];
        assert!(session_markers(outside.map(String::from)).is_empty());
    }
}
