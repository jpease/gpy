use super::*;

/// A stopping watcher aborts the probe and reports "healthy".
#[test]
fn health_probe_aborts_and_reports_healthy_when_stopping() {
    let stop_flag = AtomicBool::new(true);
    assert!(
        native_backend_delivers_events(Duration::from_secs(30), &stop_flag),
        "an aborted probe must not report the native backend as silent"
    );
}

#[test]
fn fallback_policy_defaults_to_probing_for_every_watcher() {
    assert_eq!(
        FallbackPolicy::resolve(None, None, None),
        FallbackPolicy::ProbeThenFallback {
            probe_timeout: Duration::from_millis(DEFAULT_WATCH_PROBE_MS),
            poll_interval: Duration::from_millis(DEFAULT_WATCH_POLL_MS),
        }
    );
}

#[test]
fn fallback_policy_probe_zero_disables_auto_detection() {
    assert_eq!(
        FallbackPolicy::resolve(None, Some("0"), None),
        FallbackPolicy::NativeOnly
    );
}

#[test]
fn fallback_policy_honors_forced_poll() {
    assert_eq!(
        FallbackPolicy::resolve(Some("1"), None, Some("750")),
        FallbackPolicy::ForcePoll {
            poll_interval: Duration::from_millis(750),
        }
    );
}

#[test]
fn fallback_policy_ignores_unparsable_and_zero_overrides() {
    assert_eq!(
        FallbackPolicy::resolve(Some("no"), Some("abc"), Some("0")),
        FallbackPolicy::ProbeThenFallback {
            probe_timeout: Duration::from_millis(DEFAULT_WATCH_PROBE_MS),
            poll_interval: Duration::from_millis(DEFAULT_WATCH_POLL_MS),
        }
    );
}

#[test]
fn fallback_policy_honors_custom_probe_and_poll_intervals() {
    assert_eq!(
        FallbackPolicy::resolve(None, Some(" 500 "), Some(" 1500 ")),
        FallbackPolicy::ProbeThenFallback {
            probe_timeout: Duration::from_millis(500),
            poll_interval: Duration::from_millis(1_500),
        }
    );
}
