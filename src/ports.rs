use std::io;
use std::path::{Path, PathBuf};

use crate::domain::manifest::SecretRef;

/// What sits at a path, without following a final symlink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Missing,
    File,
    Dir,
    Symlink(PathBuf),
}

pub trait FileSystem {
    /// Like `lstat`: a symlink is reported as such, never followed.
    fn kind(&self, path: &Path) -> io::Result<Kind>;
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
    fn read_to_string(&self, path: &Path) -> io::Result<String>;
    /// Entries of a directory as full paths, sorted. A missing directory is empty.
    fn list_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>>;
    /// Writes through a symlink to its target, creates parent directories and
    /// keeps the existing file's permissions.
    fn write(&self, path: &Path, content: &str) -> io::Result<()>;
    fn symlink(&self, target: &Path, link: &Path) -> io::Result<()>;
    fn remove_link(&self, path: &Path) -> io::Result<()>;
    /// Moves a file, directory or link, creating the destination's parents.
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn copy_file(&self, from: &Path, to: &Path) -> io::Result<()>;
    /// Fully resolved path of something that exists, `None` otherwise.
    fn canonicalize(&self, path: &Path) -> Option<PathBuf>;
}

pub trait SecretStore {
    fn get(&self, reference: &SecretRef) -> anyhow::Result<String>;
}
