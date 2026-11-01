//! Best-effort removal of cache files: the startup age sweep, the restart
//! clear of unrefreshable entries, and one repository's git caches.

use super::key::{CACHE_KEY_CHUNK_CHARS, cache_key_path, split_key_path};
use super::name::parse_cache_file_name;
use crate::debug_log;
use std::path::Path;
use std::time::{Duration, SystemTime};

/// Age (by mtime) after which the startup sweep removes an unused entry.
#[expect(
    clippy::duration_suboptimal_units,
    reason = "from_days/from_hours are unstable (duration_constructors)"
)]
pub(super) const CACHE_ENTRY_MAX_AGE: Duration = Duration::from_secs(14 * 24 * 60 * 60);

/// Age after which a `.tmp-*` file is an orphan from a killed writer.
#[expect(
    clippy::duration_suboptimal_units,
    reason = "from_days/from_hours are unstable (duration_constructors)"
)]
pub(super) const ORPHAN_TMP_MAX_AGE: Duration = Duration::from_secs(60 * 60);

/// Entries of `dir`; `None`, logged unless `dir` is missing, if it cannot be read.
fn read_entries(dir: &Path) -> Option<std::fs::ReadDir> {
    match std::fs::read_dir(dir) {
        Ok(entries) => Some(entries),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            debug_log!("cache", "Could not scan {}: {e}", dir.display());
            None
        }
    }
}

/// Whether `entry` is a chunk directory ([`CACHE_KEY_CHUNK_CHARS`] chars, not a symlink).
fn is_chunk_dir(entry: &std::fs::DirEntry) -> bool {
    entry.file_type().is_ok_and(|t| t.is_dir())
        && entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.chars().count() == CACHE_KEY_CHUNK_CHARS)
}

/// Call `visit` on every entry under `dir` except chunk directories, which
/// are descended into and removed once the visits leave them empty.
fn walk(dir: &Path, visit: &mut impl FnMut(&std::fs::DirEntry)) {
    let Some(entries) = read_entries(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if is_chunk_dir(&entry) {
            let path = entry.path();
            walk(&path, visit);
            // Fails, harmlessly, unless the walk emptied it.
            let _ = std::fs::remove_dir(&path);
        } else {
            visit(&entry);
        }
    }
}

/// Remove entries unused for [`CACHE_ENTRY_MAX_AGE`] and temp files orphaned
/// for [`ORPHAN_TMP_MAX_AGE`] under `dir`, ignoring errors.
/// Returns `(entries_removed, orphans_removed)`.
pub(super) fn prune_stale_entries(dir: &Path, now: SystemTime) -> (u64, u64) {
    let mut removed_entries = 0_u64;
    let mut removed_orphans = 0_u64;
    walk(dir, &mut |entry| {
        let path = entry.path();
        if !path.is_file() {
            return;
        }
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            return;
        };
        let Ok(age) = now.duration_since(modified) else {
            // Future mtime (clock skew or in-flight write): leave it.
            return;
        };

        let is_orphan_tmp = entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with(".tmp-"));
        let (limit, counter) = if is_orphan_tmp {
            (ORPHAN_TMP_MAX_AGE, &mut removed_orphans)
        } else {
            (CACHE_ENTRY_MAX_AGE, &mut removed_entries)
        };

        if age > limit && std::fs::remove_file(&path).is_ok() {
            *counter = counter.saturating_add(1);
        }
    });
    (removed_entries, removed_orphans)
}

/// Remove every cache file under `dir` that
/// [`is_unrefreshable`](super::name::CacheFileName::is_unrefreshable).
pub(super) fn clear_unrefreshable_files(dir: &Path) {
    walk(dir, &mut |entry| {
        let file_name = entry.file_name();
        if file_name
            .to_str()
            .and_then(parse_cache_file_name)
            .is_some_and(|name| name.is_unrefreshable())
            && let Err(e) = std::fs::remove_file(entry.path())
        {
            debug_log!(
                "cache",
                "Could not remove unrefreshable cache {}: {e}",
                entry.path().display()
            );
        }
    });
}

/// Delete the git cache files of `key` under `cache_dir`. Only the key's own
/// directory is scanned: files below it belong to longer keys.
pub(super) fn remove_git_files(cache_dir: &Path, key: &str) {
    let stem = cache_key_path(key);
    let (dir, last_chunk) = split_key_path(cache_dir, &stem);
    let Some(entries) = read_entries(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        if file_name
            .to_str()
            .and_then(parse_cache_file_name)
            .is_some_and(|name| name.kind.is_git() && name.key == last_chunk)
            && let Err(e) = std::fs::remove_file(entry.path())
        {
            debug_log!(
                "cache",
                "Could not remove git cache {}: {e}",
                entry.path().display()
            );
        }
    }
}
