//! Native git subprocess backend
//!
//! Implements git operations using native git command-line interface.
//! This approach leverages git's built-in optimizations (fsmonitor, untracked cache)
//! for superior performance on large repositories.
//!
//! ## Performance Characteristics
//!
//! - Tiny repos (~200 files): ~31ms total (git + overhead)
//! - Small repos (~1.4k files): ~32ms
//! - Medium repos (~28k files): ~40ms
//! - Large repos (~91k files): ~45ms
//!
//! Performance benefits from git's native optimizations:
//! - `core.fsmonitor`: Only checks files reported as changed
//! - `core.untrackedcache`: Caches untracked file information (auto-enabled)
//! - Optimized index traversal in C
//! - Single subprocess call using `git status --porcelain=v2 --branch`
//!
//! ## Reliability
//!
//! Uses `git status --porcelain=v2 --branch` for machine-readable output that
//! combines branch, ahead/behind, and file status in a single call. This format
//! has been stable since Git 2.11.0 (2016) and is designed for scripting.

use super::{CompleteStatus, RebaseProgress, RepositoryState, RepositoryStatus};
use crate::Error;
use crate::Result;
use crate::config::types::GitTimeout;
use crate::profiling::Timer;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

/// Git output parsing logic.
pub mod parser;

// Import types from the parser module
use parser::{BranchHead, V2ParseResult};

/// Cache of repositories where we've already checked/enabled untracked cache
static UNTRACKED_CACHE_CHECKED: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

/// Cap on `.git/config` reads: can grow with many remotes/submodules/includes,
/// so this is sized generously above a typical config rather than pinned to a
/// single-line file's size class.
const GIT_CONFIG_READ_CAP: usize = 65_536;

/// Cap on `.git` gitdir-pointer file reads: a single-line pointer file by
/// construction.
const GITDIR_POINTER_READ_CAP: usize = 4_096;

/// Cap on `rebase-merge/msgnum` reads: a single bare integer by construction.
const MSGNUM_READ_CAP: usize = 4_096;

/// Cap on `rebase-merge/git-rebase-todo` reads: an interactive rebase of
/// hundreds of commits can produce a todo file well beyond a few KB.
const REBASE_TODO_READ_CAP: usize = 65_536;

/// Whether `content` (a `.git/config` file's contents) has
/// `untrackedcache = true` set.
///
/// A substring heuristic, not a real INI parse — deliberately so; see
/// [`NativeGitBackend::is_untracked_cache_enabled`]'s doc for why a false
/// negative here is harmless (the caller just retries enabling it).
#[must_use]
fn config_has_untracked_cache_true(content: &str) -> bool {
    content.to_lowercase().contains("untrackedcache = true")
}

/// Parse a `msgnum` file's contents (a bare integer, the current step number
/// in an `am`-style rebase).
#[must_use]
fn parse_msgnum(contents: &str) -> Option<u32> {
    contents.trim().parse().ok()
}

/// Count the non-blank, non-comment lines in a `git-rebase-todo` file's
/// contents.
///
/// Each such line is one remaining rebase step: `pick`, `exec`, `label`,
/// etc. — see [`NativeGitBackend::count_remaining_todo_steps`]'s doc for the
/// full command set.
#[must_use]
fn count_todo_lines(contents: &str) -> Option<u32> {
    let count = contents
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with('#')
        })
        .count();
    u32::try_from(count).ok()
}

/// Native git backend using subprocess calls
pub struct NativeGitBackend;

impl NativeGitBackend {
    /// Create a new native git backend
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Execute a git command and capture output.
    ///
    /// Uses the default `git.timeout_seconds` config timeout to prevent hung
    /// repos (network mounts, dead FUSE filesystems) from wedging agent threads.
    ///
    /// # Errors
    ///
    /// Returns an error if the git command fails to execute, returns a non-zero
    /// exit code, or exceeds the timeout.
    fn run_git(repo_path: &Path, args: &[&str]) -> Result<String> {
        let timeout = Duration::from_secs(GitTimeout::DEFAULT);
        run_git_with_timeout(repo_path, args, timeout)
    }

    /// Execute a git command that may fail (for optional operations).
    fn run_git_optional(repo_path: &Path, args: &[&str]) -> Option<String> {
        let timeout = Duration::from_secs(GitTimeout::DEFAULT);
        run_git_with_timeout(repo_path, args, timeout).ok()
    }

    /// Check if untracked cache is enabled for this repository
    fn is_untracked_cache_enabled(repo_path: &Path) -> bool {
        // Optimization: Check local config directly to avoid subprocess overhead (~3ms)
        let Some(git_dir) = Self::resolve_git_dir_opt(repo_path) else {
            return false;
        };

        let config_path = git_dir.join("config");
        let Ok(content) = crate::fs_util::read_small_file(&config_path, GIT_CONFIG_READ_CAP) else {
            return false;
        };

        // If not found locally, return false.
        // We skip checking global config via `git config` to save the subprocess cost.
        // If it's enabled globally but not locally, we will "re-enable" it locally,
        // which is harmless and makes future checks fast.
        config_has_untracked_cache_true(&content)
    }

    /// Enable untracked cache for this repository
    ///
    /// This provides massive performance improvements on repeated git status calls
    /// by caching directory mtimes and untracked file lists.
    ///
    /// Based on gitstatus research: 9.5x faster on hot runs (295ms -> 30ms)
    ///
    /// # Errors
    ///
    /// Returns an error if the git config command fails
    fn enable_untracked_cache(repo_path: &Path) -> Result<()> {
        // Enable untracked cache
        Self::run_git(repo_path, &["config", "core.untrackedCache", "true"])?;

        // Initialize the cache (git update-index --untracked-cache)
        let _ = Self::run_git_optional(repo_path, &["update-index", "--untracked-cache"]);

        Ok(())
    }

