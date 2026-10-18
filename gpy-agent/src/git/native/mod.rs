//! Native git subprocess backend built on `git status --porcelain=v2 --branch -z`.
//!
//! Shelling out lets git use its own optimizations (fsmonitor, untracked cache).

use super::{CompleteStatus, RebaseProgress, RepositoryState, RepositoryStatus};
use crate::Error;
use crate::Result;
use crate::config::types::GitTimeout;
use crate::profiling::Timer;
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Git output parsing logic.
pub mod parser;

use parser::{BranchHead, V2ParseResult};

/// Repos already checked for `core.untrackedCache` this process.
static UNTRACKED_CACHE_CHECKED: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

/// Per-repository `core.ignorecase`, read once.
static IGNORE_CASE: Mutex<Option<HashMap<PathBuf, bool>>> = Mutex::new(None);

/// Cap on `.git` gitdir-pointer file reads.
const GITDIR_POINTER_READ_CAP: usize = 4_096;

const MSGNUM_READ_CAP: usize = 4_096;

/// Cap on `rebase-merge/git-rebase-todo` reads; long interactive rebases exceed a few KB.
const REBASE_TODO_READ_CAP: usize = 65_536;

/// Parse a `msgnum` file (current step of an interactive rebase).
#[must_use]
fn parse_msgnum(contents: &str) -> Option<u32> {
    contents.trim().parse().ok()
}

/// Count non-blank, non-comment lines (remaining steps) in `git-rebase-todo`.
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

/// Cap on `sequencer/todo` reads.
const SEQUENCER_TODO_READ_CAP: usize = 65_536;

/// Classify a `sequencer/todo` file by its first non-blank, non-comment
/// line, mirroring git's `sequencer_get_last_command`.
#[must_use]
fn parse_sequencer_command(contents: &str) -> Option<RepositoryState> {
    let line = contents
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))?;
    match line.split_whitespace().next()? {
        "pick" | "p" => Some(RepositoryState::CherryPicking),
        "revert" => Some(RepositoryState::Reverting),
        _ => None,
    }
}

/// Native git backend using subprocess calls
pub struct NativeGitBackend;

impl NativeGitBackend {
    /// Create a new native git backend
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Execute a git command that may fail (for optional operations).
    fn run_git_optional(repo_path: &Path, args: &[&str], deadline: Instant) -> Option<String> {
        run_git_until(repo_path, args, deadline).ok()
    }

    /// Deadline for a standalone git query outside a status operation.
    fn default_deadline() -> Instant {
        Instant::now() + Duration::from_secs(GitTimeout::DEFAULT)
    }

    /// Whether `core.ignorecase` is true for the repository at `repo_path`,
    /// at any config scope.
    ///
    /// Cached per repository; a failed read counts as `false` and is retried.
    #[must_use]
    pub fn ignores_case(repo_path: &Path) -> bool {
        Self::ignores_case_until(repo_path, Self::default_deadline())
    }

    /// [`Self::ignores_case`], with the uncached read finishing by `deadline`.
    fn ignores_case_until(repo_path: &Path, deadline: Instant) -> bool {
        let cached = IGNORE_CASE
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(|known| known.get(repo_path).copied());
        if let Some(ignore_case) = cached {
            return ignore_case;
        }

        // `--default false` makes an unset key succeed with `false`, so only a
        // genuine failure reaches the `else`.
        let Some(value) = Self::run_git_optional(
            repo_path,
            &[
                "config",
                "--type=bool",
                "--default",
                "false",
                "--get",
                "core.ignorecase",
            ],
            deadline,
        ) else {
            return false;
        };
        let ignore_case = value.trim() == "true";
        IGNORE_CASE
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert_with(HashMap::new)
            .insert(repo_path.to_path_buf(), ignore_case);
        ignore_case
    }

