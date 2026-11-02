//! Guard against git's stale untracked cache (#1204).
//!
//! Git validates a cached directory listing by whole-second mtime. A listing
//! cached in second `S` that misses a change made later in `S` is carried
//! forward by an index write in a later second, and every later `git status`
//! replays it until the directory changes in a new second. A rescan with the
//! cache on cannot see through that, so the agent checks once per index write:
//! it runs the status again with the cache bypassed in parallel with the normal
//! one, and compares the untracked files. On a mismatch it reports the bypassed
//! result and has git drop the stale cache, which also repairs the user's own
//! `git status`.

use super::parser::{self, V2ParseState};
use super::{NativeGitBackend, UNTRACKED_CACHE_SETUP, run_git_env_until, run_git_until};
use crate::Error;
use crate::Result;
use crate::cache::bounded::NATIVE_GIT_CONFIG_CACHE_CAPACITY;
use crate::cache::ttl_map::TtlMap;
use crate::git::FileStatus;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Verified state is dropped after this long so a repository is checked again.
const STATE_TTL: Duration = Duration::from_hours(24);

/// After a failed or timed-out check, how long before trying again.
const BACKOFF_TTL: Duration = Duration::from_secs(60);

/// Longest a check claims a repository; a crashed check frees it after this.
const FLIGHT_TTL: Duration = Duration::from_secs(60);

/// Mismatches on one index state that trigger a repair without the
/// stale-directory signature.
const REPAIR_AFTER_MISMATCHES: u8 = 2;

/// Identity of one index write. Git replaces the index through a lock-file
/// rename, so a write changes the mtime, or the size where the clock is coarse.
#[derive(Clone, Copy, PartialEq, Eq)]
struct IndexKey {
    /// Index size in bytes.
    len: u64,
    /// Index modification time.
    modified: SystemTime,
}

/// Index state whose untracked cache matched an uncached walk.
static VERIFIED: TtlMap<PathBuf, IndexKey> =
    TtlMap::with_ttl(NATIVE_GIT_CONFIG_CACHE_CAPACITY, STATE_TTL);

/// Consecutive mismatches seen on one index state.
static MISMATCHES: TtlMap<PathBuf, (IndexKey, u8)> =
    TtlMap::with_ttl(NATIVE_GIT_CONFIG_CACHE_CAPACITY, STATE_TTL);

/// Repositories whose last check failed.
static BACKOFF: TtlMap<PathBuf, ()> =
    TtlMap::with_ttl(NATIVE_GIT_CONFIG_CACHE_CAPACITY, BACKOFF_TTL);

/// Repositories with a check running, so concurrent refreshes check once.
static IN_FLIGHT: TtlMap<PathBuf, ()> =
    TtlMap::with_ttl(NATIVE_GIT_CONFIG_CACHE_CAPACITY, FLIGHT_TTL);

/// A check that has claimed its repository and holds the index state it
/// verifies.
pub(super) struct Probe {
    /// Repository being checked.
    repo: PathBuf,
    /// Index state read before the status ran.
    key: IndexKey,
    /// Git directory holding that index.
    git_dir: PathBuf,
}

impl Drop for Probe {
    fn drop(&mut self) {
        IN_FLIGHT.remove(&self.repo);
    }
}

fn index_key(git_dir: &Path) -> Option<IndexKey> {
    let meta = fs::metadata(git_dir.join("index")).ok()?;
    Some(IndexKey {
        len: meta.len(),
        modified: meta.modified().ok()?,
    })
}

/// Whether git is mid-operation, when rewriting the index would collide with it.
fn operation_in_progress(git_dir: &Path) -> bool {
    git_dir.join("index.lock").exists()
        || [
            "rebase-merge",
            "rebase-apply",
            "MERGE_HEAD",
            "CHERRY_PICK_HEAD",
            "REVERT_HEAD",
            "sequencer",
        ]
        .iter()
        .any(|name| git_dir.join(name).exists())
}

/// Whether the index carries an untracked-cache extension. A path named like
/// the signature gives a false positive, which costs one extra check.
fn index_has_untracked_cache(git_dir: &Path) -> bool {
    fs::read(git_dir.join("index")).is_ok_and(|bytes| bytes.windows(4).any(|w| w == b"UNTR"))
}

/// Start a check for `repo` when its index changed since it was last verified.
///
/// Call before the normal status. Returns `None` when the check is not needed
/// or not safe now: outside the agent, an unchanged index, no untracked cache,
/// a git operation in progress, a recent failure, or another check running.
fn begin(repo: &Path) -> Option<Probe> {
    if !UNTRACKED_CACHE_SETUP.load(Ordering::Relaxed) {
        return None;
    }
    let git_dir = NativeGitBackend::resolve_git_dir(repo);
    let key = index_key(&git_dir)?;
    if VERIFIED.get(repo) == Some(key) || BACKOFF.get(repo).is_some() {
        return None;
    }
    if operation_in_progress(&git_dir) {
        return None;
    }
    if !index_has_untracked_cache(&git_dir) {
        VERIFIED.insert(repo.to_path_buf(), key);
        return None;
    }
    if !IN_FLIGHT.insert_if_absent(repo.to_path_buf(), ()) {
        return None;
    }
    Some(Probe {
        repo: repo.to_path_buf(),
        key,
        git_dir,
    })
}

