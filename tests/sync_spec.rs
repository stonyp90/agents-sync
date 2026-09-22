//! Behaviour of a full sync, expressed as given/when/then scenarios on an
//! in-memory home directory.

mod common;

use std::path::{Path, PathBuf};

use agents_sync::app::plan::{Change, Replacing};
use agents_sync::ports::{FileSystem, Kind};
use common::*;

fn kind(fs: &impl FileSystem, p: &str) -> Kind {
    fs.kind(Path::new(p)).unwrap()
}

fn link_to(p: &str) -> Kind {
    Kind::Symlink(PathBuf::from(p))
}

#[test]
fn given_an_empty_home_when_syncing_then_every_tool_links_the_shared_instructions() {
    let fs = base_fs(FULL);
    sync(&fs, false);

    for p in [
        "/h/.claude/CLAUDE.md",
        "/h/.codex/AGENTS.md",
        "/h/.copilot/copilot-instructions.md",
        "/h/.qoder/AGENTS.md",
    ] {
        assert_eq!(kind(&fs, p), link_to("/h/.agents/AGENTS.md"), "{p}");
        assert_eq!(fs.read_to_string(Path::new(p)).unwrap(), "# Shared rules\n");
    }
}

#[test]
fn skills_are_linked_only_into_tools_that_do_not_read_the_shared_folder_and_exclusions_hold() {
    let fs = base_fs(FULL);
    sync(&fs, false);

    for home in [".claude", ".qoder", ".cursor"] {
        for skill in ["alpha", "beta"] {
            let p = format!("/h/{home}/skills/{skill}");
            assert_eq!(
                kind(&fs, &p),
                link_to(&format!("/h/.agents/skills/{skill}"))
            );
        }
        assert_eq!(
            kind(&fs, &format!("/h/{home}/skills/cursor-only")),
            Kind::Missing
        );
    }
    // Codex and Copilot read ~/.agents/skills natively: nothing to link.
    assert_eq!(kind(&fs, "/h/.codex/skills/alpha"), Kind::Missing);
    assert_eq!(kind(&fs, "/h/.copilot/skills/alpha"), Kind::Missing);
}

#[test]
fn an_identical_copy_is_adopted_with_a_backup_but_a_different_one_waits_for_adopt() {
    let fs = base_fs(FULL)
        .with_file("/h/.claude/skills/alpha/SKILL.md", ALPHA)
        .with_file("/h/.claude/skills/beta/SKILL.md", "edited by hand\n");

    let p = plan(&fs, false);
    assert!(p.changes.iter().any(|c| matches!(c,
        Change::Link { path, replacing: Replacing::Existing { identical: true }, .. }
            if path == Path::new("/h/.claude/skills/alpha"))));
    assert!(p.changes.iter().any(|c| matches!(c,
        Change::Conflict { path, .. } if path == Path::new("/h/.claude/skills/beta"))));

    sync(&fs, false);
    assert_eq!(
        kind(&fs, "/h/.claude/skills/alpha"),
        link_to("/h/.agents/skills/alpha")
    );
    assert_eq!(
        fs.read_to_string(Path::new(
            "/h/.agents/backups/T/.claude/skills/alpha/SKILL.md"
        ))
        .unwrap(),
        ALPHA
    );
    assert_eq!(
        kind(&fs, "/h/.claude/skills/beta"),
        Kind::Dir,
        "conflict left untouched"
    );

    sync(&fs, true);
    assert_eq!(
        kind(&fs, "/h/.claude/skills/beta"),
        link_to("/h/.agents/skills/beta")
    );
    assert_eq!(
        fs.read_to_string(Path::new(
            "/h/.agents/backups/T/.claude/skills/beta/SKILL.md"
        ))
        .unwrap(),
        "edited by hand\n"
    );
}

