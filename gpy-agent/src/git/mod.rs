//! Git repository operations and status detection.
//!
//! Path terms: `path` is untrusted IPC input, `repo_path` is validated,
//! `git_root` is the canonical repo root, `git_dir` is the `.git` directory.

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

/// Walk up from `path` to the canonical directory containing a `.git` entry
/// (directory or pointer file). One stat per level; no bare repos, no
/// absolutizing (see [`find_repo_root`]).
#[must_use]
pub fn find_worktree_root(path: &Path) -> Option<PathBuf> {
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

/// Whether `path` is a bare repository (`HEAD`/`objects`/`refs`/`config`, no `.git`).
fn is_bare_repo_dir(path: &Path) -> bool {
    path.join("HEAD").is_file()
        && path.join("objects").is_dir()
        && path.join("refs").is_dir()
        && path.join("config").is_file()
}

/// Walk up recognizing bare repos, absolutizing relative paths and starting
/// from a file's parent. Costlier than [`find_worktree_root`]; used as its fallback.
fn find_repo_root_bare_and_relative_aware(path: &Path) -> Option<PathBuf> {
    let mut current = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .ok()
            .map_or_else(|| path.to_path_buf(), |cwd| cwd.join(path))
    };

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

/// Find the repository root containing `path`: the cheap `.git` walk, then a
/// fallback that also recognizes bare repos and relative paths.
///
/// The watcher deliberately uses the cheap walk to exclude bare repos.
#[must_use]
pub fn find_repo_root(path: &Path) -> Option<PathBuf> {
    find_worktree_root(path).or_else(|| find_repo_root_bare_and_relative_aware(path))
}

/// Resolve `target` against `base` unless absolute. No filesystem access.
#[must_use]
pub(crate) fn resolve_relative_to(base: &Path, target: &Path) -> PathBuf {
    if target.is_absolute() {
        target.to_path_buf()
    } else {
        base.join(target)
    }
}

/// Extract the target of a `gitdir:` line; `None` if missing or empty.
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

/// Read cap for single-line pointer files (`.git` and `commondir`).
pub(crate) const POINTER_READ_CAP: usize = 4_096;

/// The git directory of `repo_root`: `<root>/.git`, or the target of its
/// `gitdir:` pointer file (worktrees, submodules). Not canonicalized; `None`
/// when there is no `.git` or the pointer is unreadable or malformed.
#[must_use]
pub(crate) fn resolve_gitdir(repo_root: &Path) -> Option<PathBuf> {
    let git_entry = repo_root.join(".git");
    if git_entry.is_dir() {
        return Some(git_entry);
    }
    if git_entry.is_file() {
        let contents = crate::fs_util::read_small_file(&git_entry, POINTER_READ_CAP).ok()?;
        return parse_gitdir_link(&contents, repo_root);
    }
    None
}

/// Resolve `gitdir:` pointer contents against `base`, without canonicalizing.
#[must_use]
pub(crate) fn parse_gitdir_link(content: &str, base: &Path) -> Option<PathBuf> {
    let target = Path::new(gitdir_pointer_target(content)?);
    Some(resolve_relative_to(base, target))
}

/// Common git directory of a linked worktree; `None` for ordinary repos and
/// submodules (no `commondir` pointer).
pub(crate) fn resolve_common_dir(git_dir: &Path) -> Option<PathBuf> {
    let common = crate::fs_util::read_small_file(&git_dir.join("commondir"), POINTER_READ_CAP)
        .ok()
        .and_then(|contents| join_commondir_pointer(git_dir, &contents))
        .or_else(|| common_dir_from_worktrees_layout(git_dir))?;
    (common != git_dir).then_some(common)
}

/// Parse `commondir` contents, joining relative paths onto `git_dir`.
#[must_use]
pub(crate) fn parse_commondir_pointer(git_dir: &Path, contents: &str) -> Option<PathBuf> {
    let target = Path::new(contents.trim());
    if target.as_os_str().is_empty() {
        return None;
    }
    Some(resolve_relative_to(git_dir, target))
}

/// Resolve a `commondir` pointer (typically `../..`) to a canonical path.
fn join_commondir_pointer(git_dir: &Path, contents: &str) -> Option<PathBuf> {
    let resolved = parse_commondir_pointer(git_dir, contents)?;
    fs::canonicalize(&resolved).ok()
}

/// Fallback when `commondir` is unreadable: git's (unpromised) layout is
/// `<common>/worktrees/<name>`.
fn common_dir_from_worktrees_layout(git_dir: &Path) -> Option<PathBuf> {
    let parent = git_dir.parent()?;
    if parent.file_name()? != "worktrees" {
        return None;
    }
    fs::canonicalize(parent.parent()?).ok()
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

    /// `Some(self)` for in-progress operations an incremental update must
    /// preserve; `None` for states re-derived from counts.
    #[must_use]
    pub(crate) const fn as_preserved_special_state(self) -> Option<Self> {
        match self {
            Self::Clean | Self::Dirty | Self::Conflicts => None,
            special => Some(special),
        }
    }
}

/// Git repository status information
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
/// Only interactive rebases (`.git/rebase-merge`) report progress. `total` is
/// `msgnum` plus remaining `git-rebase-todo` steps, since git never rewrites
/// `end` on `--edit-todo`.
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

/// A full status scan: the summary plus the per-file statuses behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteStatus {
    /// The summarized repository status.
    pub status: RepositoryStatus,
    /// Per-file statuses the summary was aggregated from.
    pub files: HashMap<PathBuf, FileStatus>,
}

/// An incremental scan of named paths: their statuses and the
/// `core.ignorecase` rule the scan matched them under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathspecScan {
    /// Statuses of the named paths that are not clean.
    pub files: HashMap<PathBuf, FileStatus>,
    /// Whether the pathspecs matched case-insensitively; the caller merges
    /// the result under the same rule.
    pub ignore_case: bool,
}

impl RepositoryStatus {
    /// Zero the ahead/behind counts and their capped flags.
    /// Used when `git.show_upstream` is disabled.
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
mod tests {
    use super::*;

    /// Payloads from older agents without stash/detached/rebase-progress
    /// fields must still deserialize with defaults.
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

    /// The composed [`find_repo_root`] must reach its bare-repo fallback.
    ///
    /// The cheap walk cannot see bare repos, so this pins the fallback wiring.
    #[test]
    fn find_repo_root_falls_back_to_bare_repo_detection_when_fast_walk_finds_nothing() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let bare = temp.path().join("bare.git");
        fs::create_dir_all(bare.join("objects")).expect("objects dir");
        fs::create_dir_all(bare.join("refs")).expect("refs dir");
        fs::write(bare.join("HEAD"), "ref: refs/heads/main\n").expect("HEAD file");
        fs::write(bare.join("config"), "[core]\n\tbare = true\n").expect("config file");

        // Otherwise an ancestor repo would make this test vacuous.
        assert_eq!(
            find_worktree_root(&bare),
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
