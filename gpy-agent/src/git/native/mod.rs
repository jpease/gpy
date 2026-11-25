//! Native git subprocess backend built on `git status --porcelain=v2 --branch -z`.
//!
//! Shelling out lets git use its own optimizations (untracked cache). Every
//! command runs with `core.fsmonitor` off, so a repository's own config cannot
//! make a prompt run its fsmonitor hook. That does not reach every command a
//! repository can configure: a repository-local `filter.<driver>.clean` or
//! `filter.<driver>.process` still runs when `git status` compares the content
//! of a file `.gitattributes` assigns to it, and is not detected here.

use super::{
    CompleteStatus, PathspecScan, RebaseProgress, RepositoryState, RepositoryStatus,
    StatusAggregate,
};
use crate::Error;
use crate::Result;
use crate::cache::bounded::NATIVE_GIT_CONFIG_CACHE_CAPACITY;
use crate::cache::ttl_map::TtlMap;
use crate::config::types::GitTimeout;
use crate::profiling::Timer;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Git output parsing logic.
pub mod parser;

use parser::{BranchHead, V2ParseState};

mod sequencer;
use sequencer::{count_todo_steps, parse_msgnum, parse_sequencer_command};

/// One `git status` capture: the resolved head plus the parsed counts and files.
/// `parsed.branch` is consumed by head resolution and is `None`.
struct StatusV2 {
    /// The resolved branch name.
    branch: String,
    /// Whether HEAD is detached.
    detached: bool,
    /// Parser output; its `branch` has been taken.
    parsed: V2ParseState,
}

/// How long a repository's `core.untrackedCache` check is trusted. Fixed, not
/// tied to `language.cache_ttl_hours`.
const UNTRACKED_CACHE_RECHECK: Duration = Duration::from_hours(24);

/// Repos whose `core.untrackedCache` was checked within [`UNTRACKED_CACHE_RECHECK`].
static UNTRACKED_CACHE_CHECKED: TtlMap<PathBuf, ()> =
    TtlMap::with_ttl(NATIVE_GIT_CONFIG_CACHE_CAPACITY, UNTRACKED_CACHE_RECHECK);

/// Per-process switch for the untracked-cache setup; see
/// [`NativeGitBackend::disable_untracked_cache_setup`].
static UNTRACKED_CACHE_SETUP: AtomicBool = AtomicBool::new(true);

/// How long a repository's `core.ignorecase` is trusted before it is re-read.
const IGNORE_CASE_TTL: Duration = Duration::from_mins(1);

/// Per-repository `core.ignorecase`, expiring after [`IGNORE_CASE_TTL`].
static IGNORE_CASE: TtlMap<PathBuf, bool> =
    TtlMap::with_ttl(NATIVE_GIT_CONFIG_CACHE_CAPACITY, IGNORE_CASE_TTL);

/// How long a detached HEAD's short hash is reused for the same commit.
const SHORT_HEAD_TTL: Duration = Duration::from_hours(1);

/// `git rev-parse --short` answers per `(repository, full commit id)`, so a
/// detached HEAD costs one extra spawn per commit, not one per scan.
static SHORT_HEADS: TtlMap<(PathBuf, String), String> =
    TtlMap::with_ttl(NATIVE_GIT_CONFIG_CACHE_CAPACITY, SHORT_HEAD_TTL);

const MSGNUM_READ_CAP: usize = 4_096;

/// Bytes of `rebase-merge/git-rebase-todo` read before the count is given up as
/// unknown; a 1 `MiB` plan is not a rebase.
const REBASE_TODO_READ_CAP: usize = 1 << 20;

