//! `touch_git`: refreshing cached git files without rendering them.

use super::git_bg_tests::{cache_with_repo, clean_status};
use super::tests::{set_mtime, temp_cache};
use super::*;
use crate::theme::ThemeManager;
use std::time::Duration;

/// Files per `prev_bg` context: 4 positions × 3 dialects.
const SEGMENT_FILES_PER_CONTEXT: usize = 12;

/// `write_git` of a clean status with the default config and theme.
fn write(cache: &InstantPromptCache, repo: &Path, prev_bg: Option<&str>) -> Result<bool> {
    let theme = ThemeManager::builtin("default")
        .expect("builtin default theme")
        .get();
    cache.write_git(
        repo,
        &clean_status(),
        &Config::default(),
        &theme,
        prev_bg,
        &Palette::default(),
    )
}

/// Every regular file below `dir`.
fn files_below(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).expect("read dir").flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(files_below(&path));
        } else {
            files.push(path);
        }
    }
    files
}

/// The git segment and git background files in `cache`.
fn git_files(cache: &InstantPromptCache) -> Vec<PathBuf> {
    files_below(&cache.cache_dir)
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .and_then(parse_cache_file_name)
                .is_some_and(|parsed| parsed.kind.is_git())
        })
        .collect()
}

/// The files of `files` whose name carries the `prev_bg` token of `context`.
fn in_context<'a>(files: &'a [PathBuf], context: &str) -> Vec<&'a PathBuf> {
    let marker = format!(".{}.", prev_bg_token(Some(context)));
    files
        .iter()
        .filter(|path| path.to_string_lossy().contains(&marker))
        .collect()
}

/// Move every file's mtime back an hour.
fn backdate<'a>(files: impl IntoIterator<Item = &'a PathBuf>) {
    let long_ago = SystemTime::now()
        .checked_sub(Duration::from_secs(3600))
        .expect("an hour ago is representable");
    for path in files {
        set_mtime(path, long_ago);
    }
}

/// Whether `path` was modified within the last ten minutes.
fn is_fresh(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .expect("mtime")
        .elapsed()
        .is_ok_and(|age| age < Duration::from_secs(600))
}

#[test]
fn touch_refreshes_mtime_without_rewriting() {
    let (_dir, cache, repo) = cache_with_repo();
    assert!(write(&cache, &repo, Some("blue")).expect("write"));
    let files = git_files(&cache);
    // The segment files plus one git background file per dialect.
    assert_eq!(
        files.len(),
        SEGMENT_FILES_PER_CONTEXT + PromptDialect::ALL.len()
    );
    let sentinel = in_context(&files, "blue")
        .into_iter()
        .next()
        .expect("a blue file")
        .clone();
    std::fs::write(&sentinel, "sentinel").expect("overwrite");
    backdate(&files);

    assert!(cache.touch_git(&repo, Some("blue")));

    for path in &files {
        assert!(is_fresh(path), "{} was not refreshed", path.display());
    }
    assert_eq!(
        std::fs::read_to_string(&sentinel).expect("read"),
        "sentinel",
        "a touch must not render"
    );
}

#[test]
fn touch_with_a_prev_bg_leaves_other_contexts_alone() {
    let (_dir, cache, repo) = cache_with_repo();
    write(&cache, &repo, Some("blue")).expect("write blue");
    write(&cache, &repo, Some("green")).expect("write green");
    let files = git_files(&cache);
    backdate(&files);

    assert!(cache.touch_git(&repo, Some("blue")));

    let blue = in_context(&files, "blue");
    assert_eq!(blue.len(), SEGMENT_FILES_PER_CONTEXT);
    assert!(blue.iter().all(|path| is_fresh(path)));
    let green = in_context(&files, "green");
    assert_eq!(green.len(), SEGMENT_FILES_PER_CONTEXT);
    assert!(green.iter().all(|path| !is_fresh(path)));
}

#[test]
fn touch_without_prev_bg_refreshes_every_known_context() {
    let (_dir, cache, repo) = cache_with_repo();
    for prev_bg in [None, Some("blue"), Some("green")] {
        write(&cache, &repo, prev_bg).expect("write");
    }
    let files = git_files(&cache);
    backdate(&files);

    assert!(cache.touch_git(&repo, None));

    for path in &files {
        assert!(is_fresh(path), "{} was not refreshed", path.display());
    }
}

#[test]
fn touch_for_an_unrendered_prev_bg_reports_false() {
    let (_dir, cache, repo) = cache_with_repo();
    write(&cache, &repo, Some("blue")).expect("write");

    assert!(!cache.touch_git(&repo, Some("green")));
}

#[test]
fn touch_reports_a_missing_file() {
    let (_dir, cache, repo) = cache_with_repo();
    write(&cache, &repo, Some("blue")).expect("write");
    let files = git_files(&cache);
    std::fs::remove_file(files.first().expect("a git file")).expect("remove");

    assert!(!cache.touch_git(&repo, Some("blue")));
}

#[test]
fn touch_skips_a_suppressed_repository() {
    let (_dir, cache, repo) = cache_with_repo();
    write(&cache, &repo, Some("blue")).expect("write");
    cache.remove_git(&repo);

    assert!(!cache.touch_git(&repo, Some("blue")));
}

#[test]
fn six_busy_repositories_keep_unchanged_records() {
    const CONTEXTS: usize = MAX_PREV_BG_CONTEXTS_PER_KEY;
    let (dir, cache) = temp_cache();
    let repos: Vec<PathBuf> = (0..6_u8)
        .map(|i| {
            let repo = dir.path().join(format!("repo{i}"));
            std::fs::create_dir_all(repo.join(".git")).expect("git dir");
            repo
        })
        .collect();
    let context = |i: usize| format!("#0000{i:02x}");
    for repo in &repos {
        for i in 0..CONTEXTS {
            write(&cache, repo, Some(&context(i))).expect("write");
        }
    }

    let first = repos.first().expect("a repository");
    assert!(
        !write(&cache, first, Some(&context(0))).expect("republish"),
        "an unchanged status must not rewrite files"
    );
}
