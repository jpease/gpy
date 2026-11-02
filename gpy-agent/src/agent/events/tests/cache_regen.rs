use super::*;
use crate::agent::events::reload::{
    regen_language_caches, regenerate_instant_caches_for_theme_change,
};

fn cache_file_names(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .expect("read cache dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect()
}

/// A theme change regenerates warm git instant caches so the repaint does not flicker.
#[test]
fn theme_change_regenerates_git_caches_for_active_path() {
    use crate::git::RepositoryState;
    use std::collections::HashMap;

    let (_tmp, repo) = create_temp_repo();
    let cache_tmp = tempdir().expect("cache dir");
    let ctx = make_hermetic_ctx(cache_tmp.path());

    ctx.registry.register(4321, Some(repo.clone()));
    ctx.cache
        .set(&repo, status(0, RepositoryState::Clean), HashMap::new());

    regenerate_instant_caches_for_theme_change(&ctx, &Config::default());

    let files = cache_file_names(cache_tmp.path());

    let git_none = files
        .iter()
        .find(|f| f.ends_with(".git.none.ansi"))
        .expect("context-free git cache must be regenerated");
    for dialect in crate::formatter::PromptDialect::ALL {
        let ext = format!(".{}", dialect.cache_ext());
        assert!(
            files
                .iter()
                .any(|f| f.contains(".git.none.") && f.ends_with(&ext)),
            "context-free {ext} git cache must be regenerated: {files:?}"
        );
        // A non-`none` token file proves the new theme's `prev_bg` was seeded.
        assert!(
            files.iter().any(|f| {
                f.contains(".git.")
                    && f.ends_with(&ext)
                    && !f.contains(".git.none.")
                    && !f.contains(".git_last.none.")
            }),
            "a seeded prev_bg {ext} git cache must exist: {files:?}"
        );
    }

    let rendered = fs::read_to_string(cache_tmp.path().join(git_none)).expect("read git cache");
    assert!(
        rendered.contains("main"),
        "regenerated git cache must carry the cached branch: {rendered:?}"
    );
    assert!(
        rendered.contains('\u{1b}'),
        "regenerated git cache must be styled ANSI: {rendered:?}"
    );
}

/// A repo cached before it was skipped must not get its git cache republished on reload.
#[test]
fn theme_change_does_not_publish_git_caches_for_skipped_repo() {
    use crate::git::RepositoryState;
    use std::collections::HashMap;

    let (_tmp, repo) = create_temp_repo();
    let cache_tmp = tempdir().expect("cache dir");
    let ctx = make_hermetic_ctx(cache_tmp.path());

    ctx.registry.register(4321, Some(repo.clone()));
    ctx.cache
        .set(&repo, status(0, RepositoryState::Clean), HashMap::new());

    let mut config = Config::default();
    config.git.skip_paths = vec![repo.to_string_lossy().into_owned()];
    regenerate_instant_caches_for_theme_change(&ctx, &config);

    let files = cache_file_names(cache_tmp.path());
    assert!(
        !files.iter().any(|f| f.contains(".git")),
        "a skipped repo must not get git caches: {files:?}"
    );
}

#[test]
fn show_icons_toggle_regenerates_git_caches_without_caps() {
    use crate::git::RepositoryState;
    use std::collections::HashMap;

    let (_tmp, repo) = create_temp_repo();
    let cache_tmp = tempdir().expect("cache dir");
    let ctx = make_hermetic_ctx(cache_tmp.path());
    ctx.registry.register(4321, Some(repo.clone()));
    ctx.cache
        .set(&repo, status(0, RepositoryState::Clean), HashMap::new());

    let git_cache_text = || -> String {
        cache_file_names(cache_tmp.path())
            .iter()
            .filter(|f| {
                f.contains(".git.")
                    && Path::new(f)
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("ansi"))
            })
            .map(|f| fs::read_to_string(cache_tmp.path().join(f)).expect("read git cache"))
            .collect()
    };
    let has_private_use = |text: &str| text.chars().any(|c| ('\u{e000}'..='\u{f8ff}').contains(&c));

    let mut config = Config::default();
    config.ui.show_icons = true;
    regenerate_instant_caches_for_theme_change(&ctx, &config);
    let with_icons = git_cache_text();
    assert!(
        with_icons.contains("main") && has_private_use(&with_icons),
        "control: show_icons=true cache must carry the caps: {with_icons:?}"
    );

    config.ui.show_icons = false;
    regenerate_instant_caches_for_theme_change(&ctx, &config);
    let without_icons = git_cache_text();
    assert!(
        without_icons.contains("main"),
        "regenerated cache must still carry the branch: {without_icons:?}"
    );
    assert!(
        !has_private_use(&without_icons),
        "show_icons=false cache must not contain private-use glyphs: {without_icons:?}"
    );
}

