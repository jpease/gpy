//! Git repository operations and status detection
//!
//! Provides git status information using native git subprocess for optimal performance.
//!
//! # Path Terminology
//!
//! This module uses several path-related terms with specific meanings:
//!
//! - **`path`**: Raw untrusted path from IPC client (string from user input)
//! - **`repo_path`**: Validated path after security checks (trusted PathBuf)
//! - **`git_root`**: Canonical repository root directory (e.g., `/home/user/project`)
//! - **`cwd`**: Current working directory, may be a subdirectory within a repository
//! - **`git_dir`**: The `.git` directory specifically (internal git metadata)
//!
//! These distinctions are intentional to maintain security boundaries and clarity
//! about which paths have been validated and canonicalized.

/// Caching layer for git operations
pub mod cache;
/// Native git subprocess backend implementation
pub mod native;
/// High-level git status detection
pub mod status;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Walk up from `path` looking for a `.git` entry (directory or file — the
/// latter for worktrees/submodules), returning the canonicalized directory
/// that contains it.
///
/// Cheap: one `.exists()` call per directory level, no filesystem probing
/// beyond that — this is the walk every hot-path caller (IPC handlers, the
/// watcher) uses, and it is what [`crate::watcher::MultiRepoWatcher::find_git_root`]
/// delegates to. Does not recognize bare repositories (no `.git` entry at
/// all) or absolutize a relative `path` against the current directory —
/// [`find_repo_root`] adds both, at the cost of extra stat calls, as a
/// fallback for callers that need it.
#[must_use]
pub(crate) fn find_git_dir_only(path: &Path) -> Option<PathBuf> {
    let mut current = path;
    loop {
        let git_dir = current.join(".git");
        if git_dir.exists() {
            return fs::canonicalize(current)
                .ok()
                .or_else(|| Some(current.to_path_buf()));
        }
        current = current.parent()?;
    }
}

/// Whether `path` is itself a bare repository — it directly contains the
/// `HEAD`/`objects`/`refs`/`config` layout git puts inside `.git` for a
/// non-bare one, with no `.git` subentry of its own.
fn is_bare_repo_dir(path: &Path) -> bool {
    path.join("HEAD").is_file()
        && path.join("objects").is_dir()
        && path.join("refs").is_dir()
        && path.join("config").is_file()
}

/// Walk up from `path` recognizing bare repositories as well as `.git`
/// entries, absolutizing a relative `path` against the current directory
/// first and stepping off a file path to its parent.
///
/// Strictly more expensive than [`find_git_dir_only`] — up to four extra
/// stat calls per directory level — so it runs only as that walk's fallback.
fn find_repo_root_bare_and_relative_aware(path: &Path) -> Option<PathBuf> {
    let mut current = if path.is_absolute() {
        path.to_path_buf()
    } else {
        // Try to make it absolute if possible, otherwise stick with relative
        std::env::current_dir()
            .ok()
            .map_or_else(|| path.to_path_buf(), |cwd| cwd.join(path))
    };

    // Handle file paths (start from parent)
    if current.is_file() {
        current.pop();
    }

    loop {
        if current.join(".git").exists() || is_bare_repo_dir(&current) {
            return fs::canonicalize(&current).ok().or(Some(current));
        }
        if !current.pop() {
            break;
        }
    }
    None
}

/// Find the git repository root containing `path`, walking up the directory
/// tree.
///
/// Tries the cheap `.git`-only walk first ([`find_git_dir_only`]) — the
/// common case, one stat call per level. Falls back to a walk that also
/// recognizes bare repositories (root directly contains
/// `HEAD`/`objects`/`refs`/`config`, no `.git` subentry) and absolutizes a
/// relative `path` against the current directory, only when the cheap walk
/// finds nothing — so the common case pays no extra cost. This generalizes
/// the fallback `ipc/handlers/git_handler.rs` previously applied ad hoc at one
/// call site (#321) into one shared implementation.
///
/// Not used by every repo-root-finding call site in this crate: some
/// deliberately want the cheap walk's narrower behavior (e.g. the watcher's
/// live-registration path intentionally excludes bare repos — see
/// `MultiRepoWatcher::register_repo`'s doc comment) or a different
/// no-filesystem-failure contract (`cache::instant_prompt::find_project_root`).
/// Use this one when you want "the most correct answer, willing to pay a bit
/// more for it" — cache-key/display-path resolution, not live-watch
/// registration.
#[must_use]
pub fn find_repo_root(path: &Path) -> Option<PathBuf> {
    find_git_dir_only(path).or_else(|| find_repo_root_bare_and_relative_aware(path))
}

/// Resolve `target` against `base`: an absolute target passes through, a
/// relative one is joined onto `base`.
///
/// Pure — no filesystem access, no canonicalization.
#[must_use]
pub(crate) fn resolve_relative_to(base: &Path, target: &Path) -> PathBuf {
    if target.is_absolute() {
        target.to_path_buf()
    } else {
        base.join(target)
    }
}

/// Extract the target path string from `.git`-file-style pointer contents.
///
/// The contents hold a `gitdir:` line followed by a path; whitespace after
/// the colon is optional and trimmed. Returns `None` if no `gitdir:` line is
/// present, or the target after the colon is empty.
///
/// Pure — no filesystem access.
#[must_use]
pub(crate) fn gitdir_pointer_target(contents: &str) -> Option<&str> {
    let target = contents
        .lines()
        .find_map(|line| line.trim().strip_prefix("gitdir:"))
        .map(str::trim)?;
    if target.is_empty() {
        None
    } else {
        Some(target)
    }
}

/// Resolve a `gitdir:`-pointer's contents against `base` (the directory
/// containing the pointer file / the repo root): an absolute target passes
/// through, a relative one is joined against `base`.
///
/// Pure — no filesystem access, no canonicalization (callers canonicalize
/// afterward if they need it, since not every caller does — `resolve_git_dir`'s
/// existence-check-only callers don't).
#[must_use]
pub(crate) fn parse_gitdir_link(content: &str, base: &Path) -> Option<PathBuf> {
    let target = Path::new(gitdir_pointer_target(content)?);
    Some(resolve_relative_to(base, target))
}

/// High-level repository states used for prompt rendering and IPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RepositoryState {
    /// Repository has no pending worktree or index changes.
    Clean,
    /// Repository has pending index or worktree changes.
    Dirty,
    /// Repository has merge conflicts.
    Conflicts,
    /// Repository is in the middle of a merge operation.
    Merging,
    /// Repository is in the middle of a rebase operation.
    Rebasing,
    /// Repository is in the middle of a cherry-pick operation.
    CherryPicking,
    /// Repository is in the middle of a revert operation.
    Reverting,
    /// Repository is in the middle of a bisect operation.
    Bisecting,
    /// Repository is applying patches (e.g., `git am`).
    Applying,
    /// Repository has an in-progress state that doesn't fit other variants.
    InProgress,
}