    /// Ensure untracked cache is enabled for this repo (cached check)
    ///
    /// Only checks once per repo per process to avoid overhead.
    fn ensure_untracked_cache(repo_path: &Path) {
        let _t = Timer::new("native_ensure_untracked_cache");
        let repo_path_buf = repo_path.to_path_buf();

        // Check if we've already verified this repo (fast path)
        let already_checked = UNTRACKED_CACHE_CHECKED.lock().is_ok_and(|mut guard| {
            guard
                .get_or_insert_with(HashSet::new)
                .contains(&repo_path_buf)
        });

        if already_checked {
            return; // Already checked this repo
        }

        // Check and enable if needed (slow operations, no lock)
        if !Self::is_untracked_cache_enabled(repo_path) {
            // Silently try to enable it - don't fail if it doesn't work
            let _ = Self::enable_untracked_cache(repo_path);
        }

        // Re-acquire lock to mark as checked
        if let Ok(mut guard) = UNTRACKED_CACHE_CHECKED.lock()
            && let Some(set) = guard.as_mut()
        {
            set.insert(repo_path_buf);
        }
    }

    /// List tracked files that also match the repo's ignore rules — the
    /// "force-added" set produced by `git add -f` on an otherwise-ignored path.
    ///
    /// Runs `git ls-files --cached -i --exclude-standard -z`, which reports
    /// only files that are BOTH in the index (`--cached`) AND ignored (`-i`
    /// with `--exclude-standard`, i.e. `.gitignore` + `.git/info/exclude` +
    /// global excludes). Untracked-but-ignored files (ordinary build output)
    /// are absent because they are not cached; tracked-not-ignored files are
    /// absent because `-i` filters them out. The result is normally empty or
    /// tiny (force-adds are rare).
    ///
    /// Paths are returned absolute and canonicalized best-effort (join
    /// `repo_root`, then `canonicalize`, falling back to the joined path when
    /// the file cannot be resolved). `-z` yields NUL-separated records so paths
    /// containing spaces or other special bytes survive intact. A git failure
    /// (missing repo, timeout) yields an empty vec — callers treat that as "no
    /// force-adds", which fails open to the pre-existing suppress behavior.
    #[must_use]
    pub fn tracked_ignored_files(repo_root: &Path) -> Vec<PathBuf> {
        let Some(output) = Self::run_git_optional(
            repo_root,
            &["ls-files", "--cached", "-i", "--exclude-standard", "-z"],
        ) else {
            return Vec::new();
        };

        output
            .split('\0')
            .filter(|entry| !entry.is_empty())
            .map(|rel| {
                let joined = repo_root.join(rel);
                fs::canonicalize(&joined).unwrap_or(joined)
            })
            .collect()
    }

    /// Check if sparse checkout is enabled in the repository
    #[must_use]
    pub fn is_sparse_checkout(repo_path: &Path) -> bool {
        // Check if core.sparseCheckout is enabled
        Self::run_git_optional(repo_path, &["config", "--get", "core.sparseCheckout"])
            .is_some_and(|v| v == "true")
    }

    /// Resolve the actual git directory (handling .git files for submodules/worktrees)
    /// Returns Option<PathBuf> for when we need to gracefully handle non-existence
    fn resolve_git_dir_opt(repo_root: &Path) -> Option<PathBuf> {
        let git_item = repo_root.join(".git");

        // If .git is a directory, that's it
        if git_item.is_dir() {
            return Some(git_item);
        }

        // If .git is a file (submodule or worktree), parse the link. No
        // canonicalization: callers only check the resolved path for existence.
        if git_item.is_file()
            && let Ok(content) = crate::fs_util::read_small_file(&git_item, GITDIR_POINTER_READ_CAP)
        {
            return crate::git::parse_gitdir_link(&content, repo_root);
        }
        None
    }

    /// Resolve the actual git directory (handling .git files for submodules/worktrees)
    ///
    /// Falls back to the `.git` path itself when it is neither a directory nor
    /// a parseable pointer file — exactly the cases [`Self::resolve_git_dir_opt`]
    /// reports as `None`.
    fn resolve_git_dir(repo_root: &Path) -> PathBuf {
        let git_item = repo_root.join(".git");
        Self::resolve_git_dir_opt(repo_root).unwrap_or(git_item)
    }

    /// Resolve a parsed [`BranchHead`] into its final `(branch, detached)`
    /// pair, running a subprocess when the header didn't carry the answer
    /// directly.
    ///
    /// For a detached HEAD, `branch` is the bare short commit hash with no
    /// `HEAD@` prefix — callers must consult `detached` rather than sniffing
    /// the branch string.
    fn resolve_branch_head(
        branch_head: BranchHead,
        repo_path: &Path,
        timeout: Duration,
    ) -> (String, bool) {
        const FALLBACK: &str = "main";

        match branch_head {
            BranchHead::Named(name) => (name, false),
            BranchHead::Detached => {
                // Detached HEAD - get short commit hash
                let hash =
                    run_git_with_timeout(repo_path, &["rev-parse", "--short", "HEAD"], timeout)
                        .unwrap_or_else(|_| FALLBACK.to_owned());
                (hash, true)
            }
            BranchHead::Initial => {
                // Initial/empty repository - try to get branch name
                let branch =
                    run_git_with_timeout(repo_path, &["symbolic-ref", "--short", "HEAD"], timeout)
                        .ok()
                        .unwrap_or_else(|| FALLBACK.to_owned());
                (branch, false)
            }
        }
    }