    /// Whether the user has set `core.untrackedCache` at any scope, to any value.
    fn is_untracked_cache_configured(repo_path: &Path, deadline: Instant) -> bool {
        Self::run_git_optional(
            repo_path,
            &["config", "--get", "core.untrackedCache"],
            deadline,
        )
        .is_some_and(|value| !value.trim().is_empty())
    }

    /// Enable and initialize the untracked cache.
    ///
    /// # Errors
    ///
    /// Returns an error if `git config` fails.
    fn enable_untracked_cache(repo_path: &Path, deadline: Instant) -> Result<()> {
        run_git_until(
            repo_path,
            &["config", "core.untrackedCache", "true"],
            deadline,
        )?;

        let _ = Self::run_git_optional(repo_path, &["update-index", "--untracked-cache"], deadline);

        Ok(())
    }

    /// Enable the untracked cache once per repo per process, unless the user set it.
    fn ensure_untracked_cache(repo_path: &Path, deadline: Instant) {
        let _t = Timer::new("native_ensure_untracked_cache");
        let repo_path_buf = repo_path.to_path_buf();

        let already_checked = UNTRACKED_CACHE_CHECKED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert_with(HashSet::new)
            .contains(&repo_path_buf);

        if already_checked {
            return;
        }

        if !Self::is_untracked_cache_configured(repo_path, deadline) {
            let _ = Self::enable_untracked_cache(repo_path, deadline);
        }

        if let Some(set) = UNTRACKED_CACHE_CHECKED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
        {
            set.insert(repo_path_buf);
        }
    }

