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
    Cursor,
    Droid,
    Amp,
    Kiro,
    Goose,
    Crush,
    Cline,
    Kilo,
    Auggie,
    Pi,
    Continue,
    IFlow,
    Vibe,
    Kimi,
    Letta,
    Aider,
}

/// What asides knows about an agent. Checked on 2026-11-18 against each one's own build (npm's,
/// or its installer's): how its process looks while it runs in a terminal, its `--help`.
struct Facts {
    kind: AgentKind,
    /// Its program, which is also its key in `[agents]`.
    program: &'static str,
    name: &'static str,
    /// How its process looks: each way it can be installed.
    process: &'static [Process],
    /// Variables it sets for the commands it runs (and only there): they mark a process as part of
    /// its session.
    markers: &'static [&'static str],
    /// Options that make its process a one-off run (`-p`), a server or anything but a session.
    not_flags: &'static [&'static str],
    /// Its commands that aren't sessions (`codex exec`, `claude mcp`), its first word that isn't an
    /// option.
    not_commands: &'static [&'static str],
    /// The arguments before a new session's first message, for an agent that starts with one and
    /// stays: `--` where its parser takes it, so a message starting with `-` stays a message.
    /// `None`: it can't, and isn't offered for new sessions.
    first_message: Option<&'static [&'static str]>,
}

