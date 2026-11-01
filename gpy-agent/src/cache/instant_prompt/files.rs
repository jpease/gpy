//! Cache directories and atomic, owner-only file writes.

use crate::{Error, Result};
use std::io::Write as _;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Per-process temp-file counter so concurrent writers never share a temp path.
fn next_tmp_suffix() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Create `path` owner-only (`0700`); missing parents get the default mode.
/// An existing real directory is restricted too; a symlink is left alone.
/// # Errors
/// Returns an error if a directory cannot be created or inspected.
#[cfg(unix)]
pub(in crate::cache) fn create_private_dir(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => return Err(e),
        Ok(()) | Err(_) => {}
    }
    if std::fs::symlink_metadata(path)?.file_type().is_dir()
        && let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
    {
        crate::warn_log!("cache", "cannot restrict {}: {e}", path.display());
    }
    Ok(())
}

/// Create `path` and its parents.
/// # Errors
/// Returns an error if a directory cannot be created.
#[cfg(not(unix))]
pub(in crate::cache) fn create_private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)
}

/// Create the temp file `path` owner-only (`0600` on unix), replacing a
/// leftover from an earlier attempt once.
fn create_temp_file(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    match options.open(path) {
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = std::fs::remove_file(path);
            options.open(path)
        }
        result => result,
    }
}

/// Write `content` to `dir/name` atomically via a dot-prefixed, owner-only
/// temp file + rename.
/// # Errors
/// Returns an error if the write or rename fails; the temp file is removed best-effort.
pub(in crate::cache) fn write_atomic(dir: &Path, name: &str, content: &str) -> Result<()> {
    let target = dir.join(name);
    // Independent of `name`: deriving it overflowed NAME_MAX for long names.
    let tmp_path = dir.join(format!(".tmp-{}-{}", std::process::id(), next_tmp_suffix()));

    if let Err(e) =
        create_temp_file(&tmp_path).and_then(|mut file| file.write_all(content.as_bytes()))
    {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e.into());
    }

    std::fs::rename(&tmp_path, &target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        Error::from(e)
    })
}

/// Instant-prompt cache directory (`<base>/instant-prompts/`).
/// # Errors
/// Returns an error if the base cache directory cannot be determined.
pub(super) fn get_instant_cache_dir() -> Result<PathBuf> {
    crate::paths::gpy_cache_dir().map(|d| d.join("instant-prompts"))
}
