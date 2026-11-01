//! The git pill's background file: one per dialect, beside the git cache.

use super::*;
use crate::git::RepositoryState;
use crate::theme::ThemeManager;
use std::collections::HashMap;

/// A cache in a fresh temp dir with a repository root `repo` below it.
pub(super) fn cache_with_repo() -> (tempfile::TempDir, InstantPromptCache, PathBuf) {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let cache = InstantPromptCache::new_in_dir(dir.path().join("cache")).expect("cache");
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(repo.join(".git")).expect("git dir");
    (dir, cache, repo)
}

/// A status on `main` with nothing changed.
pub(super) fn clean_status() -> RepositoryStatus {
    RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 0,
        behind: 0,
        ahead_capped: false,
        behind_capped: false,
        staged: 0,
        unstaged: 0,
        untracked: 0,
        conflicts: 0,
        state: RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    }
}

/// The default config with `git.show_upstream` set.
fn config_with_upstream(show_upstream: bool) -> Config {
    Config {
        git: crate::config::GitSettings {
            show_upstream,
            ..crate::config::GitSettings::default()
        },
        ..Config::default()
    }
}

/// A status with one unstaged change.
fn dirty_status() -> RepositoryStatus {
    RepositoryStatus {
        unstaged: 1,
        ..clean_status()
    }
}

/// Path of `repo`'s `git_bg` file for `dialect`.
fn bg_path(cache: &InstantPromptCache, repo: &Path, dialect: PromptDialect) -> PathBuf {
    let key = path_to_cache_key(&find_project_root(repo));
    let tail = format!("git_bg.none.{}", dialect.cache_ext());
    let (dir, file_name) = cache_file_parts(&cache.cache_dir, &key, &tail);
    dir.join(file_name)
}

/// The recorded background for `dialect`, `None` when the file is absent.
fn recorded_bg(cache: &InstantPromptCache, repo: &Path, dialect: PromptDialect) -> Option<String> {
    std::fs::read_to_string(bg_path(cache, repo, dialect)).ok()
}

/// `write_git` for `status` with the default theme and `palette`.
fn write_git_with(
    cache: &InstantPromptCache,
    repo: &Path,
    status: &RepositoryStatus,
    config: &Config,
    palette: &Palette,
) -> bool {
    let theme = ThemeManager::builtin("default")
        .expect("builtin default theme")
        .get();
    cache
        .write_git(repo, status, config, &theme, None, palette)
        .expect("write git")
}

#[test]
fn git_write_records_the_state_background_per_dialect() {
    let (_dir, cache, repo) = cache_with_repo();
    write_git_with(
        &cache,
        &repo,
        &dirty_status(),
        &Config::default(),
        &Palette::default(),
    );
    for dialect in PromptDialect::ALL {
        assert_eq!(
            recorded_bg(&cache, &repo, dialect).as_deref(),
            Some("red"),
            "{dialect:?}"
        );
    }

    // Index 196 is spelled as hex only for Fish (indices 0-15 are names).
    let (_dir_fish, cache_fish, repo_fish) = cache_with_repo();
    let palette = Palette::new(HashMap::from([("red".to_owned(), Color::Ansi256(196))]));
    write_git_with(
        &cache_fish,
        &repo_fish,
        &dirty_status(),
        &Config::default(),
        &palette,
    );
    assert_eq!(
        recorded_bg(&cache_fish, &repo_fish, PromptDialect::Ansi).as_deref(),
        Some("#ff0000")
    );
    assert_eq!(
        recorded_bg(&cache_fish, &repo_fish, PromptDialect::BashPrompt).as_deref(),
        Some("196")
    );
    assert_eq!(
        recorded_bg(&cache_fish, &repo_fish, PromptDialect::ZshPrompt).as_deref(),
        Some("196")
    );
}

#[test]
fn hidden_upstream_records_the_clean_background() {
    let status = RepositoryStatus {
        ahead: 3,
        ..clean_status()
    };
    let (_dir, cache, repo) = cache_with_repo();
    write_git_with(
        &cache,
        &repo,
        &status,
        &config_with_upstream(false),
        &Palette::default(),
    );
    assert_eq!(
        recorded_bg(&cache, &repo, PromptDialect::BashPrompt).as_deref(),
        Some("green")
    );

    let (_dir_shown, cache_shown, repo_shown) = cache_with_repo();
    write_git_with(
        &cache_shown,
        &repo_shown,
        &status,
        &config_with_upstream(true),
        &Palette::default(),
    );
    let ahead_behind = ThemeManager::builtin("default")
        .expect("builtin default theme")
        .get()
        .segments
        .git
        .get_bg_color(crate::theme::GitState::AheadBehind)
        .to_owned();
    assert_ne!(ahead_behind, "green", "the default theme tells them apart");
    assert_eq!(
        recorded_bg(&cache_shown, &repo_shown, PromptDialect::BashPrompt).as_deref(),
        Some(ahead_behind.as_str())
    );
}

#[test]
fn remove_git_deletes_the_background_file() {
    let (_dir, cache, repo) = cache_with_repo();
    write_git_with(
        &cache,
        &repo,
        &dirty_status(),
        &Config::default(),
        &Palette::default(),
    );
    assert!(bg_path(&cache, &repo, PromptDialect::Ansi).exists());
    cache.remove_git(&repo);
    for dialect in PromptDialect::ALL {
        assert!(!bg_path(&cache, &repo, dialect).exists(), "{dialect:?}");
    }
}

#[test]
fn suppressed_repository_writes_no_git_bg() {
    let (_dir, cache, repo) = cache_with_repo();
    cache.remove_git(&repo);
    write_git_with(
        &cache,
        &repo,
        &dirty_status(),
        &Config::default(),
        &Palette::default(),
    );
    for dialect in PromptDialect::ALL {
        assert!(!bg_path(&cache, &repo, dialect).exists(), "{dialect:?}");
    }
}

#[test]
fn git_bg_names_match_the_shared_vectors() {
    let vectors = include_str!("../../../../tests/fixtures/instant_cache_naming_vectors.tsv");
    let mut rows = 0_u32;
    for line in vectors.lines() {
        let mut columns = line.split('\t');
        if columns.next() != Some("bgfile") {
            continue;
        }
        let (Some(ext), Some(expected)) = (columns.next(), columns.next()) else {
            panic!("short bgfile row: {line}");
        };
        let dialect = PromptDialect::ALL
            .into_iter()
            .find(|dialect| dialect.cache_ext() == ext)
            .expect("bgfile extension names a dialect");
        let name = CacheFileName {
            key: "k",
            kind: CacheKind::GitBg,
            position: SEGMENT_POSITIONS[0],
            token: NO_PREV_BG_TOKEN,
            dialect,
        };
        assert_eq!(name.tail(), expected);
        assert_eq!(parse_cache_file_name(&format!("k.{expected}")), Some(name));
        rows = rows.saturating_add(1);
    }
    assert_eq!(rows, 3, "one bgfile row per dialect");
}
