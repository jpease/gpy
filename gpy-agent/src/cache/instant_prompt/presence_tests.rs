//! Presence markers: the file a shell's omit check probes instead of
//! scanning the cache directory (`__gpy_instant_cache_present`).

use super::git_bg_tests::{cache_with_repo, clean_status};
use super::*;
use crate::theme::ThemeManager;

/// Path of `kind`'s presence marker for `repo` in `dialect`.
fn marker_path(
    cache: &InstantPromptCache,
    repo: &Path,
    kind: CacheKind,
    dialect: PromptDialect,
) -> PathBuf {
    let key = path_to_cache_key(&find_project_root(repo));
    let (dir, name) = cache_file_parts(
        &cache.cache_dir,
        &key,
        &presence_marker(&key, kind, dialect).tail(),
    );
    dir.join(name)
}

fn write_git(cache: &InstantPromptCache, repo: &Path, prev_bg: Option<&str>) {
    let theme = ThemeManager::builtin("default")
        .expect("builtin default theme")
        .get();
    cache
        .write_git(
            repo,
            &clean_status(),
            &Config::default(),
            &theme,
            prev_bg,
            &Palette::default(),
        )
        .expect("write git");
}

fn write_lang(cache: &InstantPromptCache, repo: &Path, prev_bg: Option<&str>) {
    let theme = ThemeManager::builtin("default")
        .expect("builtin default theme")
        .get();
    cache
        .write_language_variants(
            repo,
            &[],
            &Config::default(),
            &theme,
            prev_bg,
            &Palette::default(),
        )
        .expect("write lang");
}

/// A shell whose first request carried a `prev_bg` gets only token files, so
/// the marker must not depend on a `.none` entry existing.
#[test]
fn writes_mark_each_kind_present_in_every_dialect() {
    let (_dir, cache, repo) = cache_with_repo();
    write_git(&cache, &repo, Some("blue"));
    write_lang(&cache, &repo, Some("blue"));

    for dialect in PromptDialect::ALL {
        for kind in [CacheKind::Git, CacheKind::Lang] {
            assert!(
                marker_path(&cache, &repo, kind, dialect).is_file(),
                "{kind:?} marker missing for {dialect:?}"
            );
        }
    }
}

/// A removed repository must stop counting as present, or a shell with the
/// agent down would give its git segment a position and fork oneshot.
#[test]
fn remove_git_drops_the_git_marker_only() {
    let (_dir, cache, repo) = cache_with_repo();
    write_git(&cache, &repo, None);
    write_lang(&cache, &repo, None);

    cache.remove_git(&repo);

    for dialect in PromptDialect::ALL {
        assert!(!marker_path(&cache, &repo, CacheKind::Git, dialect).exists());
        assert!(marker_path(&cache, &repo, CacheKind::Lang, dialect).is_file());
    }
}

/// The restart clear removes every language entry, so the language marker
/// goes with them; the git `.none` entries survive, and so does their marker.
#[test]
fn restart_clear_keeps_markers_in_step_with_entries() {
    let (_dir, cache, repo) = cache_with_repo();
    write_git(&cache, &repo, None);
    write_lang(&cache, &repo, None);

    cache.clear_unrefreshable_files();

    for dialect in PromptDialect::ALL {
        assert!(marker_path(&cache, &repo, CacheKind::Git, dialect).is_file());
        assert!(!marker_path(&cache, &repo, CacheKind::Lang, dialect).exists());
    }
}

/// A cache from before markers existed gains them on the next touch: the
/// touch fails on the missing marker, and the caller's fallback write adds it.
#[test]
fn touch_without_marker_falls_back_to_a_write() {
    let (_dir, cache, repo) = cache_with_repo();
    write_git(&cache, &repo, None);
    for dialect in PromptDialect::ALL {
        std::fs::remove_file(marker_path(&cache, &repo, CacheKind::Git, dialect))
            .expect("remove marker");
    }

    assert!(!cache.touch_git(&repo, None));
}

/// No `prev_bg` can name a file that collides with a marker.
#[test]
fn no_render_context_produces_the_presence_token() {
    assert_ne!(prev_bg_token(Some(PRESENCE_TOKEN)), PRESENCE_TOKEN);
}