    /// Get complete git status using porcelain v2 format (combines branch, ahead/behind, and file status)
    ///
    /// # Errors
    ///
    /// Returns an error if git status command fails
    fn get_status_v2(
        repo_path: &Path,
        paths: Option<&[PathBuf]>,
        max_ahead_behind: usize,
        timeout: Duration,
    ) -> Result<V2ParseResult> {
        let _t = Timer::new("native_get_status_v2");
        // Ensure untracked cache is enabled for performance
        Self::ensure_untracked_cache(repo_path);

        // Build git status command with v2 format. `-z` makes Git emit
        // NUL-terminated records with unquoted, unescaped paths so renames,
        // spaces, tabs, and non-ASCII bytes survive parsing intact.
        let mut args = vec!["status", "--porcelain=v2", "--branch", "-z"];

        // Add specific paths if provided. `:(literal)` stops git reading a
        // leading `:`, `*`, `?` or `[` in a filename as pathspec magic or a
        // glob (#713).
        let path_strings: Vec<String>;
        if let Some(p) = paths {
            args.push("--");
            path_strings = p
                .iter()
                .map(|pb| format!(":(literal){}", pb.display()))
                .collect();
            for path_str in &path_strings {
                args.push(path_str);
            }
        }

        let output = run_git_with_timeout(repo_path, &args, timeout)?;

        let records = parser::parse_v2_records(&output, max_ahead_behind);
        let (branch, detached) = Self::resolve_branch_head(records.branch, repo_path, timeout);

        Ok(V2ParseResult {
            branch,
            detached,
            ahead: records.ahead,
            behind: records.behind,
            ahead_capped: records.ahead_capped,
            behind_capped: records.behind_capped,
            staged: records.staged,
            unstaged: records.unstaged,
            untracked: records.untracked,
            conflicts: records.conflicts,
            files: records.files,
        })
    }

    /// Get the repository state (merging, rebasing, etc.)
    #[must_use]
    pub fn get_repo_state(repo_path: &Path) -> Option<RepositoryState> {
        let git_dir = Self::resolve_git_dir(repo_path);

        // Check for various in-progress states
        if git_dir.join("MERGE_HEAD").exists() {
            return Some(RepositoryState::Merging);
        }

        let rebase_apply = git_dir.join("rebase-apply");
        if rebase_apply.join("applying").exists() {
            // `git am` and `git rebase --apply` share rebase-apply, but only
            // `am` writes this marker; check it first so an in-progress `am`
            // isn't reported as a generic rebase.
            return Some(RepositoryState::Applying);
        }

        if git_dir.join("rebase-merge").exists() || rebase_apply.exists() {
            return Some(RepositoryState::Rebasing);
        }

        if git_dir.join("CHERRY_PICK_HEAD").exists() {
            return Some(RepositoryState::CherryPicking);
        }

        if git_dir.join("REVERT_HEAD").exists() {
            return Some(RepositoryState::Reverting);
        }

        if git_dir.join("BISECT_LOG").exists() {
            return Some(RepositoryState::Bisecting);
        }

        None
    }

    /// Count stashed changesets via `git stash list -z` (NUL-delimited entries).
    ///
    /// Gated by `stash_enabled` so disabling stash capture skips the
    /// subprocess entirely rather than paying for a call whose result is
    /// discarded. Uses the same hardened `run_git_optional` path (own process
    /// group, timeout) as every other git call here.
    fn get_stash_count(repo_path: &Path, stash_enabled: bool, timeout: Duration) -> u32 {
        if !stash_enabled {
            return 0;
        }

        run_git_with_timeout(repo_path, &["stash", "list", "-z"], timeout)
            .ok()
            .map_or(0, |output| {
                u32::try_from(output.split('\0').filter(|entry| !entry.is_empty()).count())
                    .unwrap_or(u32::MAX)
            })
    }

    /// Read interactive-rebase progress from `.git/rebase-merge/msgnum` (the
    /// completed-step count) and the current `.git/rebase-merge/git-rebase-todo`
    /// (the remaining-step count).
    ///
    /// Only interactive rebases (`git rebase -i`) track a step counter this
    /// way; am-style rebases (`.git/rebase-apply`) have no equivalent file, so
    /// this returns `None` for them. Plain file reads, no subprocess.
    ///
    /// `total` deliberately does NOT read `rebase-merge/end` (#481): git
    /// writes `end` once, when the rebase starts, and never rewrites it when
    /// `git rebase --edit-todo` changes the plan, so it goes stale the moment
    /// a step is added or removed mid-rebase. Git's own `git status` doesn't
    /// use it either — its "N remaining commands" figure comes from re-reading
    /// the current `git-rebase-todo` file. Counting that file's remaining
    /// non-comment, non-blank lines and adding it to `msgnum` (which git *does*
    /// keep accurate) matches what `git status` reports and stays correct
    /// immediately after an `--edit-todo`, without waiting for `--continue` to
    /// rewrite `end`.
    fn get_rebase_progress(repo_path: &Path) -> Option<RebaseProgress> {
        let rebase_merge = Self::resolve_git_dir(repo_path).join("rebase-merge");
        if !rebase_merge.exists() {
            return None;
        }

        let msgnum_contents =
            crate::fs_util::read_small_file(&rebase_merge.join("msgnum"), MSGNUM_READ_CAP).ok()?;
        let step = parse_msgnum(&msgnum_contents)?;
        let remaining = Self::count_remaining_todo_steps(&rebase_merge)?;
        let total = step.checked_add(remaining)?;

        Some(RebaseProgress { step, total })
    }

    /// Count the non-comment, non-blank lines in the current
    /// `rebase-merge/git-rebase-todo` file: each one is a step git has not
    /// executed yet.
    ///
    /// Every instruction line — `pick`, `exec`, `label`, `reset`, `break`, …
    /// — advances `msgnum` by exactly 1 when git executes it; git does not
    /// special-case any instruction type in its own step accounting, so a
    /// uniform line count matches `git status`'s "remaining commands" figure
    /// (#481).
    fn count_remaining_todo_steps(rebase_merge: &Path) -> Option<u32> {
        let todo = crate::fs_util::read_small_file(
            &rebase_merge.join("git-rebase-todo"),
            REBASE_TODO_READ_CAP,
        )
        .ok()?;
        count_todo_lines(&todo)
    }

