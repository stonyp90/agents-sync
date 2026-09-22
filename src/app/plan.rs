//! Compares the desired state with what is on disk and lists the changes.
//! Reading only: nothing is written here.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::formats::{self, Edit};
use crate::domain::desired::{Desired, Format};
use crate::domain::render::Rendered;
use crate::domain::target::Target;
use crate::ports::{FileSystem, Kind};

/// What agents-sync remembers owning, so it can prune what the manifest drops.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub owned: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    #[serde(default)]
    pub generated: BTreeSet<String>,
}

impl State {
    pub fn load(fs: &dyn FileSystem, path: &Path) -> Result<State> {
        if fs.kind(path)? == Kind::Missing {
            return Ok(State::default());
        }
        let text = fs.read_to_string(path)?;
        serde_json::from_str(&text)
            .with_context(|| format!("corrupt state file {}", path.display()))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Replacing {
    Nothing,
    Link(PathBuf),
    /// A real file or directory, moved to the backups before linking.
    Existing {
        identical: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Link {
        target: Target,
        path: PathBuf,
        to: PathBuf,
        replacing: Replacing,
    },
    RemoveLink {
        target: Target,
        path: PathBuf,
        was: PathBuf,
    },
    Write {
        target: Target,
        path: PathBuf,
        before: Option<String>,
        after: String,
    },
    RemoveFile {
        target: Target,
        path: PathBuf,
    },
    Conflict {
        target: Target,
        path: PathBuf,
        reason: String,
    },
}

impl Change {
    pub fn path(&self) -> &Path {
        match self {
            Change::Link { path, .. }
            | Change::RemoveLink { path, .. }
            | Change::Write { path, .. }
            | Change::RemoveFile { path, .. }
            | Change::Conflict { path, .. } => path,
        }
    }

    pub fn target(&self) -> Target {
        match self {
            Change::Link { target, .. }
            | Change::RemoveLink { target, .. }
            | Change::Write { target, .. }
            | Change::RemoveFile { target, .. }
            | Change::Conflict { target, .. } => *target,
        }
    }

    pub fn is_conflict(&self) -> bool {
        matches!(self, Change::Conflict { .. })
    }
}

#[derive(Debug)]
pub struct Plan {
    pub changes: Vec<Change>,
    pub warnings: Vec<String>,
    pub next_state: State,
}

pub struct PlanOptions {
    /// Replace real files that differ from the shared version (after backup).
    pub adopt: bool,
    /// The shared store: never moved away, whatever path leads to it.
    pub protected: Vec<PathBuf>,
}

pub fn plan(
    rendered: &Rendered,
    fs: &dyn FileSystem,
    state: &State,
    opts: &PlanOptions,
) -> Result<Plan> {
    let mut planner = Planner {
        fs,
        state,
        opts,
        changes: Vec::new(),
        next: State::default(),
    };
    let desired = &rendered.desired;
    let link_paths: BTreeSet<&Path> = desired
        .iter()
        .filter_map(|d| match d {
            Desired::Link { path, .. } => Some(path.as_path()),
            _ => None,
        })
        .collect();
    let file_paths: BTreeSet<&Path> = desired
        .iter()
        .filter_map(|d| match d {
            Desired::File { path, .. } => Some(path.as_path()),
            _ => None,
        })
        .collect();

    // Structured edits are grouped so each file is read and written once.
    let mut edits: Vec<(PathBuf, Target, Format, Vec<Edit>)> = Vec::new();

    for d in desired {
        match d {
            Desired::Link { target, path, to } => planner.link(*target, path, to)?,
            Desired::ManagedLinks { target, dir, root } => {
                planner.prune_links(*target, dir, root, &link_paths)?
            }
            Desired::Key {
                target,
                file,
                format,
                key,
                value,
                mode,
            } => {
                let edit = Edit {
                    key,
                    value,
                    mode: *mode,
                };
                match edits.iter_mut().find(|(f, ..)| f == file) {
                    Some((.., list)) => list.push(edit),
                    None => edits.push((file.clone(), *target, *format, vec![edit])),
                }
            }
            Desired::File {
                target,
                path,
                content,
            } => planner.file(*target, path, content)?,
            Desired::ManagedFiles { target, dir } => {
                planner.prune_files(*target, dir, &file_paths)?
            }
        }
    }
    for (file, target, format, list) in &edits {
        planner.structured(*target, file, *format, list)?;
    }

    Ok(Plan {
        changes: planner.changes,
        warnings: rendered.warnings.clone(),
        next_state: planner.next,
    })
}

struct Planner<'a> {
    fs: &'a dyn FileSystem,
    state: &'a State,
    opts: &'a PlanOptions,
    changes: Vec<Change>,
    next: State,
}

impl Planner<'_> {
    fn link(&mut self, target: Target, path: &Path, to: &Path) -> Result<()> {
        let replacing = match self.fs.kind(path)? {
            Kind::Missing => Replacing::Nothing,
            Kind::Symlink(current) if resolve_link(path, &current) == to => return Ok(()),
            Kind::Symlink(current) => Replacing::Link(current),
            Kind::File | Kind::Dir => {
                let real = self.fs.canonicalize(path);
                if real.is_some() && real == self.fs.canonicalize(to) {
                    return Ok(()); // Same object, reached through a linked parent.
                }
                if let Some(real) = &real {
                    if self.is_protected(real) {
                        self.changes.push(Change::Conflict {
                            target,
                            path: path.to_path_buf(),
                            reason: "resolves inside the shared store; refusing to replace it"
                                .into(),
                        });
                        return Ok(());
                    }
                }
                let identical = same_content(self.fs, path, to)?;
                if !identical && !self.opts.adopt {
                    self.changes.push(Change::Conflict {
                        target,
                        path: path.to_path_buf(),
                        reason:
                            "exists with different content; review it, then rerun with --adopt \
                                 (it will be backed up)"
                                .into(),
                    });
                    return Ok(());
                }
                Replacing::Existing { identical }
            }
        };
        self.changes.push(Change::Link {
            target,
            path: path.to_path_buf(),
            to: to.to_path_buf(),
            replacing,
        });
        Ok(())
    }

    fn is_protected(&self, real: &Path) -> bool {
        self.opts
            .protected
            .iter()
            .filter_map(|p| self.fs.canonicalize(p))
            .any(|p| real.starts_with(p))
    }

    fn prune_links(
        &mut self,
        target: Target,
        dir: &Path,
        root: &Path,
        keep: &BTreeSet<&Path>,
    ) -> Result<()> {
        for entry in self.fs.list_dir(dir)? {
            if let Kind::Symlink(to) = self.fs.kind(&entry)? {
                if resolve_link(&entry, &to).starts_with(root) && !keep.contains(entry.as_path()) {
                    self.changes.push(Change::RemoveLink {
                        target,
                        path: entry,
                        was: to,
                    });
                }
            }
        }
        Ok(())
    }

    fn file(&mut self, target: Target, path: &Path, content: &str) -> Result<()> {
        let key = path.display().to_string();
        let before = match self.fs.kind(path)? {
            Kind::Missing => None,
            Kind::File => Some(self.fs.read_to_string(path)?),
            _ => {
                self.changes.push(Change::Conflict {
                    target,
                    path: path.to_path_buf(),
                    reason: "expected a regular file".into(),
                });
                return Ok(());
            }
        };
        if before.as_deref() == Some(content) {
            self.next.generated.insert(key);
            return Ok(());
        }
        let ours = self.state.generated.contains(&key);
        if before.is_some() && !ours && !self.opts.adopt {
            self.changes.push(Change::Conflict {
                target,
                path: path.to_path_buf(),
                reason:
                    "exists and was not generated by agents-sync; rerun with --adopt to replace it \
                         (it will be backed up)"
                        .into(),
            });
            return Ok(());
        }
        self.next.generated.insert(key);
        self.changes.push(Change::Write {
            target,
            path: path.to_path_buf(),
            before,
            after: content.to_string(),
        });
        Ok(())
    }

    fn prune_files(&mut self, target: Target, dir: &Path, keep: &BTreeSet<&Path>) -> Result<()> {
        for generated in &self.state.generated {
            let path = PathBuf::from(generated);
            if path.starts_with(dir)
                && !keep.contains(path.as_path())
                && self.fs.kind(&path)? == Kind::File
            {
                self.changes.push(Change::RemoveFile { target, path });
            }
        }
        Ok(())
    }

    fn structured(
        &mut self,
        target: Target,
        file: &Path,
        format: Format,
        edits: &[Edit],
    ) -> Result<()> {
        let key = file.display().to_string();
        let prev = self.state.owned.get(&key).cloned().unwrap_or_default();
        let before = match self.fs.kind(file)? {
            Kind::Missing => None,
            _ => Some(
                self.fs
                    .read_to_string(file)
                    .with_context(|| format!("reading {}", file.display()))?,
            ),
        };
        let edited = match format {
            Format::Json => formats::edit_json(before.as_deref(), edits, &prev),
            Format::Toml => formats::edit_toml(before.as_deref(), edits, &prev),
        };
        match edited {
            Err(e) => {
                if !prev.is_empty() {
                    self.next.owned.insert(key, prev);
                }
                self.changes.push(Change::Conflict {
                    target,
                    path: file.to_path_buf(),
                    reason: format!("cannot edit: {e:#}"),
                });
            }
            Ok(edited) => {
                if !edited.owned.is_empty() {
                    self.next.owned.insert(key, edited.owned);
                }
                if edited.changed {
                    self.changes.push(Change::Write {
                        target,
                        path: file.to_path_buf(),
                        before,
                        after: edited.content,
                    });
                }
            }
        }
        Ok(())
    }
}

fn resolve_link(link: &Path, to: &Path) -> PathBuf {
    if to.is_absolute() {
        to.to_path_buf()
    } else {
        link.parent().unwrap_or(Path::new("/")).join(to)
    }
}

/// Byte-for-byte comparison of two files or two directory trees.
pub fn same_content(fs: &dyn FileSystem, a: &Path, b: &Path) -> io::Result<bool> {
    match (fs.kind(a)?, fs.kind(b)?) {
        (Kind::File, Kind::File) => Ok(fs.read(a)? == fs.read(b)?),
        (Kind::Symlink(x), Kind::Symlink(y)) => Ok(x == y),
        (Kind::Dir, Kind::Dir) => {
            let names = |dir: &Path| -> io::Result<Vec<_>> {
                Ok(fs
                    .list_dir(dir)?
                    .into_iter()
                    .filter_map(|p| p.file_name().map(|n| n.to_os_string()))
                    .collect())
            };
            let (left, right) = (names(a)?, names(b)?);
            if left != right {
                return Ok(false);
            }
            for name in left {
                if !same_content(fs, &a.join(&name), &b.join(&name))? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::desired::KeyMode;

    #[test]
    fn replace_mode_does_not_create_an_empty_key() {
        let value = serde_json::json!({});
        let edits = [Edit {
            key: "hooks",
            value: &value,
            mode: KeyMode::Replace,
        }];
        let edited = formats::edit_json(Some("{\"a\":1}"), &edits, &Default::default()).unwrap();
        assert!(!edited.changed);
    }
}
