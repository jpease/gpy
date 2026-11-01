use super::*;

/// Supervisor settings render nothing (no refresh) but change the theme
/// export, so each rings the reload doorbell; an unchanged reload rings nothing.
#[cfg(unix)]
#[test]
#[serial_test::serial(global_ttl)]
fn supervisor_only_changes_ring_the_reload_doorbell() {
    use crate::config::types::{SupervisorCheckInterval, SupervisorMaxRestartAttempts};

    let original = Config::default();
    let mut ctx = make_agent_context();
    let counted = count_rings(&mut ctx);
    // Resolved the way `reload` re-resolves it, so re-reading it changes nothing.
    ctx.theme_manager = Arc::new(ThemeManager::new(original.ui.theme.as_str()).expect("theme"));
    let watcher_slot: SharedWatcherSlot = Arc::new(Mutex::new(None));
    let rings = |old: &Config, new: &Config| {
        let before = rung(&counted);
        handle_config_reload(
            &ctx,
            &watcher_slot,
            WatcherConfig::default(),
            true,
            old,
            new,
        )
        .expect("config reload should succeed");
        rung(&counted) > before
    };

    rings(&original, &original); // Writes the first export.
    assert!(!rings(&original, &original), "an unchanged reload rang");

    let mut disabled = original.clone();
    disabled.agent.supervisor.enabled = false;
    let mut interval = disabled.clone();
    interval.agent.supervisor.check_interval_seconds =
        SupervisorCheckInterval::new(45).expect("valid interval");
    let mut attempts = interval.clone();
    attempts.agent.supervisor.max_restart_attempts =
        SupervisorMaxRestartAttempts::new(2).expect("valid attempts");
    for (field, old, new) in [
        ("enabled", &original, &disabled),
        ("check_interval_seconds", &disabled, &interval),
        ("max_restart_attempts", &interval, &attempts),
    ] {
        assert!(
            !config_refresh_required(old, new),
            "agent.supervisor.{field} must not force a re-render"
        );
        assert!(rings(old, new), "agent.supervisor.{field} must ring");
    }
}

/// A theme reload and a config reload racing on one stale export ring once
/// between them: the gate makes the later one see the earlier one's export.
#[cfg(unix)]
#[test]
#[serial_test::serial(global_ttl)]
fn racing_theme_and_config_reloads_ring_once() {
    let config = Config::default();
    let mut ctx = make_agent_context();
    let rings = count_rings(&mut ctx);
    // Resolved the way `reload` re-resolves it, so re-reading it changes nothing.
    ctx.theme_manager = Arc::new(ThemeManager::new(config.ui.theme.as_str()).expect("theme"));
    let racer = Mutex::new(None);
    // Stands in for a swap that left the (empty) export stale; the config
    // reload starts mid-swap and, ungated, would write that export and ring.
    let swap = || {
        let (thread_ctx, thread_config) = (ctx.clone(), config.clone());
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        *racer.lock().expect("racer lock") = Some(thread::spawn(move || {
            let slot: SharedWatcherSlot = Arc::new(Mutex::new(None));
            handle_config_reload(
                &thread_ctx,
                &slot,
                WatcherConfig::default(),
                true,
                &thread_config,
                &thread_config,
            )
            .expect("config reload should succeed");
            let _ = done_tx.send(());
        }));
        let _ = done_rx.recv_timeout(Duration::from_millis(300));
        true
    };

    handle_theme_reload(&ctx, &swap);
    racer
        .lock()
        .expect("racer lock")
        .take()
        .expect("config reload started")
        .join()
        .expect("config reload thread");

    assert_eq!(rung(&rings), 1, "one theme change must ring once");
}