    /// Run the status-v2, repo-state, and stash-count captures in parallel
    /// threads and join their results.
    ///
    /// Split out of [`Self::get_complete_status`] purely to keep that
    /// function's line count down; the three captures are independent git
    /// reads (one subprocess each, plus repo-state's plain file probes) with
    /// nothing to coordinate beyond joining.
    ///
    /// # Errors
    ///
    /// Returns an error if a capture thread panics.
    #[expect(
        clippy::type_complexity,
        reason = "still a 3-tuple of independent per-thread results after de-tupling V2ParseResult out of it; a named wrapper would only be used here and by its one caller, which already destructures it immediately"
    )]
    fn run_parallel_captures(
        repo_root: &Path,
        paths: Option<&[PathBuf]>,
        max_ahead_behind: usize,
        stash_enabled: bool,
        timeout: Duration,
    ) -> Result<(Result<V2ParseResult>, Option<RepositoryState>, u32)> {
        let (status_v2_join_result, state_join_result, stash_join_result) =
            std::thread::scope(|s| {
                let status_v2_handle =
                    s.spawn(|| Self::get_status_v2(repo_root, paths, max_ahead_behind, timeout));
                let state_handle = s.spawn(|| Self::get_repo_state(repo_root));
                let stash_handle =
                    s.spawn(|| Self::get_stash_count(repo_root, stash_enabled, timeout));

                let status_v2_result = status_v2_handle.join().map_err(|panic| {
                    Error::process(
                        "git_status_v2".to_owned(),
                        format!("status worker panicked: {panic:?}"),
                    )
                });
                let state_result = state_handle.join().map_err(|panic| {
                    Error::process(
                        "git_state".to_owned(),
                        format!("state worker panicked: {panic:?}"),
                    )
                });
                let stash_result = stash_handle.join().map_err(|panic| {
                    Error::process(
                        "git_stash".to_owned(),
                        format!("stash worker panicked: {panic:?}"),
                    )
                });

                (status_v2_result, state_result, stash_result)
            });

        Ok((
            status_v2_join_result?,
            state_join_result?,
            stash_join_result?,
        ))
    }

    /// Get complete git status including branch, ahead/behind, and file counts
    ///
    /// This is the main entry point for getting git status. It uses git status --porcelain=v2
    /// to combine branch, ahead/behind, and file status into a single subprocess call,
    /// reducing overhead. Repo state, stash count, and rebase progress are captured
    /// alongside it (state and stash on separate threads; rebase progress is a cheap
    /// direct file read with no subprocess).
    ///
    /// # Errors
    ///
    /// Returns an error if any git commands fail, or if any of the parallel
    /// capture threads panic (the panic is converted to an [`Error`] rather
    /// than propagated).
    pub fn get_complete_status(
        path: &Path,
        paths: Option<&[PathBuf]>,
        max_ahead_behind: usize,
        stash_enabled: bool,
        timeout: Duration,
    ) -> Result<Option<CompleteStatus>> {
        let _t = Timer::new("native_get_complete_status");
        let Some(repo_root) = crate::git::find_repo_root(path) else {
            return Ok(None);
        };

        let (status_v2_res, state_opt, stash_count) = Self::run_parallel_captures(
            &repo_root,
            paths,
            max_ahead_behind,
            stash_enabled,
            timeout,
        )?;

        let v2 = status_v2_res?;

        let state = resolve_overall_state(
            v2.conflicts,
            state_opt,
            v2.staged,
            v2.unstaged,
            v2.untracked,
        );
        let rebase_progress = Self::get_rebase_progress(&repo_root);

        let status = RepositoryStatus {
            branch: v2.branch,
            ahead: v2.ahead,
            behind: v2.behind,
            ahead_capped: v2.ahead_capped,
            behind_capped: v2.behind_capped,
            staged: v2.staged,
            unstaged: v2.unstaged,
            untracked: v2.untracked,
            conflicts: v2.conflicts,
            state,
            stash_count,
            detached: v2.detached,
            rebase_progress,
        };

        Ok(Some(CompleteStatus {
            status,
            files: v2.files,
        }))
    }
}

impl Default for NativeGitBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// Collapse conflict count, optional in-progress repo state, and dirty counts into
/// a single `RepositoryState` using the canonical precedence:
/// conflicts > in-progress operation > dirty > clean.
#[must_use]
pub(crate) const fn resolve_overall_state(
    conflicts: u32,
    special_state: Option<RepositoryState>,
    staged: u32,
    unstaged: u32,
    untracked: u32,
) -> RepositoryState {
    if conflicts > 0 {
        RepositoryState::Conflicts
    } else if let Some(state) = special_state {
        state
    } else if staged > 0 || unstaged > 0 || untracked > 0 {
        RepositoryState::Dirty
    } else {
        RepositoryState::Clean
    }
}

/// Execute a git command with a hard timeout.
///
/// Spawns the child in its own process group (on Unix) via
/// [`crate::process::spawn_in_own_process_group`] and waits for it via
/// [`crate::process::wait_with_timeout`], so the wait returns promptly
/// regardless of the child's behavior even when it ignores `SIGTERM`. This
/// prevents a hung repo (network mount, dead FUSE, stuck `core.fsmonitor`
/// hook) or a descendant holding the output pipes open from wedging a
/// blocking worker indefinitely. Shares its implementation with the
/// language-version hardening in [`crate::language::version`] (#151, #177,
/// #590).
///
/// # Errors
///
/// Returns an error if the process cannot be spawned, its I/O fails, it exits
/// with a non-zero status, or the timeout expires.
fn run_git_with_timeout(repo_path: &Path, args: &[&str], timeout: Duration) -> Result<String> {
    let mut command = Command::new("git");
    command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .arg("-C")
        .arg(repo_path)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::process::spawn_in_own_process_group(&mut command);

    let child = command
        .spawn()
        .map_err(|e| Error::git(format!("Failed to execute git: {e}")))?;

    let output = crate::process::wait_with_timeout(child, timeout).map_err(|e| match e {
        crate::process::WaitError::Io(io_err) => Error::git(format!("Git I/O error: {io_err}")),
        crate::process::WaitError::TimedOut(t) => {
            Error::git(format!("git command timed out after {t:?}"))
        }
        crate::process::WaitError::WorkerDisconnected => {
            Error::git("git command worker exited without a result".to_owned())
        }
    })?;
    git_output_to_string(&output)
}

