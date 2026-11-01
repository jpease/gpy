use super::*;

#[test]
#[serial_test::serial(global_ttl)]
fn config_reload_disables_and_reenables_watcher() {
    let (
        theme_manager,
        watcher_config,
        watcher_slot,
        client_registry,
        git_cache,
        config_manager,
        instant_cache,
    ) = setup_reload_context();

    let initial_config = config_manager.get();
    let ctx = AgentContext {
        registry: client_registry,
        cache: git_cache,
        config_manager,
        theme_manager,
        instant_cache,
        language_cache: crate::language::DetectionCache::new(),
        palette_cache: Arc::new(crate::palette::PaletteCache::from_config(&initial_config)),
        dispatch_guard: Arc::new(RepoRefreshCoordinator::default()),
        watch_registry: Arc::new(crate::watcher::WatchRegistry::new()),
        theme_export_dir: temp_export_dir(),
        announce_gate: Arc::new(Mutex::new(())),
    };

    let apply = |old_cfg: &Config, new_cfg: &Config| {
        handle_config_reload(&ctx, &watcher_slot, watcher_config, false, old_cfg, new_cfg)
            .expect("config reload should succeed");
    };

    let original = Config::default();
    let mut git_disabled = original.clone();
    git_disabled.git.enabled = false;
    apply(&original, &git_disabled);
    expect_watcher_state(
        &watcher_slot,
        true,
        "watcher should remain active when language live updates still need it",
    );

    let mut disabled = git_disabled.clone();
    disabled.language.enabled = false;
    apply(&git_disabled, &disabled);
    expect_watcher_state(
        &watcher_slot,
        false,
        "watcher should be cleared when both git and language updates are disabled",
    );

    let mut reenabled = disabled.clone();
    reenabled.git.enabled = true;
    apply(&disabled, &reenabled);
    expect_watcher_state(
        &watcher_slot,
        true,
        "watcher should be recreated when git.enabled returns to true",
    );

    let mut no_live_updates = reenabled.clone();
    no_live_updates.agent.live_updates = false;
    apply(&reenabled, &no_live_updates);
    expect_watcher_state(
        &watcher_slot,
        false,
        "watcher should stop when agent.live_updates is false",
    );
}

/// A reload adding a `git.skip_paths` entry for a watched repository stops
/// watching it; removing the entry arms it again for the registered client.
#[test]
#[serial_test::serial(global_ttl)]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn config_reload_skip_path_stops_and_restores_watching() {
    let (_tmp, repo) = create_temp_repo();
    let ctx = make_agent_context();
    let watcher_config = WatcherConfig::default();
    let watcher = create_watcher(watcher_config, &ctx).expect("create real watcher");
    let pid = std::process::id();
    ctx.registry.register(pid, Some(repo.clone()));
    watcher
        .attach_client(pid, &repo)
        .expect("register with watcher");
    assert_eq!(watcher.watched_repo_count(), 1);
    let watcher_slot: SharedWatcherSlot = Arc::new(Mutex::new(Some(Arc::clone(&watcher))));

    let original = Config::default();
    let mut skipped = original.clone();
    skipped.git.skip_paths = vec![repo.to_string_lossy().into_owned()];

    handle_config_reload(
        &ctx,
        &watcher_slot,
        watcher_config,
        false,
        &original,
        &skipped,
    )
    .expect("reload adding the skip entry");
    assert_eq!(
        watcher.watched_repo_count(),
        0,
        "a newly skipped repository must stop being watched"
    );

    handle_config_reload(
        &ctx,
        &watcher_slot,
        watcher_config,
        false,
        &skipped,
        &original,
    )
    .expect("reload removing the skip entry");
    assert_eq!(
        watcher.watched_repo_count(),
        1,
        "removing the skip entry must watch the repository again"
    );
    assert_eq!(watcher.client_pids(), vec![pid]);
}

