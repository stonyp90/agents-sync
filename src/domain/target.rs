/// A coding agent whose configuration agents-sync writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Target {
    Claude,
    Codex,
    Copilot,
    Qoder,
    Cursor,
}

impl Target {
    pub const ALL: [Target; 5] = [
        Target::Claude,
        Target::Codex,
        Target::Copilot,
        Target::Qoder,
        Target::Cursor,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Target::Claude => "claude",
            Target::Codex => "codex",
            Target::Copilot => "copilot",
            Target::Qoder => "qoder",
            Target::Cursor => "cursor",
        }
    }

    pub fn from_name(name: &str) -> Option<Target> {
        Target::ALL.into_iter().find(|t| t.name() == name)
    }

    pub fn names() -> String {
        Target::ALL.map(Target::name).join(", ")
    }

    /// Directory under the user's home when neither the manifest nor the
    /// environment says otherwise.
    pub fn default_home_dir(self) -> &'static str {
        match self {
            Target::Claude => ".claude",
            Target::Codex => ".codex",
            Target::Copilot => ".copilot",
            Target::Qoder => ".qoder",
            Target::Cursor => ".cursor",
        }
    }

    /// Environment variable the tool itself honours to relocate its home.
    pub fn home_env_var(self) -> Option<&'static str> {
        match self {
            Target::Claude => Some("CLAUDE_CONFIG_DIR"),
            Target::Codex => Some("CODEX_HOME"),
            _ => None,
        }
    }

    /// Global instructions file inside the tool's home.
    pub fn instructions_file(self) -> Option<&'static str> {
        match self {
            Target::Claude => Some("CLAUDE.md"),
            Target::Codex | Target::Qoder => Some("AGENTS.md"),
            Target::Copilot => Some("copilot-instructions.md"),
            // Cursor keeps user rules in its app database, not in a file.
            Target::Cursor => None,
        }
    }

    /// Codex and Copilot read `~/.agents/skills` natively.
    pub fn reads_native_skills(self) -> bool {
        matches!(self, Target::Codex | Target::Copilot)
    }

    pub fn syncs_hooks(self) -> bool {
        // Cursor hooks use a different event model; they are not translated.
        self != Target::Cursor
    }

    /// Settings file that holds the `hooks` key (Copilot uses its own file).
    pub fn hooks_file(self) -> &'static str {
        match self {
            Target::Codex => "hooks.json",
            _ => "settings.json",
        }
    }

    pub fn supports_event(self, event: &str) -> bool {
        match self {
            Target::Claude | Target::Qoder => true,
            Target::Codex => !matches!(event, "PostToolUseFailure" | "Notification"),
            Target::Copilot => event != "PostCompact",
            Target::Cursor => false,
        }
    }
}
