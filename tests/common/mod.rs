#![allow(dead_code)]

use std::path::Path;

use agents_sync::adapters::mem_fs::MemFs;
use agents_sync::app::{self, plan::Plan, Env};

pub const MANIFEST: &str = "/h/.agents/agents.toml";
pub const LAUNCHER: &str = "/opt/bin/agents-sync";

pub const FULL: &str = r#"
version = 1

[instructions]
source = "AGENTS.md"

[skills]
dir = "skills"
exclude = ["cursor-*"]

[agents]
dir = "agents"

[[hooks]]
event = "PreToolUse"
matcher = "Bash"
command = "rtk hook claude"

[[hooks]]
event = "SessionStart"
command = "node ~/.claude/hooks/gsd-check-update.js"

[[hooks]]
event = "PostToolUseFailure"
command = "echo failed"

[mcp.github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_PERSONAL_ACCESS_TOKEN = "keychain:agents-sync.github" }

[mcp.notion]
url = "https://mcp.notion.com/mcp"

[mcp.plain]
command = "uvx"
args = ["plain-mcp"]
env = { MODE = "fast" }
skip = ["cursor"]

[targets.claude]
[targets.codex]
[targets.copilot]
[targets.qoder]
[targets.cursor]
"#;

pub const ALPHA: &str = "---\nname: alpha\ndescription: A\n---\nbody\n";

pub fn env() -> Env {
    Env {
        home: "/h".into(),
        vars: Default::default(),
        launcher: LAUNCHER.into(),
    }
}

pub fn base_fs(manifest: &str) -> MemFs {
    MemFs::default()
        .with_file(MANIFEST, manifest)
        .with_file("/h/.agents/AGENTS.md", "# Shared rules\n")
        .with_file("/h/.agents/skills/alpha/SKILL.md", ALPHA)
        .with_file(
            "/h/.agents/skills/beta/SKILL.md",
            "---\nname: beta\ndescription: B\n---\nbody\n",
        )
        .with_file(
            "/h/.agents/skills/cursor-only/SKILL.md",
            "---\nname: cursor-only\n---\n",
        )
        .with_file(
            "/h/.agents/agents/reviewer.md",
            "---\nname: reviewer\ndescription: Reviews code\ntools: Read\n---\n<role>\nYou review.\n</role>\n",
        )
}

pub fn plan(fs: &MemFs, adopt: bool) -> Plan {
    let loaded = app::load(fs, Path::new(MANIFEST), &env()).unwrap();
    app::compute_plan(fs, &loaded, adopt).unwrap()
}

pub fn sync(fs: &MemFs, adopt: bool) -> Plan {
    let loaded = app::load(fs, Path::new(MANIFEST), &env()).unwrap();
    app::sync(fs, &loaded, adopt, "T").unwrap().0
}

pub fn json(fs: &MemFs, path: &str) -> serde_json::Value {
    use agents_sync::ports::FileSystem;
    serde_json::from_str(&fs.read_to_string(Path::new(path)).unwrap()).unwrap()
}