/// Convert a raw `process::Output` to a trimmed string, propagating git errors.
///
/// # Errors
///
/// Returns an error if the git process exited with a non-zero status.
fn git_output_to_string(output: &std::process::Output) -> Result<String> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::git(format!("Git command failed: {}", stderr.trim())));
    }
    // Don't trim leading whitespace — git status porcelain uses it.
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_owned())
}

#[cfg(test)]
#[allow(clippy::missing_panics_doc)]
mod tests {
    use super::*;
    use crate::git::RebaseProgress;
    use tempfile::TempDir;

    #[test]
    fn config_has_untracked_cache_true_table() {
        let cases: &[(&str, bool)] = &[
            ("[core]\n\tuntrackedcache = true\n", true),
            ("[core]\n\tUNTRACKEDCACHE = TRUE\n", true),
            ("[core]\n\tuntrackedCache = true\n", true),
            ("[core]\n\tuntrackedcache = false\n", false),
            ("[core]\n\tfsmonitor = true\n", false),
            ("", false),
        ];
        for (input, expected) in cases {
            assert_eq!(
                config_has_untracked_cache_true(input),
                *expected,
                "input: {input:?}"
            );
        }
    }

    #[test]
    fn parse_msgnum_table() {
        let cases: &[(&str, Option<u32>)] = &[
            ("3", Some(3)),
            ("3\n", Some(3)),
            ("  3  \n", Some(3)),
            ("", None),
            ("not-a-number", None),
            ("-1", None),
        ];
        for (input, expected) in cases {
            assert_eq!(parse_msgnum(input), *expected, "input: {input:?}");
        }
    }

    #[test]
    fn count_todo_lines_table() {
        let cases: &[(&str, Option<u32>)] = &[
            ("pick abc123 msg\npick def456 msg\n", Some(2)),
            ("pick abc123 msg\n# comment\n\nexec make test\n", Some(2)),
            ("# only comments\n\n", Some(0)),
            ("", Some(0)),
        ];
        for (input, expected) in cases {
            assert_eq!(count_todo_lines(input), *expected, "input: {input:?}");
        }
    }