enum Process {
    /// A native executable with this name, by its path or its `argv[0]` (or the title it gives
    /// itself, which `ps` shows there), or a version of it kept as `…/<name>/versions/<version>`
    /// (Claude Code's and Cursor's own installs: started by that path, its process isn't named so).
    Executable(&'static str),
    /// A script run by Node, Bun or Python whose path has this word (npm's and uv's installs), or
    /// named as the agent's program (`node /opt/homebrew/bin/cn`).
    Script(&'static str),
    /// Another name it's started by (Cursor's `agent`), its own only with its files on the command
    /// line: what's named so isn't, otherwise.
    Alias { name: &'static str, path: &'static str },
}

impl Process {
    fn matches(&self, program: &str, executable: &str, args: &[&str]) -> bool {
        let first = args.first().map(|a| program_name(a));
        match *self {
            Process::Executable(name) => {
                program_name(executable) == name
                    || first.as_deref() == Some(name)
                    || executable.replace('\\', "/").contains(&format!("/{name}/versions/"))
            }
            Process::Script(word) => {
                (is_runtime(executable) || first.as_deref().is_some_and(is_runtime))
                    && args.iter().skip(1).find(|a| **a != "-m" && !a.starts_with("--")).is_some_and(|script| {
                        let script = script.to_ascii_lowercase();
                        script.contains(word) || script_name(&script) == program
                    })
            }
            Process::Alias { name, path } => {
                (program_name(executable) == name || first.as_deref() == Some(name))
                    && args.iter().take(4).any(|a| a.replace('\\', "/").contains(path))
            }
        }
    }

    /// Whether a process named `executable` may be this, before its arguments are read.
    fn may_be(&self, executable: &str) -> bool {
        match *self {
            Process::Executable(name) | Process::Alias { name, .. } => program_name(executable) == name,
            Process::Script(_) => is_runtime(executable),
        }
    }
}

/// A program's name from its path, without Windows' `.exe`: `/opt/homebrew/bin/node` and
/// `C:\Program Files\nodejs\node.exe` are both `node`.
fn program_name(path: &str) -> String {
    let base = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match base.len().checked_sub(4).filter(|&at| base.is_char_boundary(at)) {
        Some(at) if base[at..].eq_ignore_ascii_case(".exe") => base[..at].to_string(),
        _ => base.to_string(),
    }
}

/// A script's name without its extension: `/x/dist/cn.js` is `cn`.
fn script_name(path: &str) -> &str {
    let base = path.rsplit(['/', '\\']).next().unwrap_or(path);
    [".js", ".mjs", ".cjs", ".py"].iter().find_map(|ext| base.strip_suffix(ext)).unwrap_or(base)
}

/// Whether `executable` runs scripts (Node, Bun, Python): agents installed with npm or uv run in
/// one.
pub fn is_runtime(executable: &str) -> bool {
    let name = program_name(executable).to_ascii_lowercase();
    let python = name.strip_prefix("python").is_some_and(|v| v.chars().all(|c| c.is_ascii_digit() || c == '.'));
    matches!(name.as_str(), "node" | "bun") || python
}

/// Options that make a process a one-off run, for agents whose `-p` prints an answer and ends.
const PRINT: &[&str] = &["-p", "--print", "--prompt"];

const FACTS: [Facts; 22] = [
    Facts {
        kind: AgentKind::Claude,
        program: "claude",
        name: "Claude Code",
        process: &[Process::Executable("claude"), Process::Script("claude-code")],
        markers: &["CLAUDECODE"],
        // Driven by another program (an editor's extension) through its standard input, too.
        not_flags: &["-p", "--print", "--input-format", "--output-format"],
        not_commands: &["agents", "mcp", "doctor", "logs", "stop", "rm", "update"],
        // After `--`: an option taking several values before it (`--add-dir`) took the prompt for
        // one more, and the session started empty.
        first_message: Some(&["--"]),
    },
    Facts {
        kind: AgentKind::Codex,
        program: "codex",
        name: "Codex",
        process: &[Process::Executable("codex"), Process::Script("codex")],
        markers: &[],
        not_flags: &[],
        not_commands: &["exec", "e", "app-server", "mcp", "mcp-server", "login", "logout", "proto", "apply"],
        // `--image` takes several values, as Claude Code's `--add-dir`.
        first_message: Some(&["--"]),
    },
    Facts {
        kind: AgentKind::OpenCode,
        program: "opencode",
        name: "OpenCode",
        process: &[Process::Executable("opencode"), Process::Script("opencode")],
        markers: &["OPENCODE"],
        not_flags: &[],
        not_commands: &["run", "serve", "mcp", "debug", "session", "auth", "models", "export"],
        // `--prompt` starts the TUI with it; a bare argument would be a project directory.
        first_message: Some(&["--prompt"]),
    },
    Facts {
        kind: AgentKind::Gemini,
        program: "gemini",
        name: "Gemini CLI",
        process: &[Process::Script("gemini")],
        markers: &["GEMINI_CLI"],
        not_flags: PRINT,
        not_commands: &["mcp", "extensions", "skills", "hooks"],
        // `-i` runs it, then stays interactive.
        first_message: Some(&["-i"]),
    },
    Facts {
        kind: AgentKind::Copilot,
        program: "copilot",
        name: "Copilot CLI",
        // npm's loader runs the program its package ships: Homebrew's and the installer's are that.
        process: &[Process::Executable("copilot"), Process::Script("copilot")],
        markers: &[],
        not_flags: &["-p", "--prompt", "--acp"],
        not_commands: &[
            "app",
            "login",
            "help",
            "init",
            "config",
            "update",
            "version",
            "workflow",
            "sessions",
            "memories",
            "plugin",
            "mcp",
            "skill",
            "instruction",
            "lsp",
            "sandbox",
            "completion",
        ],
        first_message: Some(&["-i"]),
    },
    Facts {
        kind: AgentKind::Qwen,
        program: "qwen",
        name: "Qwen Code",
        process: &[Process::Script("qwen")],
        markers: &["QWEN_CODE"],
        not_flags: PRINT,
        not_commands: &["mcp", "extensions"],
        first_message: Some(&["-i"]),
    },
    Facts {
        kind: AgentKind::Cursor,
        program: "cursor-agent",
        name: "Cursor CLI",
        // Its installer's `node` runs it, named as it was started: `cursor-agent`, or `agent`.
        process: &[
            Process::Executable("cursor-agent"),
            Process::Alias { name: "agent", path: "/cursor-agent/" },
            Process::Script("cursor-agent"),
        ],
        markers: &[],
        not_flags: &["-p", "--print", "--list-models"],
        not_commands: &[
            "persist",
            "install-shell-integration",
            "uninstall-shell-integration",
            "login",
            "logout",
            "mcp",
            "plugin",
            "worker",
            "status",
            "whoami",
            "models",
            "team",
            "bedrock",
            "about",
            "update",
            "create-chat",
            "generate-rule",
            "rule",
        ],
        first_message: Some(&["--"]),
    },
    Facts {
        kind: AgentKind::Droid,
        program: "droid",
        name: "Droid",
        process: &[Process::Executable("droid")],
        markers: &[],
        not_flags: &[],
        not_commands: &["exec", "daemon", "search", "find", "update", "mcp", "plugin", "computer", "doctor"],
        first_message: Some(&["--"]),
    },
    Facts {
        kind: AgentKind::Amp,
        program: "amp",
        name: "Amp",
        process: &[Process::Executable("amp")],
        markers: &[],
        not_flags: &["-x", "--execute", "-ox", "--orb-execute", "--no-tui"],
        // It takes no prompt: any command isn't a session (`amp login`, `amp threads`…).
        not_commands: &[],
        first_message: None,
    },
    Facts {
        kind: AgentKind::Kiro,
        program: "kiro-cli",
        name: "Kiro CLI",
        process: &[Process::Executable("kiro-cli"), Process::Executable("kiro-cli-chat")],
        markers: &[],
        not_flags: &["--no-interactive"],
        not_commands: &[
            "login",
            "logout",
            "whoami",
            "profile",
            "settings",
            "setup",
            "update",
            "diagnostic",
            "issue",
            "mcp",
            "translate",
            "doctor",
            "agent",
            "inline",
            "integrations",
            "quit",
            "restart",
            "launch",
            "dashboard",
            "acp",
            "user",
            "help",
        ],
        first_message: Some(&["chat", "--"]),
    },
    Facts {
        kind: AgentKind::Goose,
        program: "goose",
        name: "goose",
        process: &[Process::Executable("goose")],
        markers: &[],
        not_flags: &[],
        // `goose run` too, unless it stays (`-s`): `AgentKind::is_session`.
        not_commands: &[
            "configure",
            "info",
            "doctor",
            "mcp",
            "acp",
            "serve",
            "recipe",
            "skills",
            "plugin",
            "schedule",
            "sched",
            "gateway",
            "gw",
            "update",
            "local-models",
            "lm",
            "completion",
            "help",
            "web",
            "bench",
            // The database migration tool named `goose` too: its commands, and the databases it's given.
            "up",
            "down",
            "status",
            "create",
            "redo",
            "reset",
            "version",
            "fix",
            "validate",
            "postgres",
            "mysql",
            "sqlite3",
            "mssql",
            "redshift",
            "tidb",
            "clickhouse",
            "vertica",
            "ydb",
            "turso",
            "starrocks",
        ],
        // `run -s` stays after its first message; `--text=` keeps one starting with `-` its text.
        first_message: Some(&["run", "-s"]),
    },
    Facts {
        kind: AgentKind::Crush,
        program: "crush",
        name: "Crush",
        process: &[Process::Executable("crush"), Process::Script("crush")],
        markers: &[],
        not_flags: &[],
        // It takes no prompt: any command isn't a session (`crush run`, `crush logs`…).
        not_commands: &[],
        first_message: None,
    },
    Facts {
        kind: AgentKind::Cline,
        program: "cline",
        name: "Cline",
        process: &[Process::Executable("cline")],
        markers: &[],
        // A prompt without `-i` runs on its own and ends: `AgentKind::is_session`.
        not_flags: &["--json", "--acp", "-z", "--zen"],
        not_commands: &[
            "auth", "config", "plugin", "skill", "connect", "mcp", "doctor", "history", "h", "hook", "schedule", "hub",
            "update", "version", "kanban",
        ],
        first_message: None,
    },
    Facts {
        kind: AgentKind::Kilo,
        program: "kilo",
        name: "Kilo Code",
        // OpenCode's own, renamed: the same interface, and the same ways in.
        process: &[Process::Executable("kilo"), Process::Executable("kilocode"), Process::Script("kilocode")],
        markers: &[],
        not_flags: &[],
        not_commands: &[
            "completion",
            "acp",
            "mcp",
            "attach",
            "run",
            "debug",
            "auth",
            "providers",
            "agent",
            "upgrade",
            "uninstall",
            "serve",
            "models",
            "stats",
            "export",
            "import",
            "github",
            "pr",
            "session",
            "plugin",
            "plug",
            "db",
            "console",
            "cloud",
            "roll-call",
            "profile",
            "auto",
            "remote",
            "daemon",
            "config",
            "worktree",
            "help",
        ],
        first_message: Some(&["--prompt"]),
    },
    Facts {
        kind: AgentKind::Auggie,
        program: "auggie",
        name: "Auggie",
        process: &[Process::Executable("auggie"), Process::Script("auggie")],
        markers: &[],
        not_flags: &["-p", "--print", "--acp", "--mode"],
        not_commands: &[
            "install",
            "remove",
            "uninstall",
            "update",
            "upgrade",
            "list",
            "config",
            "command",
            "commands",
            "cloud",
            "daemon",
            "session",
            "mcp",
            "auth",
            "token",
            "tokens",
            "model",
        ],
        first_message: Some(&[]),
    },
    Facts {
        kind: AgentKind::Pi,
        program: "pi",
        name: "Pi",
        // It names its process `pi`.
        process: &[Process::Executable("pi"), Process::Script("pi-coding-agent")],
        markers: &[],
        not_flags: &["-p", "--print", "--mode", "--export"],
        not_commands: &["install", "remove", "uninstall", "update", "list", "config"],
        first_message: Some(&[]),
    },
    Facts {
        kind: AgentKind::Continue,
        program: "cn",
        name: "Continue CLI",
        process: &[Process::Script("@continuedev")],
        markers: &[],
        not_flags: &["-p", "--print"],
        not_commands: &["ls", "checks", "review", "login", "logout", "serve", "remote", "update"],
        first_message: Some(&["--"]),
    },
    Facts {
        kind: AgentKind::IFlow,
        program: "iflow",
        name: "iFlow CLI",
        process: &[Process::Script("iflow")],
        markers: &[],
        not_flags: &["-p", "--prompt", "--experimental-acp"],
        not_commands: &["mcp", "commands", "agent", "workflow", "skill", "plugin"],
        first_message: Some(&["-i"]),
    },
    Facts {
        kind: AgentKind::Vibe,
        program: "vibe",
        name: "Mistral Vibe",
        // uv's script hands over to the program it ships (`vibe-rs`).
        process: &[Process::Executable("vibe-rs"), Process::Executable("vibe"), Process::Script("mistral-vibe")],
        markers: &[],
        not_flags: &["-p", "--prompt", "--prompt-file", "--setup", "-v", "--version"],
        not_commands: &["mcp", "config", "help"],
        first_message: Some(&["--"]),
    },
    Facts {
        kind: AgentKind::Kimi,
        program: "kimi",
        name: "Kimi Code",
        // It names its process `kimi-code`.
        process: &[Process::Executable("kimi-code"), Process::Executable("kimi"), Process::Script("kimi-code")],
        markers: &[],
        not_flags: &["-p", "--prompt", "-V", "--version"],
        // It takes no prompt: any command isn't a session (`kimi export`, `kimi acp`…).
        not_commands: &[],
        first_message: None,
    },
    Facts {
        kind: AgentKind::Letta,
        program: "letta",
        name: "Letta Code",
        process: &[Process::Script("letta-code")],
        markers: &[],
        not_flags: &["-p", "--info", "-v", "--version"],
        not_commands: &[],
        first_message: None,
    },
    Facts {
        kind: AgentKind::Aider,
        program: "aider",
        name: "Aider",
        process: &[Process::Script("aider")],
        markers: &[],
        // A message runs once and ends; its other words are files to add.
        not_flags: &[
            "-m",
            "--message",
            "--msg",
            "-f",
            "--message-file",
            "--commit",
            "--lint",
            "--test",
            "--apply",
            "--exit",
            "--show-repo-map",
            "--show-prompts",
            "--list-models",
            "--models",
            "--version",
            "--help",
        ],
        not_commands: &[],
        first_message: None,
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
    pub const ALL: [AgentKind; 22] = [
        AgentKind::Claude,
        AgentKind::Codex,
        AgentKind::OpenCode,
        AgentKind::Gemini,
        AgentKind::Copilot,
        AgentKind::Qwen,
        AgentKind::Cursor,
        AgentKind::Droid,
        AgentKind::Amp,
        AgentKind::Kiro,
        AgentKind::Goose,
        AgentKind::Crush,
        AgentKind::Cline,
        AgentKind::Kilo,
        AgentKind::Auggie,
        AgentKind::Pi,
        AgentKind::Continue,
        AgentKind::IFlow,
        AgentKind::Vibe,
        AgentKind::Kimi,
        AgentKind::Letta,
        AgentKind::Aider,
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

    /// The agent a process is, from its executable (`comm`, a path or a name) and its arguments:
    /// `claude`, `…/claude/versions/2.1.303`, or `node /opt/homebrew/bin/gemini`.
    pub fn of_process(executable: &str, args: &[&str]) -> Option<AgentKind> {
        AgentKind::ALL.into_iter().find(|k| {
            let facts = k.facts();
            facts.process.iter().any(|p| p.matches(facts.program, executable, args))
        })
    }

    /// Whether a process named `executable` may be an agent, before its arguments are read (they
    /// are then, to tell): a runtime running a script, or a program named as one.
    pub fn may_be(executable: &str) -> bool {
        FACTS.iter().any(|f| f.process.iter().any(|p| p.may_be(executable)))
    }

    /// Whether its process with these arguments (its program first) is an interactive session, not
    /// a one-off run, a server or a subcommand.
    pub fn is_session(self, args: &[String]) -> bool {
        let facts = self.facts();
        let words: Vec<&str> = args.iter().skip(1).map(String::as_str).collect();
        let has = |flags: &[&str]| {
            words.iter().any(|w| {
                flags
                    .iter()
                    .any(|f| w == f || (f.starts_with("--") && w.strip_prefix(f).is_some_and(|r| r.starts_with('='))))
            })
        };
        let first = words.iter().find(|w| !w.starts_with('-')).copied();
        let command = |not: &[&str]| first.is_some_and(|f| not.contains(&f));
        if has(facts.not_flags) || command(facts.not_commands) {
            return false;
        }
        match self {
            // `goose run` ends after its instructions, unless `-s` keeps it.
            AgentKind::Goose => first != Some("run") || has(&["-s", "--interactive"]),
            // A prompt alone runs without its interface, and ends.
            AgentKind::Cline => first.is_none() || has(&["-i", "--tui"]),
            // They take no prompt: any word is a command.
            AgentKind::Amp | AgentKind::Crush | AgentKind::Kimi | AgentKind::Letta => first.is_none(),
            _ => true,
        }
    }

    pub fn submit_key(self) -> SubmitKey {
        if self == AgentKind::Codex { SubmitKey::Tab } else { SubmitKey::Enter }
    }

    /// Whether a new session of it can start with an aside as its first message.
    pub fn launches(self) -> bool {
        self.facts().first_message.is_some()
    }

    /// Its command line for a new session in a terminal, starting with `prompt` as its first
    /// message, plus the user's extra arguments (`[agents]`): `None` for an agent that can't start
    /// with one. Words are for `sh`.
    pub fn command(self, prompt: &str, extra: &[String]) -> Option<Vec<String>> {
        let before = self.facts().first_message?;
        let mut words = vec![self.program().to_string()];
        words.extend(extra.iter().cloned());
        words.extend(before.iter().map(|w| w.to_string()));
        words.push(if self == AgentKind::Goose { format!("--text={prompt}") } else { prompt.into() });
        Some(words)
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
        assert_eq!(AgentKind::parse("cursor-agent"), Some(AgentKind::Cursor));
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
        // Claude Code started by its versioned path, as when it starts itself again.
        let versioned = "/Users/me/.local/share/claude/versions/2.1.303";
        assert_eq!(AgentKind::of_process(versioned, &[]), Some(AgentKind::Claude));
        assert_eq!(AgentKind::of_process("2.1.303", &["claude", "--resume"]), Some(AgentKind::Claude));
        // npm's installs, under Node or Bun.
        let npm = "/Users/me/.npm-global/lib/node_modules/@anthropic-ai/claude-code/cli.js";
        assert_eq!(AgentKind::of_process("node", &["node", npm]), Some(AgentKind::Claude));
        assert_eq!(
            AgentKind::of_process("/opt/homebrew/bin/bun", &["bun", "/x/node_modules/opencode-ai/bin/opencode"]),
            Some(AgentKind::OpenCode)
        );
        assert_eq!(
            AgentKind::of_process("node", &["node", "/opt/homebrew/lib/node_modules/@openai/codex/bin/codex.js"]),
            Some(AgentKind::Codex)
        );
        assert!(is_runtime("/opt/homebrew/bin/node") && is_runtime("bun") && !is_runtime("claude"));
        assert!(is_runtime(r"C:\Program Files\nodejs\node.exe") && !is_runtime(r"C:\x\claude.exe"));
    }

    #[test]
    fn recognizes_agent_processes_on_windows() {
        let claude = r"C:\Users\me\.local\bin\claude.exe";
        assert_eq!(AgentKind::of_process(claude, &[claude]), Some(AgentKind::Claude));
        let versioned = r"C:\Users\me\.local\share\claude\versions\2.1.303.exe";
        assert_eq!(AgentKind::of_process(versioned, &[]), Some(AgentKind::Claude));
        let npm = r"C:\Users\me\AppData\Roaming\npm\node_modules\@anthropic-ai\claude-code\cli.js";
        assert_eq!(AgentKind::of_process(r"C:\Program Files\nodejs\node.exe", &["node", npm]), Some(AgentKind::Claude));
        assert_eq!(AgentKind::of_process("Codex.EXE", &[]), None);
        assert_eq!(AgentKind::of_process("codex.EXE", &[]), Some(AgentKind::Codex));
        assert_eq!(AgentKind::of_process(r"C:\x\pwsh.exe", &[]), None);
    }

    #[test]
    fn codex_queues_with_tab() {
        assert_eq!(AgentKind::Codex.submit_key(), SubmitKey::Tab);
        assert_eq!(AgentKind::Claude.submit_key(), SubmitKey::Enter);
    }

    #[test]
    fn builds_command_lines_with_the_first_message() {
        let words = |kind: AgentKind, prompt: &str| kind.command(prompt, &[]).unwrap();
        assert_eq!(
            AgentKind::Claude.command("Fix it", &["--model".into(), "opus".into()]).unwrap(),
            ["claude", "--model", "opus", "--", "Fix it"]
        );
        assert_eq!(words(AgentKind::Codex, "-v is wrong"), ["codex", "--", "-v is wrong"]);
        assert_eq!(words(AgentKind::OpenCode, "Fix it"), ["opencode", "--prompt", "Fix it"]);
        assert_eq!(words(AgentKind::Gemini, "Fix it"), ["gemini", "-i", "Fix it"]);
        assert_eq!(words(AgentKind::Kiro, "Fix it"), ["kiro-cli", "chat", "--", "Fix it"]);
        assert_eq!(words(AgentKind::Goose, "-v is wrong"), ["goose", "run", "-s", "--text=-v is wrong"]);
        assert_eq!(words(AgentKind::Pi, "Fix it"), ["pi", "Fix it"]);
        assert_eq!(words(AgentKind::Kilo, "Fix it"), ["kilo", "--prompt", "Fix it"]);
        // Those that can't start with a message aren't offered for new sessions.
        for kind in
            [AgentKind::Amp, AgentKind::Crush, AgentKind::Cline, AgentKind::Kimi, AgentKind::Letta, AgentKind::Aider]
        {
            assert!(kind.command("Fix it", &[]).is_none() && !kind.launches(), "{kind:?}");
        }
        assert!(AgentKind::Cursor.launches() && AgentKind::Droid.launches());
    }

    #[test]
    fn recognizes_each_agent_as_it_runs() {
        // Process shapes as `ps` showed them, each agent started in a terminal (2026-11-18).
        let of = |executable: &str, args: &str| {
            let args: Vec<&str> = args.split(' ').filter(|a| !a.is_empty()).collect();
            AgentKind::of_process(executable, &args)
        };
        let npm = "/opt/homebrew/lib/node_modules";
        let cases = [
            // npm's loader runs Copilot's own program, as Homebrew's install is.
            (of("node", "node /opt/homebrew/bin/copilot"), AgentKind::Copilot),
            (of(&format!("{npm}/@github/copilot-darwin-arm64/copilot"), ""), AgentKind::Copilot),
            // Cursor's `node`, named as it was started (`ps` on a Mac), or by its path (Linux, Windows).
            (
                of(
                    "/Users/me/.local/bin/cursor-agent",
                    "/Users/me/.local/bin/cursor-agent --use-system-ca /Users/me/.local/share/cursor-agent/versions/2026.11.17-f532c45/index.js",
                ),
                AgentKind::Cursor,
            ),
            (
                of(
                    "/Users/me/.local/bin/agent",
                    "/Users/me/.local/bin/agent --use-system-ca /Users/me/.local/share/cursor-agent/versions/2026.11.17-f532c45/index.js",
                ),
                AgentKind::Cursor,
            ),
            (of("/home/me/.local/share/cursor-agent/versions/2026.11.17-f532c45/node", ""), AgentKind::Cursor),
            (of(r"C:\Users\me\AppData\Local\cursor-agent\versions\2026.11.17\node.exe", ""), AgentKind::Cursor),
            (of("/Users/me/.local/bin/droid", ""), AgentKind::Droid),
            (of(&format!("{npm}/@ampcode/cli-darwin-arm64/amp"), ""), AgentKind::Amp),
            (of("/Users/me/.local/bin/kiro-cli", "kiro-cli chat"), AgentKind::Kiro),
            (of("/opt/homebrew/bin/goose", "goose session"), AgentKind::Goose),
            (of("node", "node /opt/homebrew/bin/crush"), AgentKind::Crush),
            (of(&format!("{npm}/@charmland/crush/bin/crush"), ""), AgentKind::Crush),
            (of("/opt/homebrew/bin/cline", ""), AgentKind::Cline),
            (of(&format!("{npm}/@kilocode/cli-darwin-arm64/bin/kilo"), ""), AgentKind::Kilo),
            (of("node", "node /opt/homebrew/bin/auggie"), AgentKind::Auggie),
            // Pi and Kimi Code name their processes.
            (of("pi", "pi"), AgentKind::Pi),
            (of("kimi-code", "kimi-code"), AgentKind::Kimi),
            (
                of(
                    r"C:\Program Files\nodejs\node.exe",
                    &format!(r"node {npm}\@mariozechner\pi-coding-agent\dist\cli.js"),
                ),
                AgentKind::Pi,
            ),
            (of("node", "node /opt/homebrew/bin/cn"), AgentKind::Continue),
            (of("node", &format!("node {npm}/@continuedev/cli/dist/cn.js")), AgentKind::Continue),
            (
                of(
                    "/opt/homebrew/Cellar/node/26.13.1/bin/node",
                    &format!("node {npm}/@iflow-ai/iflow-cli/bundle/iflow.js"),
                ),
                AgentKind::IFlow,
            ),
            (
                of("/Users/me/.local/share/uv/tools/mistral-vibe/lib/python3.14/site-packages/vibe/_bin/vibe-rs", ""),
                AgentKind::Vibe,
            ),
            (of("node", &format!("node {npm}/@letta-ai/letta-code/letta.js")), AgentKind::Letta),
            (
                of("/Users/me/.local/share/uv/tools/aider-chat/bin/python", "python /Users/me/.local/bin/aider"),
                AgentKind::Aider,
            ),
            (of("python3.12", "python3.12 -m aider --model sonnet"), AgentKind::Aider),
        ];
        for (found, kind) in cases {
            assert_eq!(found, Some(kind));
        }
        // What's named like one, but isn't.
        assert_eq!(of("/usr/local/bin/agent", "agent --verbose"), None);
        assert_eq!(of("python3", "python3 manage.py runserver"), None);
        assert_eq!(of("node", "node /x/dist/pipeline.js"), None);
        assert!(
            AgentKind::may_be("/Users/me/.local/bin/agent") && AgentKind::may_be("Python") && !AgentKind::may_be("zsh")
        );
        assert!(is_runtime("python3.12") && is_runtime(r"C:\Python312\python.exe") && !is_runtime("pythonista"));
    }

    #[test]
    fn tells_sessions_from_one_off_runs() {
        let session =
            |kind: AgentKind, line: &str| kind.is_session(&line.split(' ').map(String::from).collect::<Vec<_>>());
        assert!(session(AgentKind::Claude, "claude --model opus") && !session(AgentKind::Claude, "claude -p hi"));
        assert!(!session(AgentKind::Claude, "claude --output-format=stream-json"));
        assert!(!session(AgentKind::Codex, "codex exec fix") && session(AgentKind::Codex, "codex resume"));
        assert!(session(AgentKind::Goose, "goose session") && session(AgentKind::Goose, "goose"));
        assert!(session(AgentKind::Goose, "goose run -s --text=hi") && !session(AgentKind::Goose, "goose run -t hi"));
        assert!(!session(AgentKind::Goose, "goose postgres user=me up"), "the database tool named goose");
        assert!(session(AgentKind::Cline, "cline") && session(AgentKind::Cline, "cline -i fix"));
        assert!(!session(AgentKind::Cline, "cline fix it") && !session(AgentKind::Cline, "cline auth"));
        assert!(
            session(AgentKind::Amp, "amp")
                && !session(AgentKind::Amp, "amp threads")
                && !session(AgentKind::Amp, "amp -x hi")
        );
        assert!(session(AgentKind::Kiro, "kiro-cli") && session(AgentKind::Kiro, "kiro-cli chat"));
        assert!(
            !session(AgentKind::Kiro, "kiro-cli login")
                && !session(AgentKind::Kiro, "kiro-cli chat --no-interactive hi")
        );
        assert!(session(AgentKind::Aider, "aider src/a.py") && !session(AgentKind::Aider, "aider --message hi"));
        assert!(session(AgentKind::Droid, "droid fix") && !session(AgentKind::Droid, "droid exec fix"));
        assert!(
            !session(AgentKind::Cursor, "cursor-agent -p fix") && !session(AgentKind::Cursor, "cursor-agent login")
        );
        assert!(!session(AgentKind::Crush, "crush run hi") && session(AgentKind::Crush, "crush -y"));
        assert!(!session(AgentKind::Vibe, "vibe --prompt=hi") && session(AgentKind::Vibe, "vibe fix"));
        assert!(!session(AgentKind::Kilo, "kilo serve") && session(AgentKind::Kilo, "kilo /Users/me/shop"));
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