#[cfg(unix)]
#[test]
#[serial_test::serial(global_ttl)]
fn config_reload_notifies_on_language_toggles() {
    let mut ctx = make_agent_context();
    let rings = count_rings(&mut ctx);
    let watcher_slot: SharedWatcherSlot = Arc::new(Mutex::new(None));
    let watcher_config = WatcherConfig::default();

    let original = Config::default();
    let mut toggled = original.clone();
    toggled.language.show_versions = !original.language.show_versions;

    let before = rung(&rings);
    handle_config_reload(
        &ctx,
        &watcher_slot,
        watcher_config,
        true,
        &original,
        &toggled,
    )
    .expect("config reload should succeed");
    let after = rung(&rings);
    assert!(
        after > before,
        "toggling show_versions should trigger notification"
    );

    let mut toggled_enabled = toggled.clone();
    toggled_enabled.language.enabled = !toggled.language.enabled;
    let before_enabled = rung(&rings);
    handle_config_reload(
        &ctx,
        &watcher_slot,
        watcher_config,
        true,
        &toggled,
        &toggled_enabled,
    )
    .expect("config reload should succeed");
    let after_enabled = rung(&rings);
    assert!(
        after_enabled > before_enabled,
        "toggling language.enabled should trigger notification"
    );

    // A real builtin theme: unresolvable theme names are a hard error.
    let mut theme_changed = toggled_enabled.clone();
    theme_changed.ui.theme =
        crate::config::types::ThemeName::new("text".to_owned()).expect("valid theme name");
    let before_theme = rung(&rings);
    handle_config_reload(
        &ctx,
        &watcher_slot,
        watcher_config,
        true,
        &toggled_enabled,
        &theme_changed,
    )
    .expect("config reload should succeed");
    let after_theme = rung(&rings);
    assert!(
        after_theme > before_theme,
        "changing ui.theme should trigger notification"
    );
}

#[allow(clippy::type_complexity)]
fn setup_reload_context() -> (
    Arc<ThemeManager>,
    WatcherConfig,
    SharedWatcherSlot,
    Arc<ClientDirectory>,
    Arc<GitStatusCache>,
    Arc<crate::config::manager::ConfigManager>,
    Arc<InstantPromptCache>,
) {
    let theme_manager = make_theme_manager();
    let watcher_config = WatcherConfig::default();
    let mock_watcher = Arc::new(
        MultiRepoWatcher::builder()
            .config(watcher_config)
            .callback(Box::new(|_event: DebouncedEvent| {}))
            .build()
            .expect("mock watcher should initialize"),
    );
    let watcher_slot: SharedWatcherSlot = Arc::new(Mutex::new(Some(mock_watcher)));
    let client_registry = make_client_registry();
    let git_cache = make_git_cache();
    let config_manager = make_config_manager();
    let instant_cache = make_instant_cache();

    (
        theme_manager,
        watcher_config,
        watcher_slot,
        client_registry,
        git_cache,
        config_manager,
        instant_cache,
    )
}

fn expect_watcher_state(watcher_slot: &SharedWatcherSlot, expect_some: bool, message: &str) {
    let watcher_guard = watcher_slot.lock().expect("watcher lock poisoned");
    assert_eq!(watcher_guard.is_some(), expect_some, "{message}");
}

#[test]
#[serial_test::serial(global_ttl)]
fn config_reload_updates_language_ttl() {
    use crate::cache::ttl_map::{LanguageTtlGuard, shared_cache_ttl_seconds};
    let _ttl = LanguageTtlGuard::set(24);
    let ctx = make_agent_context();
    let watcher_slot: SharedWatcherSlot = Arc::new(Mutex::new(None));
    let watcher_config = WatcherConfig::default();

    std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
    let before = shared_cache_ttl_seconds();
    let initial_hours = before / 3600;

    let mut old_config = Config::default();
    old_config.language.cache_ttl_hours =
        crate::config::types::CacheTtlHours::new(initial_hours).expect("valid ttl");

    let mut updated = Config::default();
    updated.language.cache_ttl_hours =
        crate::config::types::CacheTtlHours::new(36).expect("valid ttl");

    handle_config_reload(
        &ctx,
        &watcher_slot,
        watcher_config,
        true,
        &old_config,
        &updated,
    )
    .expect("config reload should succeed");

    let after = shared_cache_ttl_seconds();
    assert_eq!(
        after,
        36 * 3600,
        "cache TTL should update to new value after reload"
    );
    assert_ne!(after, before, "TTL should change from previous value");
}

#[test]
fn palette_change_triggers_refresh_required() {
    use crate::config::types::PaletteName;
    let old = Config::default();
    let mut new_cfg = Config::default();
    new_cfg.ui.palette = PaletteName::new("other-palette".to_owned()).expect("valid name");
    assert!(
        config_refresh_required(&old, &new_cfg),
        "a ui.palette change must be recognized as requiring a prompt refresh"
    );
}