    /// Minimal git repo fixture for this module's unit tests. The richer
    /// `TestRepo` fixture under `tests/fixtures/` lives in the integration-test
    /// binary and isn't reachable from `src/`-side unit tests.
    ///
    /// # Panics
    ///
    /// Panics if git is not available or repository setup fails.
    fn init_test_repo() -> (TempDir, PathBuf) {
        let dir = TempDir::new().expect("create temp dir");
        let path = fs::canonicalize(dir.path()).expect("canonicalize temp dir");

        let run = |args: &[&str]| {
            let output = Command::new("git")
                .args(args)
                .current_dir(&path)
                .output()
                .unwrap_or_else(|e| panic!("failed to run git {args:?}: {e}"));
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };

        run(&["init"]);
        run(&["symbolic-ref", "HEAD", "refs/heads/main"]);
        run(&["config", "user.name", "Test User"]);
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "commit.gpgsign", "false"]);
        fs::write(path.join("file.txt"), "content\n").expect("write file.txt");
        run(&["add", "."]);
        run(&["commit", "-m", "init"]);

        (dir, path)
    }

    /// Write an executable POSIX-`sh` script to `dir/name`, for use as a
    /// `GIT_SEQUENCE_EDITOR`.
    ///
    /// Plain shell text manipulation (read/case/echo), not `sed -i`: this
    /// machine's `sed` is BSD, CI also runs Linux with GNU `sed`, and the two
    /// disagree on the `-i` flag's argument requirement. A hand-rolled POSIX
    /// loop behaves identically on both (#481).
    #[cfg(unix)]
    fn write_sequence_editor_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let script = dir.join(name);
        fs::write(&script, format!("#!/bin/sh\n{body}\n")).expect("write sequence-editor script");
        let mut perms = fs::metadata(&script)
            .expect("stat sequence-editor script")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).expect("chmod sequence-editor script executable");
        script
    }

    #[test]
    fn detached_head_sets_flag_not_branch_prefix() {
        let (_dir, path) = init_test_repo();

        let rev_parse = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&path)
            .output()
            .expect("rev-parse HEAD");
        let sha = String::from_utf8_lossy(&rev_parse.stdout).trim().to_owned();

        let checkout = Command::new("git")
            .args(["checkout", &sha])
            .current_dir(&path)
            .output()
            .expect("checkout detached commit");
        assert!(checkout.status.success());

        let CompleteStatus { status, .. } = NativeGitBackend::get_complete_status(
            &path,
            None,
            0,
            true,
            Duration::from_secs(GitTimeout::DEFAULT),
        )
        .expect("get_complete_status should succeed")
        .expect("repo should be detected");

        assert!(status.detached, "expected detached flag to be set");
        assert!(
            !status.branch.starts_with("HEAD@"),
            "branch must not carry the legacy HEAD@ prefix, got {:?}",
            status.branch
        );
    }

    /// Regression for #571: `git.timeout_seconds` bounded only the IPC
    /// handler's wait for a reply, never the git subprocess itself.
    ///
    /// That is because `run_git`/`run_git_optional` hard-coded
    /// `GitTimeout::DEFAULT` no matter what `get_complete_status` was called
    /// with. Proves the `timeout` parameter genuinely reaches and bounds the
    /// real subprocess: an
    /// unreasonably short timeout (well under the several-millisecond
    /// fork/exec cost of any real `git` invocation) must time out, and a
    /// generous timeout against the identical repo must still succeed --
    /// showing the parameter changes behavior in both directions rather than
    /// always erroring or always being ignored.
    #[test]
    fn get_complete_status_respects_configured_timeout() {
        let (_dir, path) = init_test_repo();

        let short_start = std::time::Instant::now();
        let short_timeout_result =
            NativeGitBackend::get_complete_status(&path, None, 0, true, Duration::from_nanos(1));
        let short_elapsed = short_start.elapsed();
        assert!(
            short_timeout_result.is_err(),
            "a 1ns timeout must bound the real git subprocess and time out, got {short_timeout_result:?} after {short_elapsed:?}"
        );

        let generous_start = std::time::Instant::now();
        let generous_timeout_result =
            NativeGitBackend::get_complete_status(&path, None, 0, true, Duration::from_secs(5));
        let generous_elapsed = generous_start.elapsed();
        assert!(
            generous_timeout_result.is_ok(),
            "a 5s timeout on the same repo must still succeed, got {generous_timeout_result:?} after {generous_elapsed:?}"
        );
    }

    #[test]
    fn stash_count_reflects_stash_list() {
        let (_dir, path) = init_test_repo();

        for i in 0_u32..2 {
            fs::write(path.join("file.txt"), format!("content {i}\n"))
                .expect("modify tracked file");
            let stash = Command::new("git")
                .args(["stash", "push", "-m", &format!("stash {i}")])
                .current_dir(&path)
                .output()
                .expect("stash push");
            assert!(stash.status.success());
        }

        let CompleteStatus { status, .. } = NativeGitBackend::get_complete_status(
            &path,
            None,
            0,
            true,
            Duration::from_secs(GitTimeout::DEFAULT),
        )
        .expect("get_complete_status should succeed")
        .expect("repo should be detected");

        assert_eq!(status.stash_count, 2);
    }

    #[test]
    fn stash_count_is_zero_when_stash_disabled() {
        let (_dir, path) = init_test_repo();

        fs::write(path.join("file.txt"), "modified\n").expect("modify tracked file");
        let stash = Command::new("git")
            .args(["stash", "push", "-m", "stash"])
            .current_dir(&path)
            .output()
            .expect("stash push");
        assert!(stash.status.success());

        let CompleteStatus { status, .. } = NativeGitBackend::get_complete_status(
            &path,
            None,
            0,
            false,
            Duration::from_secs(GitTimeout::DEFAULT),
        )
        .expect("get_complete_status should succeed")
        .expect("repo should be detected");

        assert_eq!(
            status.stash_count, 0,
            "stash capture must be skipped when stash_enabled is false"
        );
    }

    #[test]
    fn rebase_progress_present_for_interactive_rebase() {
        let (_dir, path) = init_test_repo();

        let rebase_merge = path.join(".git").join("rebase-merge");
        fs::create_dir_all(&rebase_merge).expect("create rebase-merge dir");
        fs::write(rebase_merge.join("msgnum"), "3\n").expect("write msgnum");
        fs::write(rebase_merge.join("end"), "5\n").expect("write end");
        // Realistic fixture (#481): a real interactive rebase always has
        // `git-rebase-todo` present whenever `msgnum` is, since `total` is now
        // computed from `msgnum` + the todo's remaining non-comment lines
        // rather than the (git-stale-prone) `end` file. 2 remaining picks
        // here reproduces the same total (3 + 2 = 5) this test asserted
        // before the fix.
        fs::write(
            rebase_merge.join("git-rebase-todo"),
            "pick aaaaaaa commit four\npick bbbbbbb commit five\n",
        )
        .expect("write git-rebase-todo");

        let CompleteStatus { status, .. } = NativeGitBackend::get_complete_status(
            &path,
            None,
            0,
            true,
            Duration::from_secs(GitTimeout::DEFAULT),
        )
        .expect("get_complete_status should succeed")
        .expect("repo should be detected");

        assert_eq!(
            status.rebase_progress,
            Some(RebaseProgress { step: 3, total: 5 })
        );
    }

    /// Regression for #481: `total` must reflect the CURRENT
    /// `git-rebase-todo` after `git rebase --edit-todo`, not the stale
    /// `rebase-merge/end` git never rewrites when the plan changes.
    ///
    /// Drives a real interactive rebase and a real `--edit-todo`: 5 commits,
    /// stop after the first (`msgnum` = 1, `end` = 5), then edit the todo to
    /// drop 2 of the remaining 4 `pick`s. `git status` itself reports "1
    /// command done" / "2 remaining commands" for this exact state, so the
    /// correct total is 3 — proving the fix tracks the edited plan rather
    /// than the stale `end` value of 5.
    /// Add commits 2..=5 on top of `init_test_repo()`'s existing "init" commit
    /// (commit 1 of 5), so the repo has a 5-step rebase plan to work with.
    #[cfg(unix)]
    fn add_numbered_commits(path: &Path, range: std::ops::RangeInclusive<u32>) {
        for i in range {
            fs::write(path.join("file.txt"), format!("content {i}\n"))
                .expect("modify tracked file");
            let commit = Command::new("git")
                .args(["commit", "-am", &format!("commit {i}")])
                .current_dir(path)
                .output()
                .expect("commit");
            assert!(
                commit.status.success(),
                "commit {i} failed: {}",
                String::from_utf8_lossy(&commit.stderr)
            );
        }
    }

    /// Start `git rebase -i --root`, changing the first `pick` to `edit` so the rebase stops
    /// after step 1.
    ///
    /// Asserts the setup precondition the regression test depends on (`msgnum` = 1, `end` = 5)
    /// and returns the `.git/rebase-merge` dir.
    #[cfg(unix)]
    fn start_rebase_paused_at_first_edit(dir: &TempDir, path: &Path) -> PathBuf {
        let edit_first = write_sequence_editor_script(
            dir.path(),
            "edit_first_pick.sh",
            r#"todo="$1"
tmp="$todo.tmp"
: > "$tmp"
changed=0
while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
        pick\ *)
            if [ "$changed" -eq 0 ]; then
                echo "edit ${line#pick }" >> "$tmp"
                changed=1
            else
                echo "$line" >> "$tmp"
            fi
            ;;
        *)
            echo "$line" >> "$tmp"
            ;;
    esac
