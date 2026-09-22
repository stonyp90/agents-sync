use std::path::PathBuf;

use serde_json::Value;

use super::target::Target;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyMode {
    /// agents-sync owns the whole value of the key.
    Replace,
    /// The key is an object shared with the tool: agents-sync sets the
    /// entries it owns and prunes the ones it owned before.
    MergeOwned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Toml,
}

/// One fact about how a tool's configuration should look.
#[derive(Debug, Clone)]
pub enum Desired {
    Link {
        target: Target,
        path: PathBuf,
        to: PathBuf,
    },
    /// Links in `dir` that point into `root` but are no longer desired get removed.
    ManagedLinks {
        target: Target,
        dir: PathBuf,
        root: PathBuf,
    },
    Key {
        target: Target,
        file: PathBuf,
        format: Format,
        key: String,
        value: Value,
        mode: KeyMode,
    },
    /// A file generated whole by agents-sync.
    File {
        target: Target,
        path: PathBuf,
        content: String,
    },
    /// Previously generated files under `dir` that are no longer desired get removed.
    ManagedFiles { target: Target, dir: PathBuf },
}

impl Desired {
    /// Several tools can ask for the same link (Codex and Copilot share
    /// `~/.agents/skills`); only the first request is kept.
    pub fn dedupe_key(&self) -> Option<(u8, PathBuf)> {
        match self {
            Desired::Link { path, .. } => Some((0, path.clone())),
            Desired::ManagedLinks { dir, .. } => Some((1, dir.clone())),
            _ => None,
        }
    }
}
