//! File system watching for live prompt updates.
//!
//! Pipeline: `filesystem::FileSystemWatcher` (raw notify events) →
//! `debouncer::DebounceEngine` (coalescing) → `WatchCoordinator`.
//!
//! A flush thread sleeps until the earliest pending deadline (or, with nothing
//! pending, until the next event), drains expired events under the debouncer
//! lock, and hands each to its own dispatch thread, so a slow callback for one
//! repo never stalls ingestion or delivery for others. The timer thread is
//! needed because expiry must happen even when events stop.
//!
//! Entries flush once quiet for the debounce window (default
//! [`crate::env_config::DEFAULT_DEBOUNCE_MS`], `GPY_DEBOUNCE_MS`) or once older than the max-delay
//! cap ([`debouncer::DebounceEngine::default_max_delay`]), so sustained
//! churn cannot starve flushes.

mod coordinator;
pub mod debouncer;
pub mod filesystem;
pub(crate) mod hot_reload;
pub mod multi_repo;
mod registry;
mod watch_set;

pub use coordinator::{DelayedEventScheduler, EventCallback, PendingCallback, WatchCoordinator};
pub(crate) use registry::RepoFilter;
pub use registry::WatchRegistry;

use crate::env_config::{
    DEBOUNCE_MS, DEFAULT_DEBOUNCE_MS, DEFAULT_THROTTLE_MS, SIGURG_THROTTLE_MS,
};
use registry::is_git_significant_relative;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;
/// Which paths one Git event covers. Coalesced bursts accumulate paths up to
/// [`GitPaths::MAX_PATHS`], then degrade to [`GitPaths::WholeRepo`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitPaths {
    /// Changed paths, deduplicated, in observed order; never empty.
    Paths(Vec<PathBuf>),
    /// Whole repository changed (overflow, rescan, or too many paths).
    WholeRepo,
}

impl GitPaths {
    /// Most paths one event may accumulate before degrading to `WholeRepo`.
    pub const MAX_PATHS: usize = 32;

    /// The single path a raw filesystem event carries.
    #[must_use]
    pub fn single(path: PathBuf) -> Self {
        Self::Paths(vec![path])
    }

    /// The accumulated paths, or `None` for a whole-repository event.
    #[must_use]
    pub fn as_slice(&self) -> Option<&[PathBuf]> {
        match self {
            Self::Paths(paths) => Some(paths),
            Self::WholeRepo => None,
        }
    }

    /// Fold `other` into `self`, deduplicating and degrading to `WholeRepo`
    /// past [`GitPaths::MAX_PATHS`]. A `Vec` keeps order deterministic.
    pub fn merge(&mut self, other: &Self) {
        let (Self::Paths(existing), Self::Paths(incoming)) = (&mut *self, other) else {
            *self = Self::WholeRepo;
            return;
        };
        for path in incoming {
            if existing.contains(path) {
                continue;
            }
            if existing.len() >= Self::MAX_PATHS {
                *self = Self::WholeRepo;
                return;
            }
            existing.push(path.clone());
        }
    }
}

/// File system events that should trigger prompt updates
#[derive(Debug, Clone)]
pub enum FileEvent {
    /// Git repository changed
    Git {
        /// Paths that triggered the update.
        paths: GitPaths,
    },
    /// Language files changed (package.json, Cargo.toml, etc.)
    Language {
        /// Path that triggered the update.
        path: PathBuf,
    },
    /// Configuration changed
    Config {
        /// Path that triggered the update.
        path: PathBuf,
    },
    /// Theme file changed
    Theme {
        /// Path that triggered the update.
        path: PathBuf,
    },
}

/// Pending event emitted by the filesystem filter before debouncing.
#[derive(Debug, Clone)]
pub struct PendingEvent {
    /// Original filesystem event.
    pub event: FileEvent,
    /// Canonical repository root associated with the event.
    pub repo: PathBuf,
}

impl PendingEvent {
    /// A `Git` pending event covering the single changed `path`.
    #[must_use]
    pub fn git(repo: PathBuf, path: PathBuf) -> Self {
        Self {
            event: FileEvent::Git {
                paths: GitPaths::single(path),
            },
            repo,
        }
    }