done < "$todo"
mv "$tmp" "$todo""#,
        );

        let status = Command::new("git")
            .args(["rebase", "-i", "--root"])
            .env("GIT_SEQUENCE_EDITOR", &edit_first)
            .current_dir(path)
            .status()
            .expect("start interactive rebase, paused at the first edit");
        assert!(
            status.success(),
            "rebase -i --root should stop cleanly at the first edit"
        );

        let rebase_merge = path.join(".git").join("rebase-merge");
        let msgnum_before = fs::read_to_string(rebase_merge.join("msgnum"))
            .expect("read msgnum")
            .trim()
            .to_owned();
        assert_eq!(
            msgnum_before, "1",
            "setup: rebase should have stopped after step 1"
        );
        let end_before = fs::read_to_string(rebase_merge.join("end"))
            .expect("read end")
            .trim()
            .to_owned();
        assert_eq!(
            end_before, "5",
            "setup: end should reflect the original 5-commit plan"
        );

        rebase_merge
    }

    /// Run `git rebase --edit-todo`, dropping 2 of the remaining 4 `pick`
    /// lines. This is the regression itself: `end` stays untouched at "5"
    /// while the plan actually has fewer steps left.
    #[cfg(unix)]
    fn edit_todo_dropping_two_picks(dir: &TempDir, path: &Path) {
        let drop_two = write_sequence_editor_script(
            dir.path(),
            "drop_two_picks.sh",
            r#"todo="$1"
tmp="$todo.tmp"
: > "$tmp"
kept=0
while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
        pick\ *)
            kept=$((kept + 1))
            if [ "$kept" -le 2 ]; then
                echo "$line" >> "$tmp"
            fi
            ;;
        *)
            echo "$line" >> "$tmp"
            ;;
    esac