impl RepositoryState {
    /// Human-readable representation used in prompts and IPC messages.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Dirty => "dirty",
            Self::Conflicts => "conflicts",
            Self::Merging => "merging",
            Self::Rebasing => "rebasing",
            Self::CherryPicking => "cherry-picking",
            Self::Reverting => "reverting",
            Self::Bisecting => "bisecting",
            Self::Applying => "applying",
            Self::InProgress => "in-progress",
        }
    }

    /// Returns `Some(self)` when this is an in-progress operation state that an
    /// incremental (single-file) status update should preserve rather than
    /// infer from file counts alone; `None` for `Clean`/`Dirty`/`Conflicts`,
    /// which the caller re-derives from fresh counts.
    #[must_use]
    pub(crate) const fn as_preserved_special_state(self) -> Option<Self> {
        match self {
            Self::Clean | Self::Dirty | Self::Conflicts => None,
            special => Some(special),
        }
    }
}

/// Error returned when parsing an unrecognized `RepositoryState` string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseRepositoryStateError(String);

impl std::fmt::Display for ParseRepositoryStateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unrecognized repository state: {}", self.0)
    }
}

impl std::error::Error for ParseRepositoryStateError {}

impl FromStr for RepositoryState {
    type Err = ParseRepositoryStateError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "clean" => Ok(Self::Clean),
            "dirty" => Ok(Self::Dirty),
            "conflicts" => Ok(Self::Conflicts),
            "merging" => Ok(Self::Merging),
            "rebasing" => Ok(Self::Rebasing),
            "cherry-picking" => Ok(Self::CherryPicking),
            "reverting" => Ok(Self::Reverting),
            "bisecting" => Ok(Self::Bisecting),
            "applying" => Ok(Self::Applying),
            "in-progress" => Ok(Self::InProgress),
            other => Err(ParseRepositoryStateError(other.to_owned())),
        }
    }
}

