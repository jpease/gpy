//! Tests verifying `WatcherConfig` default values.
#![allow(clippy::missing_panics_doc)] // Allowed for test modules (see CONTRIBUTING.md).

use gpy_agent::watcher::WatcherConfig;

#[test]
fn watcher_config_reads_defaults() {
    let config = WatcherConfig::from_env();
    assert_eq!(
        config.debounce_duration(),
        std::time::Duration::from_millis(100)
    );
    assert_eq!(config.throttle_ms(), 150);
    // Note: cache_refresh_cooldown_ms removed - now uses CachePolicy
}
