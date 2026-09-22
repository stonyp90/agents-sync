//! In-memory filesystem with real symlink semantics, for fast and
//! deterministic tests of the use cases.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::{self, ErrorKind};
use std::path::{Component, Path, PathBuf};

use crate::ports::{FileSystem, Kind};

const MAX_LINK_DEPTH: usize = 40;

#[derive(Debug, Clone)]
enum Node {
    File(Vec<u8>),
    Dir,
    Link(PathBuf),
}

#[derive(Debug, Default)]
pub struct MemFs {
    nodes: RefCell<BTreeMap<PathBuf, Node>>,
}

fn error(kind: ErrorKind, path: &Path) -> io::Error {
    io::Error::new(kind, path.display().to_string())
}

impl MemFs {
    pub fn with_file(self, path: impl AsRef<Path>, content: &str) -> Self {
        self.write(path.as_ref(), content).expect("with_file");
        self
    }

    pub fn with_link(self, link: impl AsRef<Path>, target: impl AsRef<Path>) -> Self {
        self.symlink(target.as_ref(), link.as_ref())
            .expect("with_link");
        self
    }

    pub fn with_dir(self, path: impl AsRef<Path>) -> Self {
        self.mkdirs(path.as_ref());
        self
    }

    fn mkdirs(&self, path: &Path) {
        let mut nodes = self.nodes.borrow_mut();
        for ancestor in path.ancestors().filter(|a| !a.as_os_str().is_empty()) {
            nodes.entry(ancestor.to_path_buf()).or_insert(Node::Dir);
        }
    }

    fn node(&self, path: &Path) -> Option<Node> {
        self.nodes.borrow().get(path).cloned()
    }

    /// Resolves symlinks in every component, and in the last one too when
    /// `follow_last` is set.
    fn resolve(&self, path: &Path, follow_last: bool, depth: usize) -> io::Result<PathBuf> {
        if depth > MAX_LINK_DEPTH {
            return Err(error(ErrorKind::Other, path));
        }
        let components: Vec<Component> = path.components().collect();
        let mut current = PathBuf::from("/");
        for (i, component) in components.iter().enumerate() {
            match component {
                Component::RootDir | Component::Prefix(_) => {
                    current = PathBuf::from("/");
                    continue;
                }
                Component::CurDir => continue,
                Component::ParentDir => {
                    current.pop();
                    continue;
                }
                Component::Normal(name) => current.push(name),
            }
            if i + 1 == components.len() && !follow_last {
                break;
            }
            if let Some(Node::Link(target)) = self.node(&current) {
                let joined = if target.is_absolute() {
                    target
                } else {
                    current.parent().unwrap_or(Path::new("/")).join(target)
                };
                current = self.resolve(&joined, true, depth + 1)?;
            }
        }
        Ok(current)
    }
}

impl FileSystem for MemFs {
    fn kind(&self, path: &Path) -> io::Result<Kind> {
        let resolved = self.resolve(path, false, 0)?;
        Ok(match self.node(&resolved) {
            None => Kind::Missing,
            Some(Node::File(_)) => Kind::File,
            Some(Node::Dir) => Kind::Dir,
            Some(Node::Link(target)) => Kind::Symlink(target),
        })
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        match self.node(&self.resolve(path, true, 0)?) {
            Some(Node::File(content)) => Ok(content),
            Some(_) => Err(error(ErrorKind::InvalidInput, path)),
            None => Err(error(ErrorKind::NotFound, path)),
        }
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        String::from_utf8(self.read(path)?).map_err(|_| error(ErrorKind::InvalidData, path))
    }

    fn list_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
        let resolved = self.resolve(path, true, 0)?;
        if !matches!(self.node(&resolved), Some(Node::Dir)) {
            return Ok(Vec::new());
        }
        Ok(self
            .nodes
            .borrow()
            .keys()
            .filter(|k| k.parent() == Some(resolved.as_path()))
            .filter_map(|k| k.file_name().map(|name| path.join(name)))
            .collect())
    }

    fn write(&self, path: &Path, content: &str) -> io::Result<()> {
        let resolved = self.resolve(path, true, 0)?;
        if matches!(self.node(&resolved), Some(Node::Dir)) {
            return Err(error(ErrorKind::InvalidInput, path));
        }
        if let Some(parent) = resolved.parent() {
            self.mkdirs(parent);
        }
        self.nodes
            .borrow_mut()
            .insert(resolved, Node::File(content.as_bytes().to_vec()));
        Ok(())
    }

    fn symlink(&self, target: &Path, link: &Path) -> io::Result<()> {
        let resolved = self.resolve(link, false, 0)?;
        if self.node(&resolved).is_some() {
            return Err(error(ErrorKind::AlreadyExists, link));
        }
        if let Some(parent) = resolved.parent() {
            self.mkdirs(parent);
        }
        self.nodes
            .borrow_mut()
            .insert(resolved, Node::Link(target.to_path_buf()));
        Ok(())
    }

    fn remove_link(&self, path: &Path) -> io::Result<()> {
        let resolved = self.resolve(path, false, 0)?;
        match self.node(&resolved) {
            Some(Node::Link(_)) => {
                self.nodes.borrow_mut().remove(&resolved);
                Ok(())
            }
            _ => Err(error(ErrorKind::InvalidInput, path)),
        }
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let source = self.resolve(from, false, 0)?;
        let dest = self.resolve(to, false, 0)?;
        if self.node(&source).is_none() {
            return Err(error(ErrorKind::NotFound, from));
        }
        if self.node(&dest).is_some() {
            return Err(error(ErrorKind::AlreadyExists, to));
        }
        if let Some(parent) = dest.parent() {
            self.mkdirs(parent);
        }
        let mut nodes = self.nodes.borrow_mut();
        let moved: Vec<PathBuf> = nodes
            .keys()
            .filter(|k| k.starts_with(&source))
            .cloned()
            .collect();
        for key in moved {
            let node = nodes.remove(&key).expect("key listed above");
            let rest = key.strip_prefix(&source).expect("filtered by prefix");
            let new_key = if rest.as_os_str().is_empty() {
                dest.clone()
            } else {
                dest.join(rest)
            };
            nodes.insert(new_key, node);
        }
        Ok(())
    }

    fn copy_file(&self, from: &Path, to: &Path) -> io::Result<()> {
        let content = self.read(from)?;
        let resolved = self.resolve(to, true, 0)?;
        if let Some(parent) = resolved.parent() {
            self.mkdirs(parent);
        }
        self.nodes
            .borrow_mut()
            .insert(resolved, Node::File(content));
        Ok(())
    }

    fn canonicalize(&self, path: &Path) -> Option<PathBuf> {
        let resolved = self.resolve(path, true, 0).ok()?;
        self.node(&resolved).map(|_| resolved)
    }
}
