//! Use cases. Each takes its ports as arguments; `main` supplies the real ones.

pub mod apply;
pub mod exec_mcp;
pub mod formats;
pub mod plan;
pub mod watch;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::domain::agent::{parse_agent, AgentDef};
use crate::domain::manifest::Manifest;
use crate::domain::paths::expand;
use crate::domain::render::{render, target_paths, Layout};
use crate::ports::FileSystem;
use apply::ApplyReport;
use plan::{Plan, PlanOptions, State};

const DEFAULT_NATIVE_SKILLS: &str = "~/.agents/skills";
const DEFAULT_STATE: &str = ".agents-sync-state.json";
const DEFAULT_BACKUPS: &str = "backups";
const DEFAULT_LOG: &str = "logs/agents-sync.log";

/// What the process knows about its surroundings.
pub struct Env {
    pub home: PathBuf,
    /// Only the variables tools use to relocate themselves.
    pub vars: BTreeMap<String, String>,
    /// Path of the running agents-sync binary.
    pub launcher: PathBuf,
}

pub struct Loaded {
    pub manifest: Manifest,
    pub layout: Layout,
    pub state_path: PathBuf,
    pub backups_dir: PathBuf,
    pub log_path: PathBuf,
}

pub fn load(fs: &dyn FileSystem, manifest_path: &Path, env: &Env) -> Result<Loaded> {
    let source = fs
        .read_to_string(manifest_path)
        .with_context(|| format!("cannot read manifest {}", manifest_path.display()))?;
    let manifest = Manifest::parse(&source)?;
    let base = manifest_path
        .parent()
        .unwrap_or(Path::new("/"))
        .to_path_buf();
    let ex = |raw: &str| expand(raw, &base, &env.home);
    let var = |key: &str| env.vars.get(key).cloned();

    let homes = manifest
        .enabled_targets()
        .into_iter()
        .map(|t| {
            let config = &manifest.targets[t.name()];
            (t, target_paths(t, config, &env.home, &base, &var))
        })
        .collect();
    let paths = &manifest.paths;
    let layout = Layout {
        user_home: env.home.clone(),
        manifest_path: manifest_path.to_path_buf(),
        launcher: paths
            .launcher
            .as_deref()
            .map(ex)
            .unwrap_or_else(|| env.launcher.clone()),
        instructions: manifest.instructions.as_ref().map(|i| ex(&i.source)),
        skills_dir: manifest.skills.as_ref().map(|s| ex(&s.dir)),
        agents_dir: manifest.agents.as_ref().map(|a| ex(&a.dir)),
        native_skills_dir: ex(paths
            .native_skills
            .as_deref()
            .unwrap_or(DEFAULT_NATIVE_SKILLS)),
        homes,
    };
    Ok(Loaded {
        state_path: ex(paths.state.as_deref().unwrap_or(DEFAULT_STATE)),
        backups_dir: ex(paths.backups.as_deref().unwrap_or(DEFAULT_BACKUPS)),
        log_path: ex(manifest.watch.log.as_deref().unwrap_or(DEFAULT_LOG)),
        manifest,
        layout,
    })
}

/// Skills are folders holding a `SKILL.md`; agents are `*.md` files.
fn discover(fs: &dyn FileSystem, layout: &Layout) -> Result<(Vec<String>, Vec<AgentDef>)> {
    let mut skills = Vec::new();
    if let Some(dir) = &layout.skills_dir {
        for entry in fs.list_dir(dir)? {
            let Some(name) = entry.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                continue;
            };
            if !name.starts_with('.') && fs.read(&entry.join("SKILL.md")).is_ok() {
                skills.push(name);
            }
        }
    }
    let mut agents = Vec::new();
    if let Some(dir) = &layout.agents_dir {
        for entry in fs.list_dir(dir)? {
            if entry.extension().is_some_and(|e| e == "md") {
                let text = fs.read_to_string(&entry)?;
                agents.push(parse_agent(&entry, &text));
            }
        }
    }
    Ok((skills, agents))
}

pub fn compute_plan(fs: &dyn FileSystem, loaded: &Loaded, adopt: bool) -> Result<Plan> {
    let layout = &loaded.layout;
    let (skills, agents) = discover(fs, layout)?;
    let rendered = render(&loaded.manifest, layout, &skills, &agents);
    let state = State::load(fs, &loaded.state_path)?;
    let protected = [&layout.instructions, &layout.skills_dir, &layout.agents_dir]
        .into_iter()
        .flatten()
        .cloned()
        .collect();
    plan::plan(&rendered, fs, &state, &PlanOptions { adopt, protected })
}

/// Plans then applies; backups land in `<backups>/<stamp>/`.
pub fn sync(
    fs: &dyn FileSystem,
    loaded: &Loaded,
    adopt: bool,
    stamp: &str,
) -> Result<(Plan, ApplyReport)> {
    let plan = compute_plan(fs, loaded, adopt)?;
    let report = apply::apply(
        &plan,
        fs,
        &loaded.backups_dir.join(stamp),
        &loaded.layout.user_home,
        &loaded.state_path,
    )?;
    Ok((plan, report))
}

/// Paths whose changes should trigger a sync.
pub fn watch_paths(loaded: &Loaded) -> Vec<PathBuf> {
    let layout = &loaded.layout;
    std::iter::once(layout.manifest_path.clone())
        .chain(layout.instructions.clone())
        .chain(layout.skills_dir.clone())
        .chain(layout.agents_dir.clone())
        .collect()
}