    /// A `Git` pending event covering the whole repository (rescan).
    #[must_use]
    pub const fn git_whole_repo(repo: PathBuf) -> Self {
        Self {
            event: FileEvent::Git {
                paths: GitPaths::WholeRepo,
            },
            repo,
        }
    }
}

/// Debounced event delivered to the agent.
#[derive(Debug, Clone)]
pub struct DebouncedEvent {
    /// Original filesystem event.
    pub event: FileEvent,
    /// Canonical repository root associated with the event.
    pub repo: PathBuf,
}

impl DebouncedEvent {
    /// Whether every path behind this event lies in a `gpy/themes` tree, which
    /// the theme watcher alone reloads.
    #[must_use]
    pub fn is_theme_only(&self) -> bool {
        match &self.event {
            FileEvent::Theme { .. } => true,
            FileEvent::Git {
                paths: GitPaths::Paths(paths),
            } => paths
                .iter()
                .all(|path| !path_has_git_component(path) && is_in_gpy_themes(path)),
            FileEvent::Git {
                paths: GitPaths::WholeRepo,
            }
            | FileEvent::Language { .. }
            | FileEvent::Config { .. } => false,
        }
    }
}

/// Determine if a file path should trigger prompt updates.
#[must_use]
pub fn should_trigger_update(path: &Path, registry: &WatchRegistry) -> Option<PendingEvent> {
    filter_ignored(path)?;
    let event = classify_event(path, registry)?;
    let repo =
        crate::git::find_git_dir_only(path).or_else(|| path.parent().map(Path::to_path_buf))?;

    Some(PendingEvent { event, repo })
}

/// Whether `path` has a `.git` component immediately followed by
/// `subdir_components`. Component-based so it works with Windows separators.
fn path_contains_git_subdir(path: &Path, subdir_components: &[&str]) -> bool {
    let components: Vec<_> = path.components().collect();
    let window_len = subdir_components.len().saturating_add(1);
    components.windows(window_len).any(|window| {
        window
            .first()
            .is_some_and(|head| head.as_os_str() == OsStr::new(".git"))
            && window.get(1..).is_some_and(|rest| {
                rest.iter()
                    .zip(subdir_components)
                    .all(|(component, expected)| component.as_os_str() == OsStr::new(*expected))
            })
    })
}

/// `.git` subdirectories whose events never change repository state. `.git/info/`
/// stays unfiltered: `info/exclude` is git-significant.
const GIT_NOISE_SUBDIRS: &[&[&str]] = &[&["objects"], &["logs"], &["hooks"], &["refs", "tags"]];

fn filter_ignored(path: &Path) -> Option<()> {
    // Match file names, not path substrings (`foo.socket.rs` must pass).
    if path.file_name() == Some(OsStr::new("gpy-agent.log")) {
        return None;
    }
    if path.extension() == Some(OsStr::new("sock")) {
        return None;
    }

    if GIT_NOISE_SUBDIRS
        .iter()
        .any(|subdir| path_contains_git_subdir(path, subdir))
    {
        return None;
    }

    Some(())
}

/// Whether `path` has a `.git` component anywhere.
fn path_has_git_component(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == OsStr::new(".git"))
}

/// Whether `path` has consecutive components `first` then `second`.
fn path_has_consecutive_components(path: &Path, first: &str, second: &str) -> bool {
    let components: Vec<_> = path.components().collect();
    components.windows(2).any(|window| {
        window
            .first()
            .is_some_and(|component| component.as_os_str() == OsStr::new(first))
            && window
                .get(1)
                .is_some_and(|component| component.as_os_str() == OsStr::new(second))
    })
}

/// Whether `path` ends with components `first` then `second`.
fn path_ends_with_components(path: &Path, first: &str, second: &str) -> bool {
    let mut components = path.components().rev();
    let Some(last) = components.next() else {
        return false;
    };
    let Some(second_last) = components.next() else {
        return false;
    };
    last.as_os_str() == OsStr::new(second) && second_last.as_os_str() == OsStr::new(first)
}

