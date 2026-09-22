# agents-sync

One manifest, every coding agent. `~/.agents/agents.toml` describes your
instructions, skills, subagents, hooks and MCP servers once; agents-sync
writes each tool's own configuration from it.

| | Claude Code | Codex | Copilot CLI | Qoder | Cursor |
|---|---|---|---|---|---|
| Instructions | link `CLAUDE.md` | link `AGENTS.md` | link `copilot-instructions.md` | link `AGENTS.md` | — (kept in app DB) |
| Skills | links in `skills/` | native `~/.agents/skills` | native `~/.agents/skills` | links in `skills/` | links in `skills/` |
| Subagents | links in `agents/` | generated `agents/*.toml` | links in `agents/` | links in `agents/` | links in `agents/` |
| Hooks | `settings.json` → `hooks` | `hooks.json` | `hooks/agents-sync.json` | `settings.json` → `hooks` | — |
| MCP | `~/.claude.json` | `config.toml` `[mcp_servers]` | `mcp-config.json` | `settings.json` → `mcpServers` | `mcp.json` |

## Use

```bash
cargo install --path .
agents-sync plan --diff        # dry run, nothing written
agents-sync sync               # apply; replaced files go to ~/.agents/backups/<time>/
agents-sync sync --adopt       # also replace files whose content differs (backed up)
agents-sync check              # exit 1 on drift (`--hook` for session-start hooks)
agents-sync watch install      # launchd: re-sync when the manifest or store changes
```

See [examples/agents.toml](examples/agents.toml) for a commented manifest.

## Guarantees

- **Links first.** Markdown content (instructions, skills, subagents) is
  linked, not copied, so an edit is live in every tool at once.
- **Key-level edits.** In shared files agents-sync only touches the keys it
  owns (`hooks`, and the MCP entries it created). Models, permissions,
  plugins, trusted projects and memories stay each tool's own.
- **Nothing is lost.** Every replaced or removed file is moved or copied to
  `backups/<timestamp>/` first. A real file that differs from the shared
  version is a conflict until you pass `--adopt`. The shared store is never
  moved, even when a tool folder already links into it.
- **No secret on disk.** MCP `env` values like `keychain:<service>` make the
  generated entry call `agents-sync exec-mcp <name>`, which reads the macOS
  Keychain when the server starts. Store a secret with:
  `security add-generic-password -s <service> -a "$USER" -w`
- **Pruning.** Servers, links and generated files dropped from the manifest
  are removed; the state file (`.agents-sync-state.json`) remembers ownership.

## Known limits

- macOS only for now: secrets come from the login Keychain and `watch`
  uses launchd.
- Copilot hooks are written with Claude event names (`PreToolUse`…), which
  Copilot documents as its compatible mode; not yet confirmed in a live session.
- Cursor hooks and Cursor user rules are not synced (different event model;
  rules live in the app database).
- Codex reads `~/.agents/skills` directly, so `skills.exclude` cannot hide a
  skill from Codex or Copilot; it only limits the linked tools.
- Changing Codex hooks makes Codex ask to trust them again.

## Architecture

Hexagonal (ports and adapters):

```
src/
  domain/     pure: manifest model + validation, per-tool translation
    manifest.rs   the single file's schema
    render.rs     Manifest → Vec<Desired> (links, keys, generated files)
    hooks.rs mcp.rs agent.rs   per-dialect rendering
  ports.rs    FileSystem, SecretStore traits
  adapters/   real_fs (atomic writes, keeps permissions), mem_fs (tests),
              secrets (Keychain / env)
  app/        use cases: plan (desired vs disk → changes), apply (with
              backups), formats (JSON/TOML key edits), exec_mcp, watch
  main.rs     CLI wiring the real adapters
```

The domain never touches the disk; `app::plan` only reads through the
`FileSystem` port, and `app::apply` is the only writer. Tests run the whole
use case against the in-memory filesystem (`tests/sync_spec.rs`), plus one
end-to-end test on a real temporary directory (`tests/real_fs_spec.rs`).

No docker-compose: agents-sync is a local CLI with no services or databases.
No OpenAPI spec: it exposes no HTTP API.