#[test]
fn plugin_segment_override_change_triggers_refresh_required() {
    let old = Config::default();
    let new_cfg = crate::config::loader::parse_config_toml("[plugin.k8s]\nttl_secs = 30\n")
        .expect("valid plugin override");
    assert!(
        config_refresh_required(&old, &new_cfg),
        "a [plugin.<segment>] change must be recognized as requiring a prompt refresh"
    );
}

#[test]
fn unchanged_config_does_not_require_refresh() {
    let config = Config::default();
    assert!(
        !config_refresh_required(&config, &config),
        "identical configs must not trigger a prompt refresh"
    );
}

#[test]
fn ui_show_icons_change_triggers_refresh_required() {
    let old = crate::config::UiSettings::default();
    let new = crate::config::UiSettings {
        show_icons: !old.show_icons,
        ..crate::config::UiSettings::default()
    };
    assert!(
        ui_refresh_required(&old, &new),
        "ui.show_icons is rendered directly; changing it must require a refresh"
    );
}

/// Shells re-source `ui.transient_prompt` only after a reload doorbell.
#[test]
fn ui_transient_prompt_change_triggers_refresh_required() {
    let old = crate::config::UiSettings::default();
    let new = crate::config::UiSettings {
        transient_prompt: !old.transient_prompt,
        ..crate::config::UiSettings::default()
    };
    assert!(ui_refresh_required(&old, &new));
}

#[test]
fn git_stash_enabled_change_triggers_refresh_required() {
    let old = crate::config::GitSettings::default();
    let new = crate::config::GitSettings {
        stash_enabled: !old.stash_enabled,
        ..crate::config::GitSettings::default()
    };
    assert!(
        git_refresh_required(&old, &new),
        "git.stash_enabled affects the rendered $stash indicator; must require a refresh"
    );
}

#[test]
fn excluded_field_changes_do_not_trigger_refresh_required() {
    type Case = (&'static str, fn() -> bool);
    let cases: [Case; 3] = [
        ("git.timeout_seconds", || {
            let old = crate::config::GitSettings::default();
            let new = crate::config::GitSettings {
                timeout_seconds: crate::config::types::GitTimeout::new(
                    old.timeout_seconds.get() + 1,
                )
                .expect("valid git timeout"),
                ..crate::config::GitSettings::default()
            };
            assert_ne!(old.timeout_seconds, new.timeout_seconds);
            git_refresh_required(&old, &new)
        }),
        ("language.cache_ttl_hours", || {
            let old = crate::config::LanguageSettings::default();
            let new = crate::config::LanguageSettings {
                cache_ttl_hours: crate::config::types::CacheTtlHours::new(
                    old.cache_ttl_hours.get() + 1,
                )
                .expect("valid cache ttl"),
                ..crate::config::LanguageSettings::default()
            };
            assert_ne!(old.cache_ttl_hours, new.cache_ttl_hours);
            language_refresh_required(&old, &new)
        }),
        ("agent.timeout_seconds", || {
            let old = crate::config::AgentSettings::default();
            let new = crate::config::AgentSettings {
                timeout_seconds: crate::config::types::AgentTimeout::new(
                    old.timeout_seconds.get() + 1,
                )
                .expect("valid agent timeout"),
                ..crate::config::AgentSettings::default()
            };
            assert_ne!(old.timeout_seconds, new.timeout_seconds);
            agent_refresh_required(&old, &new)
        }),
    ];
    for (field, refresh_required) in cases {
        assert!(
            !refresh_required(),
            "{field} does not affect rendering; must not require a refresh"
        );
    }
}

#[test]
fn language_filter_change_triggers_refresh_required() {
    let old = crate::config::LanguageSettings::default();
    let new = crate::config::LanguageSettings {
        filter: crate::config::types::LanguageFilter::Primary,
        ..crate::config::LanguageSettings::default()
    };
    assert_ne!(
        old.filter, new.filter,
        "the mutation must actually change the field for this test to mean anything"
    );
    assert!(
        language_refresh_required(&old, &new),
        "language.filter is a render-time decision over already-detected languages; must require a refresh"
    );
}

#[test]
fn agent_live_updates_change_triggers_refresh_required() {
    let old = crate::config::AgentSettings::default();
    let new = crate::config::AgentSettings {
        live_updates: !old.live_updates,
        ..crate::config::AgentSettings::default()
    };
    assert!(
        agent_refresh_required(&old, &new),
        "agent.live_updates gates whether renders are pushed at all; must require a refresh"
    );
}
