use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use crate::ports::{FileSystem, Kind};

pub struct RealFs;

fn ensure_parent(path: &Path) -> io::Result<()> {
    match path.parent() {
        Some(parent) => fs::create_dir_all(parent),
        None => Ok(()),
    }
}

impl FileSystem for RealFs {
    fn kind(&self, path: &Path) -> io::Result<Kind> {
        match fs::symlink_metadata(path) {
            Ok(meta) if meta.file_type().is_symlink() => Ok(Kind::Symlink(fs::read_link(path)?)),
            Ok(meta) if meta.is_dir() => Ok(Kind::Dir),
            Ok(_) => Ok(Kind::File),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(Kind::Missing),
            Err(e) => Err(e),
        }
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        fs::read(path)
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        fs::read_to_string(path)
    }

    fn list_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut paths = entries
            .map(|entry| entry.map(|e| e.path()))
            .collect::<io::Result<Vec<_>>>()?;
        paths.sort();
        Ok(paths)
    }

    fn write(&self, path: &Path, content: &str) -> io::Result<()> {
        let target = match fs::symlink_metadata(path) {
            Ok(meta) if meta.file_type().is_symlink() => fs::canonicalize(path)?,
            _ => path.to_path_buf(),
        };
        ensure_parent(&target)?;
        let name = target
            .file_name()
            .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "path has no file name"))?;
        let tmp = target.with_file_name(format!(".{}.agents-sync.tmp", name.to_string_lossy()));
        fs::write(&tmp, content)?;
        if let Ok(meta) = fs::metadata(&target) {
            fs::set_permissions(&tmp, meta.permissions())?;
        }
        fs::rename(&tmp, &target)
    }

    fn symlink(&self, target: &Path, link: &Path) -> io::Result<()> {
        ensure_parent(link)?;
        std::os::unix::fs::symlink(target, link)
    }

    fn remove_link(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        ensure_parent(to)?;
        fs::rename(from, to)
    }

    fn copy_file(&self, from: &Path, to: &Path) -> io::Result<()> {
        ensure_parent(to)?;
        fs::copy(from, to).map(|_| ())
    }

    fn canonicalize(&self, path: &Path) -> Option<PathBuf> {
        fs::canonicalize(path).ok()
    }
}