done < "$todo"
mv "$tmp" "$todo""#,
        );

        let status = Command::new("git")
            .args(["rebase", "--edit-todo"])
            .env("GIT_SEQUENCE_EDITOR", &drop_two)
            .current_dir(path)
            .status()
            .expect("edit the rebase todo");
        assert!(status.success(), "rebase --edit-todo should succeed");
    }

    #[test]
    #[cfg(unix)]
    fn rebase_progress_total_reflects_edited_todo_after_edit_todo() {
        let (dir, path) = init_test_repo();
        add_numbered_commits(&path, 2_u32..=5);
        let rebase_merge = start_rebase_paused_at_first_edit(&dir, &path);
        edit_todo_dropping_two_picks(&dir, &path);

        let end_after = fs::read_to_string(rebase_merge.join("end"))
            .expect("read end")
            .trim()
            .to_owned();
        assert_eq!(
            end_after, "5",
            "self-check: git must not rewrite `end` on --edit-todo, or this test proves nothing"
        );

        let progress = NativeGitBackend::get_rebase_progress(&path)
            .expect("get_rebase_progress should report progress for the paused rebase");

        assert_ne!(
            progress.total, 5,
            "total must not equal the stale pre-edit end value"
        );
        assert_eq!(
            progress,
            RebaseProgress { step: 1, total: 3 },
            "total must reflect the edited plan: 1 done + 2 remaining picks"
        );

        // Clean up so the tempdir doesn't leak a stuck rebase state.
        let abort = Command::new("git")
            .args(["rebase", "--abort"])
            .current_dir(&path)
            .output()
            .expect("abort rebase");
        assert!(
            abort.status.success(),
            "rebase --abort should succeed: {}",
            String::from_utf8_lossy(&abort.stderr)
        );
    }

    #[test]
    fn rebase_progress_none_for_non_interactive_rebase() {
        let (_dir, path) = init_test_repo();

        let rebase_apply = path.join(".git").join("rebase-apply");
        fs::create_dir_all(&rebase_apply).expect("create rebase-apply dir");
        fs::write(rebase_apply.join("next"), "1\n").expect("write next");
        fs::write(rebase_apply.join("last"), "3\n").expect("write last");

        let CompleteStatus { status, .. } = NativeGitBackend::get_complete_status(
            &path,
            None,
            0,
            true,
            Duration::from_secs(GitTimeout::DEFAULT),
        )
        .expect("get_complete_status should succeed")
        .expect("repo should be detected");

        assert!(
            status.rebase_progress.is_none(),
            "am-style rebase-apply must not populate step/total"
        );
    }

    #[test]
    fn get_repo_state_returns_applying_for_conflicted_git_am() {
        let (_dir, path) = init_test_repo();

        // Realistic conflicted `git am` layout (captured from a real `git am
        // -3` run, git 2.55.0): rebase-apply carries the same `next`/`last`
        // counters as a non-interactive `rebase --apply`, so detection must
        // key off the `applying` marker rather than the directory's mere
        // presence.
        let rebase_apply = path.join(".git").join("rebase-apply");
        fs::create_dir_all(&rebase_apply).expect("create rebase-apply dir");
        for name in [
            "0001",
            "abort-safety",
            "apply-opt",
            "applying",
            "author-script",
            "final-commit",
            "info",
            "keep",
            "last",
            "messageid",
            "msg",
            "next",
            "patch",
            "quiet",
            "quoted-cr",
            "scissors",
            "sign",
            "threeway",
            "utf8",
        ] {
            fs::write(rebase_apply.join(name), "").expect("write am marker file");
        }

        assert_eq!(
            NativeGitBackend::get_repo_state(&path),
            Some(RepositoryState::Applying)
        );

        // AC2: am-style state detection must not regress interactive-rebase
        // progress reporting, which reads rebase-merge/{msgnum,end} and is
        // untouched by the applying-marker check.
        let CompleteStatus { status, .. } = NativeGitBackend::get_complete_status(
            &path,
            None,
            0,
            true,
            Duration::from_secs(GitTimeout::DEFAULT),
        )
        .expect("get_complete_status should succeed")
        .expect("repo should be detected");
        assert!(
            status.rebase_progress.is_none(),
            "am-style rebase-apply must not populate step/total"
        );
    }

    #[test]
    fn get_repo_state_returns_rebasing_for_apply_backend_rebase_without_applying_marker() {
        let (_dir, path) = init_test_repo();

        // `git rebase --apply` shares rebase-apply with `git am` but never
        // writes the `applying` marker; it must keep resolving to Rebasing
        // rather than being misread as an am in progress.
        let rebase_apply = path.join(".git").join("rebase-apply");
        fs::create_dir_all(&rebase_apply).expect("create rebase-apply dir");
        fs::write(rebase_apply.join("next"), "1\n").expect("write next");
        fs::write(rebase_apply.join("last"), "3\n").expect("write last");

        assert_eq!(
            NativeGitBackend::get_repo_state(&path),
            Some(RepositoryState::Rebasing)
        );
    }

    #[test]
    fn detached_stashed_and_mid_rebase_are_captured_simultaneously() {
        let (_dir, path) = init_test_repo();

        // Stash a change.
        fs::write(path.join("file.txt"), "modified\n").expect("modify tracked file");
        let stash = Command::new("git")
            .args(["stash", "push", "-m", "stash"])
            .current_dir(&path)
            .output()
            .expect("stash push");
        assert!(stash.status.success());

        // Detach HEAD.
        let rev_parse = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&path)
            .output()
            .expect("rev-parse HEAD");
        let sha = String::from_utf8_lossy(&rev_parse.stdout).trim().to_owned();
        let checkout = Command::new("git")
            .args(["checkout", &sha])
            .current_dir(&path)
            .output()
            .expect("checkout detached commit");
        assert!(checkout.status.success());

        // Simulate a mid-rebase interactive rebase.
        let rebase_merge = path.join(".git").join("rebase-merge");
        fs::create_dir_all(&rebase_merge).expect("create rebase-merge dir");
        fs::write(rebase_merge.join("msgnum"), "1\n").expect("write msgnum");
        fs::write(rebase_merge.join("end"), "2\n").expect("write end");
        // Realistic fixture (#481): see the matching comment in
        // `rebase_progress_present_for_interactive_rebase`. 1 remaining pick
        // reproduces the same total (1 + 1 = 2) this test asserted before.
        fs::write(
            rebase_merge.join("git-rebase-todo"),
            "pick aaaaaaa commit two\n",
        )
        .expect("write git-rebase-todo");

        let CompleteStatus { status, .. } = NativeGitBackend::get_complete_status(
            &path,
            None,
            0,
            true,
            Duration::from_secs(GitTimeout::DEFAULT),
        )
        .expect("get_complete_status should succeed")
        .expect("repo should be detected");

        assert!(status.detached, "expected detached flag to be set");
        assert_eq!(status.stash_count, 1);
        assert_eq!(
            status.rebase_progress,
            Some(RebaseProgress { step: 1, total: 2 })
        );
    }

    #[test]
    fn test_native_backend_creation() {
        let backend = NativeGitBackend::new();
        // Just verify it compiles and creates
        let _ = backend;
    }

    #[test]
    fn test_find_repo_root_nonexistent() {
        // A hardcoded Unix-style absolute path isn't reliably "nonexistent" on
        // Windows (a leading `/` resolves to the current drive's root there);
        // anchor under the real temp dir so this is nonexistent on every platform.
        let nonexistent = std::env::temp_dir().join("gpy-nonexistent-test-dir-abc123xyz");
        let result = crate::git::find_repo_root(&nonexistent);
        // Should return None for nonexistent path, not panic
        assert!(result.is_none());
    }

    #[test]
    fn resolve_overall_state_conflicts_win() {
        // Conflicts take precedence over everything else
        let state = resolve_overall_state(1, Some(RepositoryState::Merging), 5, 3, 2);
        assert_eq!(state, RepositoryState::Conflicts);
    }

    #[test]
    fn resolve_overall_state_special_state_wins_over_dirty() {
        // In-progress operation wins over dirty counts when no conflicts
        let state = resolve_overall_state(0, Some(RepositoryState::Merging), 5, 3, 2);
        assert_eq!(state, RepositoryState::Merging);
    }

    #[test]
    fn resolve_overall_state_dirty() {
        // Any changed file makes the repo dirty when no conflicts/special state
        let staged_dirty = resolve_overall_state(0, None, 1, 0, 0);
        assert_eq!(staged_dirty, RepositoryState::Dirty);
        let unstaged_dirty = resolve_overall_state(0, None, 0, 1, 0);
        assert_eq!(unstaged_dirty, RepositoryState::Dirty);
        let untracked_dirty = resolve_overall_state(0, None, 0, 0, 1);
        assert_eq!(untracked_dirty, RepositoryState::Dirty);
    }

    #[test]
    fn resolve_overall_state_clean() {
        let state = resolve_overall_state(0, None, 0, 0, 0);
        assert_eq!(state, RepositoryState::Clean);
    }

    /// Regression for #180 and #181.
    ///
    /// A NUL-delimited `-z` status containing a staged rename with spaces must
    /// key the file map by the destination path (skipping the trailing
    /// original-path record), and ahead/behind counts must be clamped to
    /// `max_ahead_behind`.
    #[test]
    fn parse_v2_output_handles_rename_and_clamps_counts() {
        let output = concat!(
            "# branch.head main\0",
            "# branch.ab +3 -1\0",
            "2 R. N... 100644 100644 100644 abc abc R100 new name.txt\0",
            "old name.txt\0",
            "? untracked.txt\0",
        );

        let result = parser::parse_v2_records(output, 2);

        assert_eq!(result.branch, BranchHead::Named("main".to_owned()));
        // #181: ahead 3 > max 2 -> clamped and capped; behind 1 <= max -> verbatim.
        assert_eq!(result.ahead, 2);
        assert!(result.ahead_capped);
        assert_eq!(result.behind, 1);
        assert!(!result.behind_capped);

        // #180: the rename is one staged change, keyed by its destination.
        assert_eq!(result.staged, 1);
        assert_eq!(result.untracked, 1);
        assert!(result.files.contains_key(Path::new("new name.txt")));
        assert!(
            !result.files.contains_key(Path::new("old name.txt")),
            "the original-path record must be consumed, not stored as an entry"
        );
        assert!(
            !result
                .files
                .contains_key(Path::new("new name.txt old name.txt")),
            "destination and original paths must not be concatenated"
        );
        let renamed = result
            .files
            .get(Path::new("new name.txt"))
            .expect("renamed file present");
        assert!(renamed.staged);
    }
}
