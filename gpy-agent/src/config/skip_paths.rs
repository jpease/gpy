//! Matcher for `git.skip_paths` (#696).
//!
//! Every git-status producer (IPC request, registration/workspace scan,
//! watcher git events, oneshot) asks [`GitSettings::is_path_skipped`], so the
//! semantics live in exactly one place: a leading `~`/`~/` is expanded against
//! the home directory, and each entry is matched both literally and through
//! its canonical form (so `/tmp/x` skips `/private/tmp/x/repo` on macOS).
//! Matching is component-wise ([`Path::starts_with`]), never a string prefix,
//! so `/tmp/foo` does not skip `/tmp/foobar`.
//!
//! Entry resolution touches the filesystem, so it is done once per distinct
//! `skip_paths` list (and home directory) and cached; the per-event cost is a
//! list comparison plus component-wise prefix checks.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::GitSettings;

/// Expand a leading `~` / `~/` in `entry` with `home`.
///
/// Returns `None` when the entry needs a home directory that is unavailable.
/// `~user/...` is not expanded and is returned literally (it can never match
/// an absolute path, which is what validation intends).
#[must_use]
fn expand_tilde(entry: &str, home: Option<&Path>) -> Option<PathBuf> {
    if entry == "~" {
        return home.map(Path::to_path_buf);
    }
    entry.strip_prefix("~/").map_or_else(
        || Some(PathBuf::from(entry)),
        |rest| home.map(|h| h.join(rest)),
    )
}

/// Pre-resolved `skip_paths` entries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkipMatcher {
    roots: Vec<PathBuf>,
}

impl SkipMatcher {
    /// Resolve `entries` once: tilde-expand each against `home`, and keep the
    /// canonical form too when the entry exists (it need not exist yet).
    #[must_use]
    pub fn new(entries: &[String], home: Option<&Path>) -> Self {
        let mut roots = Vec::new();
        for entry in entries {
            let Some(expanded) = expand_tilde(entry, home) else {
                continue;
            };
            if let Ok(canonical) = std::fs::canonicalize(&expanded)
                && canonical != expanded
            {
                roots.push(canonical);
            }
            roots.push(expanded);
        }
        Self { roots }
    }

    /// Whether `path` is at or below any skipped root. Pass a canonical path.
    #[must_use]
    pub fn is_skipped(&self, path: &Path) -> bool {
        self.roots.iter().any(|root| path.starts_with(root))
    }
}

/// The matcher for the most recent `(skip_paths, home)` pair.
struct Cached {
    entries: Vec<String>,
    home: Option<String>,
    matcher: Arc<SkipMatcher>,
}

static CACHE: Mutex<Option<Cached>> = Mutex::new(None);

impl GitSettings {
    /// Whether git detection is skipped for `path` via `git.skip_paths`.
    ///
    /// `path` should be canonical (the IPC `SafePath`, a canonical git root).
    /// Entries are resolved once per distinct list and cached.
    #[must_use]
    pub fn is_path_skipped(&self, path: &Path) -> bool {
        if self.skip_paths.is_empty() {
            return false;
        }
        let home = crate::paths::home_dir();
        let Ok(mut guard) = CACHE.lock() else {
            return resolve(&self.skip_paths, home.as_deref()).is_skipped(path);
        };
        let hit = guard
            .as_ref()
            .filter(|c| c.entries == self.skip_paths && c.home == home)
            .map(|c| Arc::clone(&c.matcher));
        let matcher = hit.unwrap_or_else(|| {
            let built = Arc::new(resolve(&self.skip_paths, home.as_deref()));
            *guard = Some(Cached {
                entries: self.skip_paths.clone(),
                home: home.clone(),
                matcher: Arc::clone(&built),
            });
            built
        });
        drop(guard);
        matcher.is_skipped(path)
    }
}

fn resolve(entries: &[String], home: Option<&str>) -> SkipMatcher {
    SkipMatcher::new(entries, home.map(Path::new))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::missing_panics_doc)]

    use super::*;

    #[test]
    fn skip_path_tilde_expands_with_injected_home() {
        let matcher = SkipMatcher::new(&["~/x".to_owned()], Some(Path::new("/h")));
        assert!(matcher.is_skipped(Path::new("/h/x")));
        assert!(matcher.is_skipped(Path::new("/h/x/sub")));
        assert!(!matcher.is_skipped(Path::new("/h/xy")));
        assert!(!matcher.is_skipped(Path::new("/other/x")));
    }

    #[test]
    fn skip_path_bare_tilde_is_home_and_missing_home_matches_nothing() {
        let with_home = SkipMatcher::new(&["~".to_owned()], Some(Path::new("/h")));
        assert!(with_home.is_skipped(Path::new("/h/anything")));
        let no_home = SkipMatcher::new(&["~/x".to_owned(), "~other/x".to_owned()], None);
        assert!(!no_home.is_skipped(Path::new("/x")));
        assert!(!no_home.is_skipped(Path::new("/h/x")));
    }

    #[test]
    fn skip_path_is_component_wise_not_string_prefix() {
        let matcher = SkipMatcher::new(&["/tmp/foo".to_owned()], None);
        assert!(matcher.is_skipped(Path::new("/tmp/foo/repo")));
        assert!(!matcher.is_skipped(Path::new("/tmp/foobar")));
    }

    #[cfg(unix)]
    #[test]
    fn skip_path_matches_through_symlink() {
        let tmp = tempfile::TempDir::new().unwrap();
        let real = tmp.path().join("real");
        std::fs::create_dir_all(real.join("repo")).unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let canonical_repo = std::fs::canonicalize(real.join("repo")).unwrap();

        let via_link = SkipMatcher::new(&[link.to_string_lossy().into_owned()], None);
        assert!(via_link.is_skipped(&canonical_repo));

        let via_real = SkipMatcher::new(&[real.to_string_lossy().into_owned()], None);
        assert!(via_real.is_skipped(&canonical_repo));
    }

    #[cfg(unix)]
    #[test]
    fn git_settings_is_path_skipped_uses_resolved_entries() {
        let tmp = tempfile::TempDir::new().unwrap();
        let real = tmp.path().join("real");
        std::fs::create_dir_all(real.join("repo")).unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let canonical_repo = std::fs::canonicalize(real.join("repo")).unwrap();

        let settings = GitSettings {
            skip_paths: vec![link.to_string_lossy().into_owned()],
            ..GitSettings::default()
        };
        assert!(settings.is_path_skipped(&canonical_repo));
        assert!(!GitSettings::default().is_path_skipped(&canonical_repo));
    }
}
