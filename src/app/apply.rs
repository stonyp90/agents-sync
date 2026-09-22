use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::plan::{Change, Plan, Replacing, State};
use crate::ports::FileSystem;

#[derive(Debug, Default)]
pub struct ApplyReport {
    pub applied: usize,
    pub conflicts: usize,
    pub backed_up: bool,
}

/// Carries out a plan. Anything replaced or removed is moved or copied under
/// `backup_dir` first, mirroring its path relative to `home`. Conflicts are
/// skipped.
pub fn apply(
    plan: &Plan,
    fs: &dyn FileSystem,
    backup_dir: &Path,
    home: &Path,
    state_path: &Path,
) -> Result<ApplyReport> {
    let backup = |path: &Path| -> PathBuf {
        let relative = path
            .strip_prefix(home)
            .unwrap_or_else(|_| path.strip_prefix("/").unwrap_or(path));
        backup_dir.join(relative)
    };
    let mut report = ApplyReport::default();

    for change in &plan.changes {
        let path = change.path();
        let context = || format!("applying change to {}", path.display());
        match change {
            Change::Conflict { .. } => {
                report.conflicts += 1;
                continue;
            }
            Change::Link { to, replacing, .. } => {
                match replacing {
                    Replacing::Nothing => {}
                    Replacing::Link(_) => fs.remove_link(path).with_context(context)?,
                    Replacing::Existing { .. } => {
                        fs.rename(path, &backup(path)).with_context(context)?;
                        report.backed_up = true;
                    }
                }
                fs.symlink(to, path).with_context(context)?;
            }
            Change::RemoveLink { .. } => fs.remove_link(path).with_context(context)?,
            Change::Write { before, after, .. } => {
                if before.is_some() {
                    fs.copy_file(path, &backup(path)).with_context(context)?;
                    report.backed_up = true;
                }
                fs.write(path, after).with_context(context)?;
            }
            Change::RemoveFile { .. } => {
                fs.rename(path, &backup(path)).with_context(context)?;
                report.backed_up = true;
            }
        }
        report.applied += 1;
    }

    save_state(fs, state_path, &plan.next_state)?;
    Ok(report)
}

fn save_state(fs: &dyn FileSystem, path: &Path, state: &State) -> Result<()> {
    if State::load(fs, path).ok().as_ref() == Some(state) {
        return Ok(());
    }
    let text = serde_json::to_string_pretty(state)? + "\n";
    fs.write(path, &text)
        .with_context(|| format!("writing state {}", path.display()))
}
