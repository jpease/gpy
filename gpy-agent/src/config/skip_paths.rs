//! Matcher for `git.skip_paths`: `~` expanded, entries matched literally and
//! canonically, component-wise (`/tmp/foo` doesn't skip `/tmp/foobar`).
//! Resolution hits the filesystem, so it's cached per distinct list and
//! redone after [`RESOLVE_TTL`].

use crate::sync_ext::MutexExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::GitSettings;

/// Expand a leading `~`/`~/`; `None` if home is needed but unknown.
/// `~user/...` stays literal (never matches an absolute path).
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

/// Whether `entry` can ever match: an absolute path, `~`, or `~/…`.
/// `Path::is_absolute` is platform-aware; `~user/…` is never expanded.
#[must_use]
pub fn is_valid_entry(entry: &str) -> bool {
    entry == "~" || entry.starts_with("~/") || Path::new(entry).is_absolute()
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

/// How long a resolved matcher is reused before its entries are canonicalized
/// again. Bounds how long a root created, removed, or retargeted after first
/// use stays stale, while lookups inside the window touch no filesystem.
const RESOLVE_TTL: Duration = Duration::from_secs(2);

/// The matcher for one `(skip_paths, home)` pair and when it was resolved.
struct Cached {
    entries: Vec<String>,
    home: Option<String>,
    resolved_at: Instant,
    matcher: Arc<SkipMatcher>,
}

/// Most distinct `(skip_paths, home)` pairs kept; the oldest is evicted past it.
const MAX_CACHED: usize = 8;

/// Holds one matcher per recent `(skip_paths, home)` pair, so configs that
/// alternate (reload overlap, tests) don't re-canonicalize on every call.
struct MatcherCache(Mutex<Vec<Cached>>);

impl MatcherCache {
    const fn new() -> Self {
        Self(Mutex::new(Vec::new()))
    }

    /// The matcher for `entries`/`home`, re-resolved when the pair is new or
    /// the cached one is at least [`RESOLVE_TTL`] old at `now`. Resolution
    /// runs outside the lock: `canonicalize` on a slow mount must not stall
    /// every other Git gate check.
    fn matcher(&self, entries: &[String], home: Option<&str>, now: Instant) -> Arc<SkipMatcher> {
        let is_pair = |c: &Cached| c.entries == entries && c.home.as_deref() == home;
        {
            let guard = self.0.lock_or_recover();
            if let Some(cached) = guard
                .iter()
                .find(|c| is_pair(c) && now.saturating_duration_since(c.resolved_at) < RESOLVE_TTL)
            {
                return Arc::clone(&cached.matcher);
            }
        }
        let matcher = Arc::new(SkipMatcher::new(entries, home.map(Path::new)));
        let mut guard = self.0.lock_or_recover();
        // A concurrent resolver may have stored a newer result meanwhile; keep it.
        if guard.iter().any(|c| is_pair(c) && c.resolved_at > now) {
            return matcher;
        }
        guard.retain(|c| !is_pair(c));
        if guard.len() >= MAX_CACHED {
            guard.remove(0);
        }
        guard.push(Cached {
            entries: entries.to_vec(),
            home: home.map(str::to_owned),
            resolved_at: now,
            matcher: Arc::clone(&matcher),
        });
        matcher
    }
}

static CACHE: MatcherCache = MatcherCache::new();

impl GitSettings {
    /// Whether Git detection may run for the repository rooted at `root` (pass
    /// its canonical root): `git.enabled` and the root not at or under a
    /// `git.skip_paths` entry, so an entry inside a repository does not skip it.
    /// Every Git status scan and Git cache publication entry point, foreground
    /// or background, must check this against the repository root.
    #[must_use]
    pub fn is_enabled_for(&self, root: &Path) -> bool {
        self.enabled && !self.is_path_skipped(root)
    }

    /// Whether `git.skip_paths` skips `path` (pass a canonical path).
    fn is_path_skipped(&self, path: &Path) -> bool {
        if self.skip_paths.is_empty() {
            return false;
        }
        let home = crate::paths::home_dir();
        CACHE
            .matcher(&self.skip_paths, home.as_deref(), Instant::now())
            .is_skipped(path)
    }
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

    /// Lookups inside the TTL reuse the resolution; after it, a root created
    /// late starts matching and a retargeted symlink drops its old target.
    /// Alternating lists each keep their own resolution.
    #[test]
    fn matcher_cache_keeps_alternating_lists_resolved() {
        let cache = MatcherCache::new();
        let now = Instant::now();
        let a = vec!["/a".to_owned()];
        let b = vec!["/b".to_owned()];

        let first_a = cache.matcher(&a, None, now);
        let first_b = cache.matcher(&b, None, now);

        assert!(Arc::ptr_eq(&first_a, &cache.matcher(&a, None, now)));
        assert!(Arc::ptr_eq(&first_b, &cache.matcher(&b, None, now)));
    }

    #[cfg(unix)]
    #[test]
    fn matcher_cache_re_resolves_created_and_retargeted_roots_after_ttl() {
        let tmp = tempfile::TempDir::new().unwrap();
        let first_dir = tmp.path().join("first");
        let second_dir = tmp.path().join("second");
        std::fs::create_dir_all(&first_dir).unwrap();
        std::fs::create_dir_all(&second_dir).unwrap();
        let first = std::fs::canonicalize(first_dir).unwrap();
        let second = std::fs::canonicalize(second_dir).unwrap();
        let link = tmp.path().join("link");
        let entries = vec![link.to_string_lossy().into_owned()];
        let cache = MatcherCache::new();
        let start = Instant::now();
        let after_ttl = |ttls: u32| start.checked_add(RESOLVE_TTL * ttls).unwrap();

        assert!(!cache.matcher(&entries, None, start).is_skipped(&first));

        std::os::unix::fs::symlink(&first, &link).unwrap();
        assert!(
            !cache.matcher(&entries, None, start).is_skipped(&first),
            "inside the TTL the earlier resolution is reused"
        );
        assert!(
            cache
                .matcher(&entries, None, after_ttl(1))
                .is_skipped(&first)
        );

        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(&second, &link).unwrap();
        let retargeted = cache.matcher(&entries, None, after_ttl(2));
        assert!(!retargeted.is_skipped(&first));
        assert!(retargeted.is_skipped(&second));
    }
}
