//! The language presence marker: the file the prompt's omit check probes
//! instead of scanning the cache directory. Git keeps none: the prompt draws
//! git in any repository, from the cache, the agent or the oneshot.

use super::git_bg_tests::{cache_with_repo, clean_status};
use super::*;
use crate::theme::ThemeManager;

/// Path of `kind`'s presence marker (`{key}.{kind}.any-entry.{ext}`) for
/// `repo` in `dialect`.
fn marker_path(
    cache: &InstantPromptCache,
    repo: &Path,
    kind: CacheKind,
    dialect: PromptDialect,
) -> PathBuf {
    let key = path_to_cache_key(&find_project_root(repo));
    let name = CacheFileName {
        key: &key,
        kind,
        position: SEGMENT_POSITIONS[0],
        token: PRESENCE_TOKEN,
        dialect,
    };
    let (dir, file_name) = cache_file_parts(&cache.cache_dir, &key, &name.tail());
    dir.join(file_name)
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
/// the marker must not depend on a `.none` entry existing. Git writes none.
#[test]
fn language_writes_mark_it_present_in_every_dialect_and_git_writes_nothing() {
    let (_dir, cache, repo) = cache_with_repo();
    write_git(&cache, &repo, Some("blue"));
    write_lang(&cache, &repo, Some("blue"));

    for dialect in PromptDialect::ALL {
        assert!(
            marker_path(&cache, &repo, CacheKind::Lang, dialect).is_file(),
            "language marker missing for {dialect:?}"
        );
        assert!(
            !marker_path(&cache, &repo, CacheKind::Git, dialect).exists(),
            "nothing reads a git marker, so none is written ({dialect:?})"
        );
    }
}

/// The restart clear removes every language entry, so the language marker
/// goes with them, and so does a git marker an older agent left.
#[test]
fn restart_clear_removes_the_markers() {
    let (_dir, cache, repo) = cache_with_repo();
    write_git(&cache, &repo, None);
    write_lang(&cache, &repo, None);
    let stale_git_marker = marker_path(&cache, &repo, CacheKind::Git, PromptDialect::Ansi);
    std::fs::write(&stale_git_marker, "").expect("write an old agent's git marker");

    cache.clear_unrefreshable_files();

    assert!(!stale_git_marker.exists());
    for dialect in PromptDialect::ALL {
        assert!(!marker_path(&cache, &repo, CacheKind::Lang, dialect).exists());
    }
}

/// No `prev_bg` can name a file that collides with a marker.
#[test]
fn no_render_context_produces_the_presence_token() {
    assert_ne!(prev_bg_token(Some(PRESENCE_TOKEN)), PRESENCE_TOKEN);
}