/// Cap on `sequencer/todo` reads.
const SEQUENCER_TODO_READ_CAP: usize = 65_536;

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
        run_git_quiet_env_until(repo_path, args, &[], deadline).ok()
    }

    /// Deadline for a standalone git query outside a status operation.
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "now plus the 10 s GitTimeout::DEFAULT cannot overflow Instant"
    )]
    fn default_deadline() -> Instant {
        Instant::now() + Duration::from_secs(GitTimeout::DEFAULT)
    }

    /// Whether `core.ignorecase` is true for the repository at `repo_path`,
    /// at any config scope.
    ///
    /// Cached per repository for [`IGNORE_CASE_TTL`], so a changed config is
    /// picked up; a failed read counts as `false` and is retried.
    #[must_use]
    pub fn ignores_case(repo_path: &Path) -> bool {
        Self::ignores_case_until(repo_path, Self::default_deadline())
    }

    /// [`Self::ignores_case`], with the uncached read finishing by `deadline`.
    fn ignores_case_until(repo_path: &Path, deadline: Instant) -> bool {
        if let Some(ignore_case) = IGNORE_CASE.get(repo_path) {
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
        IGNORE_CASE.insert(repo_path.to_path_buf(), ignore_case);
        ignore_case
    }

    /// Whether the user has set `core.untrackedCache` at any scope, to any
    /// value (`true`, `false` and `keep` all count as intent).
    fn is_untracked_cache_configured(repo_path: &Path, deadline: Instant) -> bool {
        Self::run_git_optional(
            repo_path,
            &["config", "--get", "core.untrackedCache"],
            deadline,
        )
        .is_some_and(|value| !value.trim().is_empty())
    }

    /// Turn the untracked-cache setup off for this process. Short-lived
    /// processes (`gpy-agent oneshot`, the `gpy` CLI) call this once at
    /// start so they never read or touch the repository's untracked-cache
    /// state; only the long-lived agent sets the cache up.
    pub fn disable_untracked_cache_setup() {
        UNTRACKED_CACHE_SETUP.store(false, Ordering::Relaxed);
    }

    /// Add git's untracked cache to the index unless the user set
    /// `core.untrackedCache` at any scope. Git keeps the extension under its
    /// default (`keep`); the repository config is never written.
    ///
    /// Does nothing in a process that called
    /// [`Self::disable_untracked_cache_setup`]. Otherwise runs at most once per
    /// repository per cache window, and exactly once across concurrent callers.
    /// Deferred to a later scan while a git operation is in progress, since
    /// the index write would race it for `index.lock`.
    fn ensure_untracked_cache(repo_path: &Path, deadline: Instant) {
        if !UNTRACKED_CACHE_SETUP.load(Ordering::Relaxed)
            || UNTRACKED_CACHE_CHECKED.get(repo_path).is_some()
        {
            return;
        }

        let _t = Timer::new("native_ensure_untracked_cache");

        if untracked_guard::operation_in_progress(&Self::resolve_git_dir(repo_path))
            || !UNTRACKED_CACHE_CHECKED.insert_if_absent(repo_path.to_path_buf(), ())
        {
            return;
        }

        if !Self::is_untracked_cache_configured(repo_path, deadline) {
            let _ =
                Self::run_git_optional(repo_path, &["update-index", "--untracked-cache"], deadline);
        }
    }

    /// Tracked files that match ignore rules (force-added with `git add -f`),
    /// as absolute best-effort-canonical paths; `Some(empty)` when there are
    /// none. `None` when git fails or times out, so callers do not mistake an
    /// unknown answer for "none".
    #[must_use]
    pub fn tracked_ignored_files(repo_root: &Path) -> Option<Vec<PathBuf>> {
        let output = run_git_quiet_bytes_until(
            repo_root,
            &["ls-files", "--cached", "-i", "--exclude-standard", "-z"],
            &[],
            Self::default_deadline(),
        )
        .ok()?;

        Some(
            output
                .split(|&byte| byte == b'\0')
                .filter(|entry| !entry.is_empty())
                .map(|rel| {
                    let joined = repo_root.join(parser::path_from_bytes(rel));
                    fs::canonicalize(&joined).unwrap_or(joined)
                })
                .collect(),
        )
    }

    /// Git's global ignore file for `repo_root`: `core.excludesFile` (a
    /// relative value is taken against `repo_root`), else git's default
    /// `$XDG_CONFIG_HOME/git/ignore` or `$HOME/.config/git/ignore`. Not
    /// checked for existence; `None` when no candidate can be named.
    #[must_use]
    pub fn global_excludes_file(repo_root: &Path) -> Option<PathBuf> {
        if let Some(value) = Self::run_git_optional(
            repo_root,
            &["config", "--path", "--get", "core.excludesFile"],
            Self::default_deadline(),
        ) && !value.trim().is_empty()
        {
            return Some(repo_root.join(value.trim()));
        }
        let non_empty_var = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
        non_empty_var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| non_empty_var("HOME").map(|home| Path::new(&home).join(".config")))
            .map(|config_home| config_home.join("git").join("ignore"))
    }

    /// The git directory of `repo_root`, falling back to `<repo>/.git`.
    fn resolve_git_dir(repo_root: &Path) -> PathBuf {
        crate::git::resolve_gitdir(repo_root).unwrap_or_else(|| repo_root.join(".git"))
    }

    /// Resolve a [`BranchHead`] to `(branch, detached)`; detached yields the
    /// short hash git itself prints for the status output's commit id (reused
    /// per commit, see [`SHORT_HEADS`]), or its first 7 characters when that
    /// command cannot run.
    ///
    /// # Errors
    ///
    /// Returns an error for a detached HEAD whose commit cannot be named.
    fn resolve_branch_head(
        branch_head: BranchHead,
        oid: Option<&str>,
        repo_path: &Path,
        deadline: Instant,
    ) -> Result<(String, bool)> {
        match branch_head {
            BranchHead::Named(name) => Ok((name, false)),
            BranchHead::Detached => {
                let memo_key = oid.map(|full| (repo_path.to_path_buf(), full.to_owned()));
                if let Some(hash) = memo_key.as_ref().and_then(|key| SHORT_HEADS.get(key)) {
                    return Ok((hash, true));
                }
                // The status's own commit id, not `HEAD`, so a HEAD moved since
                // the status cannot pair one commit with another's hash.
                if let Ok(hash) = run_git_quiet_env_until(
                    repo_path,
                    &["rev-parse", "--short", oid.unwrap_or("HEAD")],
                    &[],
                    deadline,
                ) {
                    if let Some(key) = memo_key {
                        SHORT_HEADS.insert(key, hash.clone());
                    }
                    return Ok((hash, true));
                }
                oid.and_then(|full| full.get(..7))
                    .map(|prefix| (prefix.to_owned(), true))
                    .ok_or_else(|| Error::git("cannot name the detached HEAD commit".to_owned()))
            }
        }
    }

    /// Get branch, ahead/behind, and file status in one porcelain v2 call.
    ///
    /// # Errors
    ///
    /// Returns an error if git status command fails or prints no branch header.
    fn get_status_v2(
        repo_path: &Path,
        max_ahead_behind: usize,
        deadline: Instant,
    ) -> Result<StatusV2> {
        let _t = Timer::new("native_get_status_v2");
        Self::ensure_untracked_cache(repo_path, deadline);

        let mut parsed = untracked_guard::status(repo_path, max_ahead_behind, deadline)?;
        let Some(branch_head) = parsed.branch.take() else {
            return Err(Error::git("git status printed no branch header".to_owned()));
        };
        let (branch, detached) =
            Self::resolve_branch_head(branch_head, parsed.oid.as_deref(), repo_path, deadline)?;

        Ok(StatusV2 {
            branch,
            detached,
            parsed,
        })
    }

    /// Get the repository state (merging, rebasing, etc.); any git query
    /// (reftable pseudorefs) finishes by `deadline`.
    ///
    /// A rebase (apply or merge) wins over a `MERGE_HEAD` left by a conflicting
    /// `--rebase-merges` step, as in git's `wt-status`.
    #[must_use]
    pub fn get_repo_state(repo_path: &Path, deadline: Instant) -> Option<RepositoryState> {
        let git_dir = Self::resolve_git_dir(repo_path);

        let rebase_apply = git_dir.join("rebase-apply");
        if rebase_apply.join("applying").exists() {
            // Only `am` writes this marker; `rebase --apply` shares the dir.
            return Some(RepositoryState::Applying);
        }

        if git_dir.join("rebase-merge").exists() || rebase_apply.exists() {
            return Some(RepositoryState::Rebasing);
        }

        if git_dir.join("MERGE_HEAD").exists() {
            return Some(RepositoryState::Merging);
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
            .map_or_default(|output| {
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
        let file = crate::fs_util::open_regular_file(&rebase_merge.join("git-rebase-todo")).ok()?;
        count_todo_steps(std::io::BufReader::new(file))
    }

    /// Run status-v2, repo-state, and stash-count captures in parallel.
    ///
    /// # Errors
    ///
    /// Returns an error if a capture thread panics.
    fn run_parallel_captures(
        repo_root: &Path,
        max_ahead_behind: usize,
        stash_enabled: bool,
        deadline: Instant,
    ) -> Result<(Result<StatusV2>, Option<RepositoryState>, u32)> {
        let (status_v2_join_result, state_join_result, stash_join_result) =
            std::thread::scope(|s| {
                let status_v2_handle =
                    s.spawn(|| Self::get_status_v2(repo_root, max_ahead_behind, deadline));
                let state_handle = s.spawn(|| Self::get_repo_state(repo_root, deadline));
                let stash_handle =
                    s.spawn(|| Self::get_stash_count(repo_root, stash_enabled, deadline));

                let status_v2_result = status_v2_handle
                    .join()
                    .map_err(|panic| Error::git(format!("status worker panicked: {panic:?}")));
                let state_result = state_handle
                    .join()
                    .map_err(|panic| Error::git(format!("state worker panicked: {panic:?}")));
                let stash_result = stash_handle
                    .join()
                    .map_err(|panic| Error::git(format!("stash worker panicked: {panic:?}")));

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
    /// `path` is canonicalized (best effort) before the repository is looked
    /// up, so a logical path through a symlink resolves the repository git
    /// and the agent's canonical request path resolve, not a lexical ancestor.
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
        max_ahead_behind: usize,
        stash_enabled: bool,
        timeout: Duration,
    ) -> Result<Option<CompleteStatus>> {
        let _t = Timer::new("native_get_complete_status");
        let canonical_path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let Some(repo_root) = crate::git::find_repo_root(&canonical_path) else {
            return Ok(None);
        };

        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            crate::Error::git("status timeout is too large for the clock".to_owned())
        })?;
        let (status_v2_res, state_opt, stash_count) =
            Self::run_parallel_captures(&repo_root, max_ahead_behind, stash_enabled, deadline)?;

        let StatusV2 {
            branch,
            detached,
            parsed,
        } = status_v2_res?;
        let counts = StatusAggregate::from_file_statuses(parsed.files.values());

        let state = resolve_overall_state(
            counts.conflicts,
            state_opt,
            counts.staged,
            counts.unstaged,
            counts.untracked,
        );
        let rebase_progress = Self::get_rebase_progress(&repo_root);

        let status = RepositoryStatus {
            branch,
            ahead: parsed.ahead,
            behind: parsed.behind,
            ahead_capped: parsed.ahead_capped,
            behind_capped: parsed.behind_capped,
            staged: counts.staged,
            unstaged: counts.unstaged,
            untracked: counts.untracked,
            conflicts: counts.conflicts,
            state,
            stash_count,
            detached,
            rebase_progress,
            stale_error: false,
        };

        Ok(Some(CompleteStatus {
            status,
            files: parsed.files,
        }))
    }

    /// File statuses for only the named paths: one `git status` with no branch
    /// header, stash, repository-state or worktree-root queries. `repo_root`
    /// must already be the canonical repository root.
    ///
    /// The scan reports the `core.ignorecase` it used, so the caller merges
    /// the result under the same rule that matched the pathspecs.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails or the deadline passes.
    pub fn get_pathspec_status(
        repo_root: &Path,
        paths: &[PathBuf],
        timeout: Duration,
    ) -> Result<PathspecScan> {
        let _t = Timer::new("native_get_pathspec_status");
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            crate::Error::git("status timeout is too large for the clock".to_owned())
        })?;
        let ignore_case = Self::ignores_case_until(repo_root, deadline);

        // `-z`: NUL-terminated records with unescaped paths.
        let mut args: Vec<OsString> = ["status", "--porcelain=v2", "-z", "--"]
            .into_iter()
            .map(OsString::from)
            .collect();
        Self::push_pathspecs(&mut args, paths, ignore_case);

        let output = run_git_bytes_until(repo_root, &args, &[], deadline)?;
        Ok(PathspecScan {
            files: parser::parse_v2_files(&output),
            ignore_case,
        })
    }

    /// Append one literal pathspec per path. `:(literal)` disables pathspec
    /// magic/globs in filenames; `icase` matches watcher spelling against
    /// index spelling on case-insensitive repositories.
    fn push_pathspecs(args: &mut Vec<OsString>, paths: &[PathBuf], ignore_case: bool) {
        let magic = if ignore_case {
            ":(literal,icase)"
        } else {
            ":(literal)"
        };
        for path in paths {
            let mut spec = OsString::from(magic);
            spec.push(path.as_os_str());
            args.push(spec);
        }
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
    run_git_env_until(repo_path, args, &[], deadline)
}

/// [`run_git_until`] with extra environment variables for the child.
///
/// # Errors
///
/// Same as [`run_git_until`].
fn run_git_env_until<S: AsRef<OsStr>>(
    repo_path: &Path,
    args: &[S],
    envs: &[(&str, &str)],
    deadline: Instant,
) -> Result<String> {
    run_git_bytes_until(repo_path, args, envs, deadline).map(|stdout| stdout_text(&stdout))
}

/// [`run_git_env_until`] returning stdout as git wrote it, for output naming
/// paths, which need not be UTF-8.
///
/// # Errors
///
/// Same as [`run_git_until`].
fn run_git_bytes_until<S: AsRef<OsStr>>(
    repo_path: &Path,
    args: &[S],
    envs: &[(&str, &str)],
    deadline: Instant,
) -> Result<Vec<u8>> {
    let result = run_git_quiet_bytes_until(repo_path, args, envs, deadline);
    if let Err(error) = &result {
        failure_log::log_failure(repo_path, args, error);
    }
    result
}

/// [`run_git_env_until`] without the failure warning, for commands whose
/// non-zero exit is an expected answer (`config --get` on an unset key,
/// `rev-parse --verify --quiet`).
///
/// # Errors
///
/// Same as [`run_git_until`].
fn run_git_quiet_env_until<S: AsRef<OsStr>>(
    repo_path: &Path,
    args: &[S],
    envs: &[(&str, &str)],
    deadline: Instant,
) -> Result<String> {
    run_git_quiet_bytes_until(repo_path, args, envs, deadline).map(|stdout| stdout_text(&stdout))
}

/// [`run_git_quiet_env_until`] returning stdout as git wrote it.
///
/// # Errors
///
/// Same as [`run_git_until`].
fn run_git_quiet_bytes_until<S: AsRef<OsStr>>(
    repo_path: &Path,
    args: &[S],
    envs: &[(&str, &str)],
    deadline: Instant,
) -> Result<Vec<u8>> {
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
    // A repository-local `core.fsmonitor` names a command that `status`,
    // `ls-files` and `update-index` would run; `-c` outranks every config scope.
    command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .envs(envs.iter().copied())
        .args(["-c", "core.fsmonitor=false"])
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
    git_stdout(output)
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

/// The stdout of a git command that succeeded.
///
/// # Errors
///
/// Returns an error if git exited non-zero.
fn git_stdout(output: std::process::Output) -> Result<Vec<u8>> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::git(format!(
            "Git command failed ({}): {}",
            output.status,
            stderr.trim()
        )));
    }
    Ok(output.stdout)
}

/// `stdout` as text with trailing whitespace trimmed; bytes that are not
/// UTF-8 are replaced.
fn stdout_text(stdout: &[u8]) -> String {
    String::from_utf8_lossy(stdout).trim_end().to_owned()
}

/// Guard against git's stale untracked cache (#1204).
mod untracked_guard;

/// Rate-limited warning log for failed git commands.
mod failure_log;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod branch_head_tests;
#[cfg(test)]
mod pathspec_tests;
#[cfg(test)]
#[cfg(unix)]
mod repo_config_tests;
#[cfg(test)]
mod repo_state_tests;
#[cfg(test)]
mod ttl_tests;
#[cfg(test)]
mod untracked_cache_tests;
#[cfg(test)]
#[cfg(unix)]
mod untracked_guard_tests;