/// The components after the last `.git` component, or `None` without one.
fn path_after_last_git_component(path: &Path) -> Option<PathBuf> {
    let last_git_index = path
        .components()
        .enumerate()
        .filter(|(_, component)| component.as_os_str() == OsStr::new(".git"))
        .map(|(index, _)| index)
        .last()?;
    Some(
        path.components()
            .skip(last_git_index.saturating_add(1))
            .collect(),
    )
}

/// Whether `path` is a `gpy/themes` directory or lies under one.
fn is_in_gpy_themes(path: &Path) -> bool {
    path_has_consecutive_components(path, "gpy", "themes")
        || path_ends_with_components(path, "gpy", "themes")
}

fn classify_event(path: &Path, registry: &WatchRegistry) -> Option<FileEvent> {
    if registry.is_registered_config_path(path) {
        return Some(FileEvent::Config {
            path: path.to_path_buf(),
        });
    }

    let file_name = path.file_name()?.to_str()?;

    let is_in_git_dir = path_has_git_component(path);

    // Theme saves often go via temp file + rename, so any change under
    // gpy/themes invalidates.
    if !is_in_git_dir && is_in_gpy_themes(path) {
        return Some(FileEvent::Theme {
            path: path.to_path_buf(),
        });
    }

    // In-tree `.git/…` metadata; external gitdirs go through
    // `attribute_external_gitdir`.
    if is_in_git_dir
        && let Some(relative) = path_after_last_git_component(path)
        && is_git_significant_relative(&relative)
    {
        return Some(FileEvent::Git {
            paths: GitPaths::single(path.to_path_buf()),
        });
    }

    let event = if is_language_cache_trigger(file_name) {
        FileEvent::Language {
            path: path.to_path_buf(),
        }
    } else if file_name.ends_with("gpy.toml") || file_name.eq_ignore_ascii_case("config.toml") {
        FileEvent::Config {
            path: path.to_path_buf(),
        }
    } else {
        return None;
    };

    Some(event)
}

/// Lockfiles and version pins that invalidate the language cache but are not
/// language-detection markers.
const LANGUAGE_CACHE_LOCKFILES: &[&str] = &[
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "Cargo.lock",
    "rust-toolchain",
    "rust-toolchain.toml",
    "go.work",
    ".tool-versions",
    ".python-version",
    ".node-version",
    ".nvmrc",
    "mise.toml",
    ".mise.toml",
    "mise.local.toml",
    ".mise.local.toml",
    "poetry.lock",
    "Gemfile.lock",
    "mix.lock",
    "Package.resolved",
    "settings.gradle",
];

/// Whether a change to `file_name` should invalidate the language cache
/// (detection markers plus [`LANGUAGE_CACHE_LOCKFILES`]). Case-sensitive.
fn is_language_cache_trigger(file_name: &str) -> bool {
    crate::language::metadata::marker_file_names().contains(file_name)
        || LANGUAGE_CACHE_LOCKFILES.contains(&file_name)
}
/// Watcher tuning parsed at startup.
#[derive(Clone, Copy, Debug)]
pub struct WatcherConfig {
    debounce: u64,
    throttle: u64,
}

impl WatcherConfig {
    /// Create a new watcher configuration with explicit values in milliseconds.
    #[must_use]
    pub const fn new(debounce_ms: u64, throttle_ms: u64) -> Self {
        Self {
            debounce: debounce_ms,
            throttle: throttle_ms,
        }
    }

    /// Load watcher configuration from environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        Self::new(DEBOUNCE_MS.positive(), SIGURG_THROTTLE_MS.positive())
    }

    /// Debounce duration as a `Duration`.
    #[must_use]
    pub const fn debounce_duration(self) -> Duration {
        Duration::from_millis(self.debounce)
    }

    /// Per-repository throttle duration in milliseconds.
    #[must_use]
    pub const fn throttle_ms(self) -> u64 {
        self.throttle
    }
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self::new(DEFAULT_DEBOUNCE_MS, DEFAULT_THROTTLE_MS)
    }
}

#[cfg(test)]
mod tests;