    /// Tracked files that match ignore rules (force-added with `git add -f`),
    /// as absolute best-effort-canonical paths. Git failure yields an empty vec.
    #[must_use]
    pub fn tracked_ignored_files(repo_root: &Path) -> Vec<PathBuf> {
        let Some(output) = Self::run_git_optional(
            repo_root,
            &["ls-files", "--cached", "-i", "--exclude-standard", "-z"],
            Self::default_deadline(),
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
        Self::run_git_optional(
            repo_path,
            &["config", "--get", "core.sparseCheckout"],
            Self::default_deadline(),
        )
        .is_some_and(|v| v == "true")
    }

    /// Resolve the git directory, following `.git` pointer files.
    fn resolve_git_dir_opt(repo_root: &Path) -> Option<PathBuf> {
        let git_item = repo_root.join(".git");

        if git_item.is_dir() {
            return Some(git_item);
        }

        // No canonicalization: callers only check existence.
        if git_item.is_file()
            && let Ok(content) = crate::fs_util::read_small_file(&git_item, GITDIR_POINTER_READ_CAP)
        {
            return crate::git::parse_gitdir_link(&content, repo_root);
        }
        None
    }

    /// Like [`Self::resolve_git_dir_opt`], falling back to `<repo>/.git`.
    fn resolve_git_dir(repo_root: &Path) -> PathBuf {
        let git_item = repo_root.join(".git");
        Self::resolve_git_dir_opt(repo_root).unwrap_or(git_item)
    }

    /// Resolve a [`BranchHead`] to `(branch, detached)`; detached yields the short hash.
    fn resolve_branch_head(
        branch_head: BranchHead,
        repo_path: &Path,
        deadline: Instant,
    ) -> (String, bool) {
        const FALLBACK: &str = "main";

        match branch_head {
            BranchHead::Named(name) => (name, false),
            BranchHead::Detached => {
                let hash = run_git_until(repo_path, &["rev-parse", "--short", "HEAD"], deadline)
                    .unwrap_or_else(|_| FALLBACK.to_owned());
                (hash, true)
            }
            BranchHead::Initial => {
                let branch =
                    run_git_until(repo_path, &["symbolic-ref", "--short", "HEAD"], deadline)
                        .ok()
                        .unwrap_or_else(|| FALLBACK.to_owned());
                (branch, false)
            }
        }
    }

    /// Get branch, ahead/behind, and file status in one porcelain v2 call.
    ///
    /// # Errors
    ///
    /// Returns an error if git status command fails
    fn get_status_v2(
        repo_path: &Path,
        paths: Option<&[PathBuf]>,
        max_ahead_behind: usize,
        deadline: Instant,
    ) -> Result<V2ParseResult> {
        let _t = Timer::new("native_get_status_v2");
        Self::ensure_untracked_cache(repo_path, deadline);

        // `-z`: NUL-terminated records with unescaped paths.
        let mut args: Vec<OsString> = ["status", "--porcelain=v2", "--branch", "-z"]
            .into_iter()
            .map(OsString::from)
            .collect();

        // `:(literal)` disables pathspec magic/globs in filenames; `icase`
        // matches watcher spelling against index spelling on case-insensitive repos.
        if let Some(p) = paths {
            let magic = if Self::ignores_case_until(repo_path, deadline) {
                ":(literal,icase)"
            } else {
                ":(literal)"
            };
            args.push(OsString::from("--"));
            for pb in p {
                let mut spec = OsString::from(magic);
                spec.push(pb.as_os_str());
                args.push(spec);
            }
        }

        let output = run_git_until(repo_path, &args, deadline)?;

        let records = parser::parse_v2_records(&output, max_ahead_behind);
        let (branch, detached) = Self::resolve_branch_head(records.branch, repo_path, deadline);

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

    /// Get the repository state (merging, rebasing, etc.); any git query
    /// (reftable pseudorefs) finishes by `deadline`.
    #[must_use]
    pub fn get_repo_state(repo_path: &Path, deadline: Instant) -> Option<RepositoryState> {
        let git_dir = Self::resolve_git_dir(repo_path);

        if git_dir.join("MERGE_HEAD").exists() {
            return Some(RepositoryState::Merging);
        }

        let rebase_apply = git_dir.join("rebase-apply");
        if rebase_apply.join("applying").exists() {
            // Only `am` writes this marker; `rebase --apply` shares the dir.
            return Some(RepositoryState::Applying);
        }

        if git_dir.join("rebase-merge").exists() || rebase_apply.exists() {
            return Some(RepositoryState::Rebasing);
        }

        let reftable = git_dir.join("reftable").is_dir();
        if git_dir.join("CHERRY_PICK_HEAD").exists()
            || (reftable && Self::pseudoref_exists(repo_path, "CHERRY_PICK_HEAD", deadline))
        {
            return Some(RepositoryState::CherryPicking);
        }

        if git_dir.join("REVERT_HEAD").exists()
            || (reftable && Self::pseudoref_exists(repo_path, "REVERT_HEAD", deadline))
        {
            return Some(RepositoryState::Reverting);
        }

        // A multi-commit sequence survives a manual `git commit`: the
        // pseudoref is gone but `sequencer/todo` still lists remaining steps.
        if let Ok(todo) = crate::fs_util::read_small_file(
            &git_dir.join("sequencer").join("todo"),
            SEQUENCER_TODO_READ_CAP,
        ) && let Some(state) = parse_sequencer_command(&todo)
        {
            return Some(state);
        }

        if git_dir.join("BISECT_LOG").exists() {
            return Some(RepositoryState::Bisecting);
        }

        None
    }

    /// Whether a pseudoref exists, asking git because reftable repos keep
    /// them in the ref database rather than as loose files.
    fn pseudoref_exists(repo_path: &Path, name: &str, deadline: Instant) -> bool {
        Self::run_git_optional(
            repo_path,
            &["rev-parse", "--verify", "--quiet", name],
            deadline,
        )
        .is_some()
    }

    /// Count stash entries via `git stash list -z`; skipped when disabled.
    fn get_stash_count(repo_path: &Path, stash_enabled: bool, deadline: Instant) -> u32 {
        if !stash_enabled {
            return 0;
        }

        run_git_until(repo_path, &["stash", "list", "-z"], deadline)
            .ok()
            .map_or(0, |output| {
                u32::try_from(output.split('\0').filter(|entry| !entry.is_empty()).count())
                    .unwrap_or(u32::MAX)
            })
    }

    /// Interactive-rebase progress: `msgnum` plus remaining `git-rebase-todo` lines.
    ///
    /// Ignores `rebase-merge/end`: git never rewrites it on `--edit-todo`, so it
    /// goes stale; this matches `git status`'s own count. am-style rebases yield `None`.
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

    /// Count remaining steps in `rebase-merge/git-rebase-todo`; each instruction
    /// advances `msgnum` by exactly 1.
    fn count_remaining_todo_steps(rebase_merge: &Path) -> Option<u32> {
        let todo = crate::fs_util::read_small_file(
            &rebase_merge.join("git-rebase-todo"),
            REBASE_TODO_READ_CAP,
        )
        .ok()?;
        count_todo_lines(&todo)
    }

    /// Run status-v2, repo-state, and stash-count captures in parallel.
    ///
    /// # Errors
    ///
    /// Returns an error if a capture thread panics.
    fn run_parallel_captures(
        repo_root: &Path,
        paths: Option<&[PathBuf]>,
        max_ahead_behind: usize,
        stash_enabled: bool,
        deadline: Instant,
    ) -> Result<(Result<V2ParseResult>, Option<RepositoryState>, u32)> {
        let (status_v2_join_result, state_join_result, stash_join_result) =
            std::thread::scope(|s| {
                let status_v2_handle =
                    s.spawn(|| Self::get_status_v2(repo_root, paths, max_ahead_behind, deadline));
                let state_handle = s.spawn(|| Self::get_repo_state(repo_root, deadline));
                let stash_handle =
                    s.spawn(|| Self::get_stash_count(repo_root, stash_enabled, deadline));

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

    /// Get complete git status: branch, ahead/behind, files, state, stash, rebase progress.
    ///
    /// `timeout` bounds the whole operation: every git subprocess it runs
    /// shares one deadline, so a repository file that blocks git (a FIFO
    /// `.git/HEAD`) costs one timeout, not one per command.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails, the deadline passes, or a capture thread panics.
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

        let deadline = Instant::now() + timeout;
        let (status_v2_res, state_opt, stash_count) = Self::run_parallel_captures(
            &repo_root,
            paths,
            max_ahead_behind,
            stash_enabled,
            deadline,
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

/// Collapse counts and in-progress state with precedence
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

/// Execute a git command in its own process group, killed if it is still
/// running at `deadline`, so a hung repo or a child ignoring `SIGTERM` cannot
/// wedge a worker. A deadline that has already passed spawns nothing.
///
/// # Errors
///
/// Returns an error on spawn/I/O failure, non-zero exit, or a missed deadline.
fn run_git_until<S: AsRef<OsStr>>(
    repo_path: &Path,
    args: &[S],
    deadline: Instant,
) -> Result<String> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(Error::git(
            "git time budget spent before the command started".to_owned(),
        ));
    }

    let mut command = Command::new("git");
    for var in REPO_LOCAL_GIT_ENV {
        command.env_remove(var);
    }
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

    let output = crate::process::wait_with_timeout(child, remaining).map_err(|e| match e {
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

/// Repository-selection env vars never forwarded to git: they override `-C`.
///
/// Mirrors `git rev-parse --local-env-vars` plus `GIT_NAMESPACE`; removing
/// `GIT_CONFIG_COUNT` disables its key/value pairs. User-level settings are kept.
const REPO_LOCAL_GIT_ENV: &[&str] = &[
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_OBJECT_DIRECTORY",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_GRAFT_FILE",
    "GIT_INDEX_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
];

/// Convert `process::Output` to a trimmed string.
///
/// # Errors
///
/// Returns an error if git exited non-zero.
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
mod tests;
