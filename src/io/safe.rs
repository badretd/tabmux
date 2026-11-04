//! Atomic file writes with no-clobber, backup, and fsync semantics.

use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use tempfile::{Builder, NamedTempFile};

use crate::error::{Error, Result};

/// A staged output file that becomes visible only on [`SafeWriter::commit`].
///
/// The temporary file lives in the destination directory so the final rename
/// stays on the same filesystem. If the writer is dropped without committing,
/// the temporary file is removed.
#[derive(Debug)]
pub struct SafeWriter {
    destination: PathBuf,
    temp: Option<NamedTempFile>,
    backup: bool,
}

impl SafeWriter {
    /// Create a staged writer for `destination`.
    pub fn create(destination: &Path, force: bool, backup: bool) -> Result<Self> {
        Self::ensure_writable(destination, force)?;

        let parent = parent_directory(destination);
        fs::create_dir_all(parent)?;

        let temp = Builder::new()
            .prefix(".tabmux-")
            .suffix(".tmp")
            .tempfile_in(parent)?;

        Ok(Self {
            destination: destination.to_path_buf(),
            temp: Some(temp),
            backup,
        })
    }

    /// Check whether an output may be written without creating anything.
    pub fn ensure_writable(destination: &Path, force: bool) -> Result<()> {
        if destination.exists() && !force {
            return Err(Error::data(format!(
                "output already exists: {} (use --force to overwrite)",
                destination.display()
            )));
        }
        Ok(())
    }

    /// The staged file for writers to target.
    pub fn file_mut(&mut self) -> &mut File {
        self.temp
            .as_mut()
            .expect("SafeWriter is only usable before commit")
            .as_file_mut()
    }

    /// Flush, fsync, optionally back up the old file, and atomically rename.
    pub fn commit(mut self) -> Result<()> {
        let temp = self.temp.take().expect("SafeWriter is only committed once");

        temp.as_file().sync_all()?;

        if self.backup && self.destination.exists() {
            let backup = backup_path(&self.destination);
            fs::copy(&self.destination, &backup)?;
        }

        temp.persist(&self.destination)
            .map_err(|error| Error::Io(error.error))?;

        sync_directory(parent_directory(&self.destination))?;
        Ok(())
    }
}

impl Write for SafeWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.file_mut().write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file_mut().flush()
    }
}

fn backup_path(destination: &Path) -> PathBuf {
    let name = destination
        .file_name()
        .and_then(OsStr::to_str)
        .filter(|name| !name.is_empty())
        .unwrap_or("output");

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);

    destination.with_file_name(format!("{name}.{stamp}.bak"))
}

fn parent_directory(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

fn sync_directory(path: &Path) -> Result<()> {
    // Directory fsync is a durability improvement; some filesystems reject it
    // (notably on Windows), so only propagate Unix-style errors we can act on.
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