/// The status output with git's untracked cache bypassed, so the untracked
/// files come from a full directory walk.
///
/// # Errors
///
/// Returns an error when git fails or misses `deadline`.
fn bypass_status(repo: &Path, deadline: Instant) -> Result<String> {
    run_git_env_until(
        repo,
        &["status", "--porcelain=v2", "-z"],
        &[("GIT_DISABLE_UNTRACKED_CACHE", "1")],
        deadline,
    )
}

fn untracked_paths(files: &HashMap<PathBuf, FileStatus>) -> HashSet<PathBuf> {
    files
        .iter()
        .filter(|(_, status)| status.untracked)
        .map(|(path, _)| path.clone())
        .collect()
}

/// Seconds since the epoch of `path`'s mtime.
fn mtime_secs(path: &Path) -> Option<u64> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    Some(modified.duration_since(UNIX_EPOCH).ok()?.as_secs())
}

/// Whether a differing entry sits in a directory last modified in an earlier
/// second than the index write: the stale-listing signature. A change made
/// after the index write has a newer mtime.
fn shows_stale_signature(repo: &Path, differing: &HashSet<PathBuf>, index_secs: u64) -> bool {
    differing.iter().any(|path| {
        let text = path.to_string_lossy();
        let is_dir = text.ends_with('/');
        let entry = PathBuf::from(text.trim_end_matches('/'));
        let containing = if is_dir {
            Some(entry.as_path())
        } else {
            entry.parent()
        };
        containing
            .and_then(|rel| mtime_secs(&repo.join(rel)))
            .is_some_and(|secs| secs < index_secs)
    })
}

/// Compare the cached status in `parsed` with the bypassed `bypass` result.
///
/// Equal: records the index state as verified. Different: replaces the
/// untracked entries of `parsed` with the bypassed ones and, on the stale
/// signature or a repeated mismatch, has git drop the cache. A mismatch or a
/// failed check never records the state, so the next index write, including
/// the repair's own, is checked again.
fn reconcile(probe: &Probe, parsed: &mut V2ParseState, bypass: Result<String>, deadline: Instant) {
    let Ok(output) = bypass else {
        BACKOFF.insert(probe.repo.clone(), ());
        return;
    };
    let truth = parser::parse_v2_files(&output);
    let cached = untracked_paths(&parsed.files);
    let real = untracked_paths(&truth);
    let differing: HashSet<PathBuf> = cached.symmetric_difference(&real).cloned().collect();
    if differing.is_empty() {
        VERIFIED.insert(probe.repo.clone(), probe.key);
        MISMATCHES.remove(&probe.repo);
        return;
    }

    for path in cached.difference(&real) {
        if let Some(file) = parsed.files.get_mut(path) {
            file.untracked = false;
            if file.is_clean() {
                parsed.files.remove(path);
            }
        }
    }
    for path in real.difference(&cached) {
        parsed.process_untracked(path.clone());
    }

    let mismatches = MISMATCHES
        .get(&probe.repo)
        .filter(|(key, _)| *key == probe.key)
        .map_or(1, |(_, count)| count.saturating_add(1));
    MISMATCHES.insert(probe.repo.clone(), (probe.key, mismatches));
    let index_secs = probe
        .key
        .modified
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    if mismatches >= REPAIR_AFTER_MISMATCHES
        || shows_stale_signature(&probe.repo, &differing, index_secs)
    {
        repair(probe, deadline);
    }
}

/// Have git drop the stale cache and write a fresh empty one, in one index
/// write. Skipped when a git operation started since the check began.
fn repair(probe: &Probe, deadline: Instant) {
    if operation_in_progress(&probe.git_dir) {
        return;
    }
    let repaired = run_git_until(
        &probe.repo,
        &[
            "-c",
            "core.untrackedCache=false",
            "update-index",
            "--untracked-cache",
        ],
        deadline,
    );
    if repaired.is_err() {
        BACKOFF.insert(probe.repo.clone(), ());
    }
}

/// Run `git status --porcelain=v2 --branch -z` for `repo` and parse it, first
/// verifying the untracked cache when the index changed since the last check.
///
/// # Errors
///
/// Returns an error if the status command fails.
pub(super) fn status(
    repo: &Path,
    max_ahead_behind: usize,
    deadline: Instant,
) -> Result<V2ParseState> {
    // Before the status, so an index write landing during it is not mistaken
    // for one this status verified.
    let probe = begin(repo);

    // `-z`: NUL-terminated records with unescaped paths.
    let (output, check) = std::thread::scope(|scope| {
        let check_handle = probe
            .as_ref()
            .map(|_| scope.spawn(|| bypass_status(repo, deadline)));
        let cached = run_git_until(
            repo,
            &["status", "--porcelain=v2", "--branch", "-z"],
            deadline,
        );
        let check_result = check_handle.map(|handle| {
            handle
                .join()
                .unwrap_or_else(|_| Err(Error::git("untracked-cache check panicked".to_owned())))
        });
        (cached, check_result)
    });

    let mut parsed = parser::parse_v2_records(&output?, max_ahead_behind);
    if let (Some(held), Some(uncached)) = (probe.as_ref(), check) {
        reconcile(held, &mut parsed, uncached, deadline);
    }
    Ok(parsed)
}