#[test]
fn language_regen_renders_cached_data() {
    use crate::language::DetectedLanguage;

    let (_tmp, repo) = create_temp_repo();
    let cache_tmp = tempdir().expect("cache dir");
    let ctx = make_hermetic_ctx(cache_tmp.path());

    ctx.language_cache.set(
        &repo,
        vec![DetectedLanguage {
            name: "rust".to_owned(),
            confidence: 1.0,
            file_count: 1,
            total_bytes: 128,
        }],
    );

    let wrote = regen_language_caches(&ctx, &repo, &Config::default());
    assert!(wrote, "first language regen must report a write");

    let files = cache_file_names(cache_tmp.path());
    for dialect in crate::formatter::PromptDialect::ALL {
        let ext = format!(".{}", dialect.cache_ext());
        assert!(
            files
                .iter()
                .any(|f| f.contains(".lang.none.") && f.ends_with(&ext)),
            "context-free {ext} language cache must be regenerated: {files:?}"
        );
        assert!(
            files.iter().any(|f| {
                f.contains(".lang.")
                    && f.ends_with(&ext)
                    && !f.contains(".lang.none.")
                    && !f.contains(".lang_last.none.")
            }),
            "a seeded prev_bg {ext} language cache must exist: {files:?}"
        );
    }
}

/// Cold paths are skipped: never run synchronous detection.
#[test]
fn language_regen_skips_cold_path() {
    let (_tmp, repo) = create_temp_repo();
    let cache_tmp = tempdir().expect("cache dir");
    let ctx = make_hermetic_ctx(cache_tmp.path());

    assert!(
        !regen_language_caches(&ctx, &repo, &Config::default()),
        "a path with no cached language data must report no write"
    );
    assert_eq!(
        cache_file_names(cache_tmp.path()).len(),
        0,
        "no language cache files should be written for a cold path"
    );
}

#[cfg(unix)]
#[test]
fn language_refresh_notifies_only_when_a_cache_write_actually_happens() {
    use crate::language::DetectedLanguage;

    let (_tmp, repo) = create_temp_repo();
    let cache_tmp = tempdir().expect("cache dir");
    let mut ctx = make_hermetic_ctx(cache_tmp.path());
    let rings = count_rings(&mut ctx);
    let config = Config::default();
    let theme = ctx.theme_manager.get();

    ctx.language_cache.set(
        &repo,
        vec![DetectedLanguage {
            name: "rust".to_owned(),
            confidence: 1.0,
            file_count: 1,
            total_bytes: 128,
        }],
    );

    let before_first = rung(&rings);
    let wrote_first = refresh_language_for(
        &ctx,
        &repo,
        &config,
        LanguageRefreshMode::CachedOnly,
        &theme,
        None,
    );
    let after_first = rung(&rings);

    assert!(
        wrote_first,
        "first render into an empty cache dir must write"
    );
    assert!(
        after_first > before_first,
        "a genuine cache write must notify (before: {before_first}, after: {after_first})"
    );

    let wrote_second = refresh_language_for(
        &ctx,
        &repo,
        &config,
        LanguageRefreshMode::CachedOnly,
        &theme,
        None,
    );
    let after_second = rung(&rings);

    assert!(
        !wrote_second,
        "a re-render of unchanged content must report no write"
    );
    assert_eq!(
        after_second, after_first,
        "a no-op re-render must not notify (after first: {after_first}, after second: {after_second})"
    );
}

#[test]
fn theme_change_skips_paths_without_warm_data() {
    let (_tmp, repo) = create_temp_repo();
    let cache_tmp = tempdir().expect("cache dir");
    let ctx = make_hermetic_ctx(cache_tmp.path());

    ctx.registry.register(4321, Some(repo));

    regenerate_instant_caches_for_theme_change(&ctx, &Config::default());

    assert_eq!(
        cache_file_names(cache_tmp.path()).len(),
        0,
        "no instant-prompt cache files should be written for a cold path"
    );
}

