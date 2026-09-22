//! Pure translation of the manifest into the state each tool should be in.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::agent::{render_codex_toml, AgentDef};
use super::desired::{Desired, Format, KeyMode};
use super::hooks;
use super::manifest::{Manifest, TargetConfig};
use super::mcp::{self, Launcher};
use super::paths::expand;
use super::target::Target;

#[derive(Debug, Clone)]
pub struct TargetPaths {
    pub home: PathBuf,
    pub mcp_file: PathBuf,
}

/// Where a tool keeps its files: manifest override, then the tool's own
/// environment variable, then its default folder.
pub fn target_paths(
    target: Target,
    config: &TargetConfig,
    user_home: &Path,
    base: &Path,
    var: &dyn Fn(&str) -> Option<String>,
) -> TargetPaths {
    let env_home = target.home_env_var().and_then(var).map(PathBuf::from);
    let relocated = config.home.is_some() || env_home.is_some();
    let home = config
        .home
        .as_deref()
        .map(|h| expand(h, base, user_home))
        .or(env_home)
        .unwrap_or_else(|| user_home.join(target.default_home_dir()));
    let mcp_file = match config.mcp_file.as_deref() {
        Some(file) => expand(file, base, user_home),
        None => match target {
            // Claude keeps user-scope MCP servers next to (not inside) ~/.claude.
            Target::Claude if !relocated => user_home.join(".claude.json"),
            Target::Claude => home.join(".claude.json"),
            Target::Codex => home.join("config.toml"),
            Target::Copilot => home.join("mcp-config.json"),
            Target::Qoder => home.join("settings.json"),
            Target::Cursor => home.join("mcp.json"),
        },
    };
    TargetPaths { home, mcp_file }
}

/// Resolved locations of the shared store and of every enabled tool.
#[derive(Debug, Clone)]
pub struct Layout {
    pub user_home: PathBuf,
    pub manifest_path: PathBuf,
    pub launcher: PathBuf,
    pub instructions: Option<PathBuf>,
    pub skills_dir: Option<PathBuf>,
    pub agents_dir: Option<PathBuf>,
    pub native_skills_dir: PathBuf,
    pub homes: BTreeMap<Target, TargetPaths>,
}

#[derive(Debug, Default)]
pub struct Rendered {
    pub desired: Vec<Desired>,
    pub warnings: Vec<String>,
}

pub fn render(
    manifest: &Manifest,
    layout: &Layout,
    skills: &[String],
    agents: &[AgentDef],
) -> Rendered {
    let excluded: Vec<glob::Pattern> = manifest
        .skills
        .iter()
        .flat_map(|s| s.exclude.iter())
        .filter_map(|p| glob::Pattern::new(p).ok())
        .collect();
    let shared: Vec<&str> = skills
        .iter()
        .map(String::as_str)
        .filter(|s| !excluded.iter().any(|p| p.matches(s)))
        .collect();

    let mut out = Rendered::default();
    for target in manifest.enabled_targets() {
        let ctx = Ctx {
            target,
            paths: &layout.homes[&target],
            manifest,
            layout,
        };
        ctx.instructions(&mut out);
        ctx.skills(&shared, &mut out);
        ctx.agents(agents, &mut out);
        ctx.hooks(&mut out);
        ctx.mcp(&mut out);
    }
    let mut seen = BTreeSet::new();
    out.desired
        .retain(|d| d.dedupe_key().is_none_or(|key| seen.insert(key)));
    out
}

struct Ctx<'a> {
    target: Target,
    paths: &'a TargetPaths,
    manifest: &'a Manifest,
    layout: &'a Layout,
}

impl Ctx<'_> {
    fn instructions(&self, out: &mut Rendered) {
        if let (Some(source), Some(file)) =
            (&self.layout.instructions, self.target.instructions_file())
        {
            out.desired.push(Desired::Link {
                target: self.target,
                path: self.paths.home.join(file),
                to: source.clone(),
            });
        }
    }

    fn skills(&self, shared: &[&str], out: &mut Rendered) {
        let Some(store) = &self.layout.skills_dir else {
            return;
        };
        let link_dir = if self.target.reads_native_skills() {
            self.layout.native_skills_dir.clone()
        } else {
            self.paths.home.join("skills")
        };
        if &link_dir == store {
            return;
        }
        for skill in shared {
            out.desired.push(Desired::Link {
                target: self.target,
                path: link_dir.join(skill),
                to: store.join(skill),
            });
        }
        out.desired.push(Desired::ManagedLinks {
            target: self.target,
            dir: link_dir,
            root: store.clone(),
        });
    }

    fn agents(&self, agents: &[AgentDef], out: &mut Rendered) {
        let Some(store) = &self.layout.agents_dir else {
            return;
        };
        let dir = self.paths.home.join("agents");
        if &dir == store {
            return;
        }
        if self.target == Target::Codex {
            for agent in agents {
                out.desired.push(Desired::File {
                    target: self.target,
                    path: dir.join(format!("{}.toml", agent.stem())),
                    content: render_codex_toml(agent),
                });
            }
            out.desired.push(Desired::ManagedFiles {
                target: self.target,
                dir,
            });
        } else {
            for agent in agents {
                out.desired.push(Desired::Link {
                    target: self.target,
                    path: dir.join(agent.file_name()),
                    to: agent.source.clone(),
                });
            }
            out.desired.push(Desired::ManagedLinks {
                target: self.target,
                dir,
                root: store.clone(),
            });
        }
    }

    fn hooks(&self, out: &mut Rendered) {
        if !self.target.syncs_hooks() {
            return;
        }
        let mut applicable = Vec::new();
        for hook in self
            .manifest
            .hooks
            .iter()
            .filter(|h| h.applies_to(self.target))
        {
            if self.target.supports_event(&hook.event) {
                applicable.push(hook);
            } else {
                out.warnings.push(format!(
                    "{}: `{}` hooks are not supported, skipped",
                    self.target.name(),
                    hook.event
                ));
            }
        }
        if self.target == Target::Copilot {
            let value = hooks::copilot_shape(&applicable);
            out.desired.push(Desired::File {
                target: self.target,
                path: self.paths.home.join("hooks").join("agents-sync.json"),
                content: serde_json::to_string_pretty(&value).expect("JSON value") + "\n",
            });
        } else {
            out.desired.push(Desired::Key {
                target: self.target,
                file: self.paths.home.join(self.target.hooks_file()),
                format: Format::Json,
                key: "hooks".into(),
                value: hooks::claude_shape(&applicable),
                mode: KeyMode::Replace,
            });
        }
    }

    fn mcp(&self, out: &mut Rendered) {
        let launcher = Launcher {
            exe: &self.layout.launcher,
            manifest: &self.layout.manifest_path,
        };
        let servers: Map<String, Value> = self
            .manifest
            .mcp
            .iter()
            .filter(|(_, s)| s.applies_to(self.target))
            .map(|(name, s)| (name.clone(), mcp::render(self.target, name, s, &launcher)))
            .collect();
        let (format, key) = match self.target {
            Target::Codex => (Format::Toml, "mcp_servers"),
            _ => (Format::Json, "mcpServers"),
        };
        out.desired.push(Desired::Key {
            target: self.target,
            file: self.paths.mcp_file.clone(),
            format,
            key: key.into(),
            value: Value::Object(servers),
            mode: KeyMode::MergeOwned,
        });
    }
}