/// Git repository status information
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent flags (cap/detached), not a state machine"
)]
pub struct RepositoryStatus {
    /// Current branch name
    pub branch: String,
    /// Commits ahead of upstream
    pub ahead: u32,
    /// Commits behind upstream
    pub behind: u32,
    /// Whether the ahead count was capped by configuration
    #[serde(default)]
    pub ahead_capped: bool,
    /// Whether the behind count was capped by configuration
    #[serde(default)]
    pub behind_capped: bool,
    /// Staged files count
    pub staged: u32,
    /// Unstaged files count
    pub unstaged: u32,
    /// Untracked files count
    pub untracked: u32,
    /// Conflicted files count
    pub conflicts: u32,
    /// Repository state (clean, merging, rebasing, etc.)
    pub state: RepositoryState,
    /// Number of stashed changesets
    #[serde(default)]
    pub stash_count: u32,
    /// Whether HEAD is detached (not on a branch)
    #[serde(default)]
    pub detached: bool,
    /// Step/total progress for an interactive rebase, when one is in progress
    #[serde(default)]
    pub rebase_progress: Option<RebaseProgress>,
}

/// Step/total progress for an interactive rebase (`git rebase -i`).
///
/// Only populated for interactive rebases (`.git/rebase-merge`); am-style
/// rebases (`.git/rebase-apply`) have no equivalent progress counter in v1.
/// `step` tracks `.git/rebase-merge/msgnum`; `total` is `step` plus the
/// remaining step count in the current `git-rebase-todo` rather than the
/// `end` file, which git never rewrites when `--edit-todo` changes the plan
/// (#481).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebaseProgress {
    /// Current step in the rebase sequence (1-indexed)
    pub step: u32,
    /// Total number of steps in the rebase sequence
    pub total: u32,
}

/// Status of a single file
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "represents atomic boolean states for file changes"
)]
pub struct FileStatus {
    /// File has staged changes
    pub staged: bool,
    /// File has unstaged changes
    pub unstaged: bool,
    /// File is untracked
    pub untracked: bool,
    /// File has merge conflicts
    pub conflicted: bool,
}

impl FileStatus {
    /// Check if the file status is effectively clean (no changes)
    #[must_use]
    pub const fn is_clean(self) -> bool {
        !self.staged && !self.unstaged && !self.untracked && !self.conflicted
    }
}

/// The result of a full git status scan: the summarized [`RepositoryStatus`]
/// plus the per-file statuses it was aggregated from.
///
/// Kept as two fields rather than folding `files` into `RepositoryStatus`
/// itself because most callers only need the summary — `files` exists for
/// callers that need per-file detail (e.g. cache population) without paying
/// to carry it through every `RepositoryStatus` in the codebase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteStatus {
    /// The summarized repository status.
    pub status: RepositoryStatus,
    /// Per-file statuses the summary was aggregated from.
    pub files: HashMap<PathBuf, FileStatus>,
}

impl RepositoryStatus {
    /// Zero the ahead/behind counts and their capped flags.
    ///
    /// Used when `config.git.show_upstream` is disabled: the upstream comparison
    /// still runs (it's cheap and other status fields depend on the same git
    /// call), but its result must not reach the rendered prompt (#592).
    #[must_use]
    pub const fn without_upstream(mut self) -> Self {
        self.ahead = 0;
        self.behind = 0;
        self.ahead_capped = false;
        self.behind_capped = false;
        self
    }
}

/// Derived aggregate counts for a collection of file statuses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct StatusAggregate {
    /// Staged files count
    pub staged: u32,
    /// Unstaged files count
    pub unstaged: u32,
    /// Untracked files count
    pub untracked: u32,
    /// Conflicted files count
    pub conflicts: u32,
}

impl StatusAggregate {
    /// Build aggregate counts from file-level status values.
    #[must_use]
    pub(crate) fn from_file_statuses<'a>(
        statuses: impl IntoIterator<Item = &'a FileStatus>,
    ) -> Self {
        statuses
            .into_iter()
            .fold(Self::default(), |aggregate, status| {
                aggregate.including(*status)
            })
    }

    #[must_use]
    fn including(self, status: FileStatus) -> Self {
        Self {
            staged: self.staged.saturating_add(u32::from(status.staged)),
            unstaged: self.unstaged.saturating_add(u32::from(status.unstaged)),
            untracked: self.untracked.saturating_add(u32::from(status.untracked)),
            conflicts: self.conflicts.saturating_add(u32::from(status.conflicted)),
        }
    }
}