#[test]
fn codex_keeps_its_own_mcp_servers_and_secrets_only_travel_through_the_launcher() {
    let fs = base_fs(FULL).with_file(
        "/h/.codex/config.toml",
        "model = \"gpt-5.5\"\n\n[mcp_servers.node_repl]\ncommand = \"/app/node_repl\"\n\n\
         [mcp_servers.github]\ncommand = \"npx\"\n\n[mcp_servers.github.env]\n\
         GITHUB_PERSONAL_ACCESS_TOKEN = \"github_pat_LEAK\"\n",
    );
    sync(&fs, false);

    let text = fs
        .read_to_string(Path::new("/h/.codex/config.toml"))
        .unwrap();
    assert!(
        !text.contains("github_pat_LEAK"),
        "plaintext token must be gone:\n{text}"
    );
    let v: toml::Value = toml::from_str(&text).unwrap();
    assert_eq!(v["model"].as_str(), Some("gpt-5.5"));
    let servers = &v["mcp_servers"];
    assert_eq!(
        servers["node_repl"]["command"].as_str(),
        Some("/app/node_repl")
    );
    assert_eq!(servers["github"]["command"].as_str(), Some(LAUNCHER));
    let args: Vec<&str> = servers["github"]["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    assert_eq!(args, ["exec-mcp", "github", "--manifest", MANIFEST]);
    assert!(servers["github"].get("env").is_none());
    assert_eq!(
        servers["notion"]["url"].as_str(),
        Some("https://mcp.notion.com/mcp")
    );
    assert_eq!(servers["plain"]["env"]["MODE"].as_str(), Some("fast"));
}

#[test]
fn each_json_tool_gets_its_own_mcp_dialect() {
    let fs = base_fs(FULL);
    sync(&fs, false);

    let claude = json(&fs, "/h/.claude.json");
    assert_eq!(claude["mcpServers"]["plain"]["type"], "stdio");
    assert_eq!(claude["mcpServers"]["notion"]["type"], "http");

    let copilot = json(&fs, "/h/.copilot/mcp-config.json");
    assert_eq!(copilot["mcpServers"]["plain"]["type"], "local");
    assert_eq!(copilot["mcpServers"]["plain"]["tools"][0], "*");

    let qoder = json(&fs, "/h/.qoder/settings.json");
    assert_eq!(qoder["mcpServers"]["plain"]["command"], "uvx");
    assert!(qoder["mcpServers"]["plain"].get("type").is_none());

    let cursor = json(&fs, "/h/.cursor/mcp.json");
    assert!(
        cursor["mcpServers"].get("plain").is_none(),
        "plain skips cursor"
    );
    assert_eq!(
        cursor["mcpServers"]["notion"]["url"],
        "https://mcp.notion.com/mcp"
    );
}

#[test]
fn a_server_dropped_from_the_manifest_is_pruned_but_servers_added_by_hand_stay() {
    let fs = base_fs(FULL).with_file(
        "/h/.cursor/mcp.json",
        r#"{"mcpServers":{"mine":{"command":"x"}}}"#,
    );
    sync(&fs, false);
    fs.write(
        Path::new(MANIFEST),
        &FULL.replace("[mcp.notion]\nurl = \"https://mcp.notion.com/mcp\"\n", ""),
    )
    .unwrap();
    sync(&fs, false);

    let cursor = json(&fs, "/h/.cursor/mcp.json");
    let names: Vec<&String> = cursor["mcpServers"].as_object().unwrap().keys().collect();
    assert_eq!(names, ["mine", "github"]);
}

#[test]
fn hooks_replace_the_tool_hooks_and_keep_every_other_setting() {
    let fs = base_fs(FULL).with_file(
        "/h/.claude/settings.json",
        r#"{"model":"opus","hooks":{"Stop":[{"hooks":[{"type":"command","command":"old"}]}]}}"#,
    );
    let p = sync(&fs, false);

    let claude = json(&fs, "/h/.claude/settings.json");
    assert_eq!(claude["model"], "opus");
    assert_eq!(claude["hooks"]["PreToolUse"][0]["matcher"], "Bash");
    assert_eq!(
        claude["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
        "rtk hook claude"
    );
    assert!(claude["hooks"].get("Stop").is_none());
    assert!(claude["hooks"].get("PostToolUseFailure").is_some());

    let codex = json(&fs, "/h/.codex/hooks.json");
    assert!(codex["hooks"].get("PostToolUseFailure").is_none());
    assert!(p
        .warnings
        .iter()
        .any(|w| w.contains("codex") && w.contains("PostToolUseFailure")));

    let copilot = json(&fs, "/h/.copilot/hooks/agents-sync.json");
    assert_eq!(copilot["version"], 1);
    assert_eq!(copilot["hooks"]["PreToolUse"][0]["bash"], "rtk hook claude");
    assert_eq!(copilot["hooks"]["PreToolUse"][0]["matcher"], "Bash");

    let qoder = json(&fs, "/h/.qoder/settings.json");
    assert_eq!(
        qoder["hooks"]["SessionStart"][0]["hooks"][0]["type"],
        "command"
    );

    assert_eq!(kind(&fs, "/h/.cursor/hooks.json"), Kind::Missing);
}

#[test]
fn codex_gets_toml_subagents_generated_from_the_markdown_source() {
    let fs = base_fs(FULL);
    sync(&fs, false);

    let text = fs
        .read_to_string(Path::new("/h/.codex/agents/reviewer.toml"))
        .unwrap();
    let v: toml::Value = toml::from_str(&text).unwrap();
    assert_eq!(v["name"].as_str(), Some("reviewer"));
    assert_eq!(v["description"].as_str(), Some("Reviews code"));
    assert_eq!(
        v["developer_instructions"].as_str(),
        Some("<role>\nYou review.\n</role>\n")
    );
    assert_eq!(
        kind(&fs, "/h/.claude/agents/reviewer.md"),
        link_to("/h/.agents/agents/reviewer.md")
    );
}

#[test]
fn an_unmanaged_codex_agent_file_is_a_conflict_until_adopted() {
    let fs = base_fs(FULL).with_file("/h/.codex/agents/reviewer.toml", "name = \"old\"\n");
    let target = Path::new("/h/.codex/agents/reviewer.toml");

    assert!(plan(&fs, false)
        .changes
        .iter()
        .any(|c| matches!(c, Change::Conflict { path, .. } if path == target)));
    assert!(plan(&fs, true)
        .changes
        .iter()
        .any(|c| matches!(c, Change::Write { path, before: Some(_), .. } if path == target)));
}

#[test]
fn a_second_sync_has_nothing_left_to_do() {
    let fs = base_fs(FULL)
        .with_file("/h/.codex/config.toml", "model = \"gpt-5.5\"\n")
        .with_file("/h/.claude.json", "{\n  \"numStartups\": 3\n}\n");
    sync(&fs, false);

    let p = plan(&fs, false);
    assert!(p.changes.is_empty(), "{:#?}", p.changes);
}

#[test]
fn removing_a_skill_or_an_agent_unlinks_it_everywhere() {
    let fs = base_fs(FULL);
    sync(&fs, false);
    fs.rename(
        Path::new("/h/.agents/skills/beta"),
        Path::new("/h/trash/beta"),
    )
    .unwrap();
    fs.rename(
        Path::new("/h/.agents/agents/reviewer.md"),
        Path::new("/h/trash/r.md"),
    )
    .unwrap();
    sync(&fs, false);

    assert_eq!(kind(&fs, "/h/.claude/skills/beta"), Kind::Missing);
    assert_eq!(kind(&fs, "/h/.claude/agents/reviewer.md"), Kind::Missing);
    assert_eq!(kind(&fs, "/h/.codex/agents/reviewer.toml"), Kind::Missing);
    assert_eq!(
        kind(&fs, "/h/.agents/backups/T/.codex/agents/reviewer.toml"),
        Kind::File,
        "generated file is backed up, not deleted"
    );
}

#[test]
fn a_tool_folder_that_already_points_at_the_shared_store_is_never_moved_away() {
    let fs = base_fs(FULL).with_link("/h/.claude/skills", "/h/.agents/skills");
    let p = plan(&fs, true);

    assert!(
        !p.changes
            .iter()
            .any(|c| c.path().starts_with("/h/.claude/skills")),
        "{:#?}",
        p.changes
    );
    sync(&fs, true);
    assert_eq!(kind(&fs, "/h/.agents/skills/alpha/SKILL.md"), Kind::File);
}

#[test]
fn only_listed_and_enabled_targets_are_touched() {
    let manifest = FULL
        .replace("[targets.codex]\n", "")
        .replace("[targets.copilot]\n", "")
        .replace("[targets.cursor]\n", "")
        .replace("[targets.qoder]\n", "[targets.qoder]\nenabled = false\n");
    let fs = base_fs(&manifest);
    let p = sync(&fs, false);

    assert!(p.changes.iter().all(|c| c.target().name() == "claude"));
    for dir in ["/h/.codex", "/h/.copilot", "/h/.qoder", "/h/.cursor"] {
        assert_eq!(kind(&fs, dir), Kind::Missing, "{dir}");
    }
}