/// A recovery write after a failed cache write must repaint despite unchanged status.
#[test]
#[cfg(unix)]
fn instant_cache_recovery_write_triggers_repaint_on_unchanged_status() {
    use std::os::unix::fs::PermissionsExt;

    let (_tmp, repo) = create_temp_repo();
    let cache_tmp = tempdir().expect("cache dir");
    let mut ctx = make_hermetic_ctx(cache_tmp.path());
    let rings = count_rings(&mut ctx);
    let mut config = Config::default();
    // The language side channel notifies on its own thread; it would race the counts.
    config.ui.enabled_segments.retain(|name| name != "language");

    let baseline_notifications = rung(&rings);
    assert_eq!(baseline_notifications, 0, "no refresh has run yet");

    let cache_dir = cache_tmp.path();
    let writable_perms = fs::metadata(cache_dir)
        .expect("cache dir metadata")
        .permissions();
    fs::set_permissions(cache_dir, std::fs::Permissions::from_mode(0o500))
        .expect("make cache dir read-only");

    // The write fails: no notify, since a repaint could only serve a stale file.
    refresh_and_notify_if_changed(&ctx, &repo, &config, None);
    let after_failed_write = rung(&rings);

    fs::set_permissions(cache_dir, writable_perms).expect("restore cache dir permissions");

    // The failed write never updated the memo, so this write reports a change.
    refresh_and_notify_if_changed(&ctx, &repo, &config, None);
    let after_recovery_write = rung(&rings);

    assert_eq!(
        after_failed_write, baseline_notifications,
        "a failed cache write must not trigger a repaint that can only serve stale content"
    );
    assert!(
        after_recovery_write > after_failed_write,
        "the first successful recovery write must trigger a repaint even though \
         repository status did not change (before: {baseline_notifications}, \
         after failed write: {after_failed_write}, after recovery: {after_recovery_write})"
    );
}

/// A language-marker event hands re-detection and its version probes to a
/// background job: the event thread returns while a 2 s probe runs, and the
/// probed version is published and rung when it lands.
#[cfg(unix)]
#[test]
fn language_event_does_not_wait_on_a_version_probe() {
    use std::os::unix::fs::PermissionsExt;

    let (_tmp, repo) = create_temp_repo();
    fs::write(repo.join("main.py"), "print('hi')\n").expect("main.py");
    fs::write(repo.join("lib.py"), "x = 1\n").expect("lib.py");
    // A forwarded venv with no version in pyvenv.cfg: only its 2 s
    // interpreter can answer.
    let venv_tmp = tempdir().expect("venv dir");
    let venv = venv_tmp.path().join("activated");
    let python = crate::language::venv::venv_python_binary(&venv);
    fs::create_dir_all(python.parent().expect("bin dir")).expect("bin dir");
    fs::write(&python, "#!/bin/sh\nsleep 2\necho 'Python 3.7.4'\n").expect("python");
    fs::set_permissions(&python, fs::Permissions::from_mode(0o755)).expect("chmod python");
    crate::language::venv::stash_project_venv(&repo, &venv);

    let cache_tmp = tempdir().expect("cache dir");
    let mut ctx = make_hermetic_ctx(cache_tmp.path());
    let rings = count_rings(&mut ctx);
    let notified_before = rung(&rings);
    let event = DebouncedEvent {
        event: FileEvent::Language {
            path: repo.join("main.py"),
        },
        repo: repo.clone(),
    };

    let started = std::time::Instant::now();
    handle_file_event(&ctx, &event, &Config::default());
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_millis(1000),
        "the event thread waited {elapsed:?} on a 2 s version probe"
    );

    let published = || {
        cache_file_names(cache_tmp.path()).iter().any(|name| {
            name.contains(".lang")
                && fs::read_to_string(cache_tmp.path().join(name))
                    .is_ok_and(|content| content.contains("3.7.4"))
        })
    };
    // The job writes the instant cache first and rings the doorbell after, so
    // a cache that already holds the version says nothing about the ring yet:
    // wait for both, not the ring right after the cache.
    let rung = || rung(&rings) > notified_before;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !(published() && rung()) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        published(),
        "the probed version must reach the instant cache"
    );
    assert!(rung(), "the publish must ring the shells");
}