#[cfg(test)]
#[allow(clippy::missing_panics_doc)]
#[allow(clippy::unwrap_used)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    /// Regression guard for the IPC backward-compat claim.
    ///
    /// An older agent's payload (predating stash/detached/rebase-progress
    /// capture) must still deserialize, with the new fields defaulting to
    /// their "nothing to report" values rather than failing to parse.
    #[test]
    fn repository_status_new_fields_default_to_zero_false_none() {
        let json = r#"{
            "branch": "main",
            "ahead": 0,
            "behind": 0,
            "staged": 0,
            "unstaged": 0,
            "untracked": 0,
            "conflicts": 0,
            "state": "clean"
        }"#;

        let status: RepositoryStatus =
            serde_json::from_str(json).expect("old-shape payload should still deserialize");

        assert_eq!(status.stash_count, 0);
        assert!(!status.detached);
        assert!(status.rebase_progress.is_none());
    }

    // ===== `gitdir:` pointer parsing (pure, no filesystem) =====

    #[test]
    fn gitdir_pointer_target_extracts_path_after_colon() {
        assert_eq!(
            gitdir_pointer_target("gitdir: /abs/path\n"),
            Some("/abs/path")
        );
    }

    #[test]
    fn gitdir_pointer_target_tolerates_no_space_after_colon() {
        assert_eq!(gitdir_pointer_target("gitdir:../foo\n"), Some("../foo"));
    }

    #[test]
    fn gitdir_pointer_target_tolerates_extra_surrounding_whitespace() {
        assert_eq!(
            gitdir_pointer_target("   gitdir:    /abs/path   \n"),
            Some("/abs/path")
        );
    }

    #[test]
    fn gitdir_pointer_target_returns_none_without_gitdir_line() {
        assert_eq!(gitdir_pointer_target("not a pointer file\n"), None);
    }

    #[test]
    fn gitdir_pointer_target_returns_none_for_empty_target() {
        assert_eq!(gitdir_pointer_target("gitdir:\n"), None);
        assert_eq!(gitdir_pointer_target("gitdir:    \n"), None);
    }

    #[test]
    fn gitdir_pointer_target_returns_none_for_empty_contents() {
        assert_eq!(gitdir_pointer_target(""), None);
    }

    #[test]
    fn gitdir_pointer_target_finds_the_line_among_others() {
        assert_eq!(
            gitdir_pointer_target("# a comment\n\ngitdir: /abs/path\ntrailing\n"),
            Some("/abs/path")
        );
    }

    #[test]
    fn parse_gitdir_link_joins_relative_target_against_base() {
        assert_eq!(
            parse_gitdir_link("gitdir: ../../.git/worktrees/foo\n", Path::new("/repo/sub")),
            Some(PathBuf::from("/repo/sub/../../.git/worktrees/foo"))
        );
    }

    #[test]
    fn parse_gitdir_link_passes_through_absolute_target() {
        assert_eq!(
            parse_gitdir_link("gitdir: /abs/worktree\n", Path::new("/repo")),
            Some(PathBuf::from("/abs/worktree"))
        );
    }

    #[test]
    fn parse_gitdir_link_returns_none_without_gitdir_line() {
        assert_eq!(
            parse_gitdir_link("ref: refs/heads/main\n", Path::new("/repo")),
            None
        );
    }

    // ===== repo-root discovery =====

    /// The composed [`find_repo_root`] must reach its bare-repo fallback.
    ///
    /// The cheap `.git`-only walk cannot see a bare repository at all, so this
    /// pins the fast-then-fallback wiring rather than the fallback alone.
    #[test]
    fn find_repo_root_falls_back_to_bare_repo_detection_when_fast_walk_finds_nothing() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let bare = temp.path().join("bare.git");
        fs::create_dir_all(bare.join("objects")).expect("objects dir");
        fs::create_dir_all(bare.join("refs")).expect("refs dir");
        fs::write(bare.join("HEAD"), "ref: refs/heads/main\n").expect("HEAD file");
        fs::write(bare.join("config"), "[core]\n\tbare = true\n").expect("config file");

        // The fast walk alone cannot see a bare repo (no `.git` entry). It must
        // also not find one in any ancestor of the temp dir, or this proves
        // nothing — assert it walked all the way out instead.
        assert_eq!(
            find_git_dir_only(&bare),
            None,
            "temp dir must not sit inside a repository for this test to mean anything"
        );

        let found = find_repo_root(&bare).expect("composed finder should detect the bare repo");
        assert_eq!(
            found,
            fs::canonicalize(&bare).unwrap_or(bare),
            "should resolve to the bare repo directory itself"
        );
    }
}
