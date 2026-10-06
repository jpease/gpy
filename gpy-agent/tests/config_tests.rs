//! Comprehensive configuration system tests
//! Tests structure, defaults, and validation

#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)] // Test functions panic on assertion failures

use gpy_agent::config::{
    AgentSettings, Config, DirectorySettings, GitIconSet, GitIcons, GitSettings, LanguageIcons,
    LanguageSettings, SupervisorSettings, UiSettings, types,
};

#[test]
fn test_agent_defaults() {
    let config = Config::default();
    assert!(config.agent.enabled);
    assert_eq!(config.agent.timeout_seconds.get(), 5);
    assert!(config.agent.live_updates);
    assert!(config.agent.supervisor.enabled);
    assert_eq!(config.agent.supervisor.check_interval_seconds.get(), 30);
    assert_eq!(config.agent.supervisor.max_restart_attempts.get(), 5);
}

#[test]
fn test_git_defaults() {
    let config = Config::default();
    assert!(config.git.enabled);
    assert!(config.git.show_upstream);
    assert_eq!(config.git.timeout_seconds.get(), 10);
}

#[test]
fn test_language_defaults() {
    let config = Config::default();
    assert!(config.language.enabled);
    assert!(config.language.show_versions);
    assert_eq!(config.language.cache_ttl_hours.get(), 24);
    assert_eq!(config.language.enabled_languages, Vec::<String>::new());
}

#[test]
fn test_ui_defaults() {
    let config = Config::default();
    assert!(config.ui.show_icons);
    assert_eq!(config.ui.theme.as_str(), "default");
    assert_eq!(
        config.ui.directory.display,
        types::DirectoryDisplay::Basename
    );
    assert_eq!(config.ui.directory.max_length.get(), 80);
}

#[test]
fn test_agent_config_creation() {
    let agent_config = AgentSettings {
        enabled: false,
        timeout_seconds: types::AgentTimeout::new(30).unwrap(),
        live_updates: false,
        supervisor: SupervisorSettings {
            enabled: false,
            check_interval_seconds: types::SupervisorCheckInterval::new(60).unwrap(),
            max_restart_attempts: types::SupervisorMaxRestartAttempts::new(10).unwrap(),
        },
    };

    assert!(!agent_config.enabled);
    assert_eq!(agent_config.timeout_seconds.get(), 30);
    assert!(!agent_config.live_updates);
    assert!(!agent_config.supervisor.enabled);
    assert_eq!(agent_config.supervisor.check_interval_seconds.get(), 60);
    assert_eq!(agent_config.supervisor.max_restart_attempts.get(), 10);
}

#[test]
fn test_git_config_creation() {
    let git_config = GitSettings {
        enabled: false,
        show_upstream: false,
        timeout_seconds: types::GitTimeout::new(20).unwrap(),
        skip_paths: Vec::new(),
        max_branch_length: types::MaxBranchLength::new(0).unwrap(),
        max_ahead_behind: 0,
        watch_worktree: true,
        icons: GitIcons::default(),
        icon_set: GitIconSet::default(),
        stash_enabled: true,
    };

    assert!(!git_config.enabled);
    assert!(!git_config.show_upstream);
    assert_eq!(git_config.timeout_seconds.get(), 20);
}

#[test]
fn test_language_config_creation() {
    let language_config = LanguageSettings {
        enabled: false,
        show_versions: false,
        cache_ttl_hours: types::CacheTtlHours::new(48).unwrap(),
        enabled_languages: vec!["rust".to_owned(), "python".to_owned()],
        display: types::LanguageDisplay::Icon,
        filter: types::LanguageFilter::All,
        detection_mode: types::DetectionMode::default(),
        confidence_threshold: types::ConfidenceThreshold::new(0.0).unwrap(),
        icons: LanguageIcons::default(),
    };

    assert!(!language_config.enabled);
    assert!(!language_config.show_versions);
    assert_eq!(language_config.cache_ttl_hours.get(), 48);
    assert_eq!(language_config.enabled_languages, vec!["rust", "python"]);
}

#[test]
fn test_ui_config_creation() {
    let ui_config = UiSettings {
        show_icons: false,
        theme: types::ThemeName::new("dark".to_owned()).unwrap(),
        palette: types::PaletteName::default(),
        directory: DirectorySettings {
            display: types::DirectoryDisplay::Full,
            max_length: types::MaxPathLength::new(120).unwrap(),
            ..DirectorySettings::default()
        },
        enabled_segments: vec!["directory".to_owned(), "git".to_owned()],
    };

    assert!(!ui_config.show_icons);
    assert_eq!(ui_config.theme.as_str(), "dark");
    assert_eq!(ui_config.directory.display, types::DirectoryDisplay::Full);
    assert_eq!(ui_config.directory.max_length.get(), 120);
}

#[test]
fn test_config_cloning() {
    let config = Config::default();
    let cloned_config = config.clone();

    // Should be identical
    assert_eq!(config.agent.enabled, cloned_config.agent.enabled);
    assert_eq!(config.git.enabled, cloned_config.git.enabled);
    assert_eq!(config.language.enabled, cloned_config.language.enabled);
    assert_eq!(config.ui.theme, cloned_config.ui.theme);
}

#[test]
fn test_config_modification() {
    let mut config = Config::default();

    // Modify agent config
    config.agent.enabled = false;
    config.agent.timeout_seconds = types::AgentTimeout::new(120).unwrap();

    // Modify git config
    config.git.show_upstream = false;

    // Modify language config
    config
        .language
        .enabled_languages
        .push("javascript".to_owned());

    // Verify modifications
    assert!(!config.agent.enabled);
    assert_eq!(config.agent.timeout_seconds.get(), 120);
    assert!(!config.git.show_upstream);
    assert!(
        config
            .language
            .enabled_languages
            .contains(&"javascript".to_owned())
    );
}

#[test]
fn test_config_debug_format() {
    let config = Config::default();
    let debug_string = format!("{config:?}");

    // Should contain key configuration sections
    assert!(debug_string.contains("Config"));
    assert!(debug_string.contains("agent"));
    assert!(debug_string.contains("git"));
    assert!(debug_string.contains("language"));
    assert!(debug_string.contains("ui"));
}

#[test]
fn test_agent_config_boundary_values() {
    // check_interval_seconds/max_restart_attempts bounds tightened by #597:
    // 1 and 0 are no longer valid ("minimum useful value" and "no retries"
    // respectively, as this test used to construct them) -- the newtypes'
    // own MIN (5 seconds, 1 attempt) are now the boundary values.
    let agent_config = AgentSettings {
        enabled: true,
        timeout_seconds: types::AgentTimeout::new(1).unwrap(), // Minimum value
        live_updates: true,
        supervisor: SupervisorSettings {
            enabled: true,
            check_interval_seconds: types::SupervisorCheckInterval::new(
                types::SupervisorCheckInterval::MIN,
            )
            .unwrap(),
            max_restart_attempts: types::SupervisorMaxRestartAttempts::new(
                types::SupervisorMaxRestartAttempts::MIN,
            )
            .unwrap(),
        },
    };

    assert_eq!(agent_config.timeout_seconds.get(), 1);
    assert_eq!(agent_config.supervisor.check_interval_seconds.get(), 5);
    assert_eq!(agent_config.supervisor.max_restart_attempts.get(), 1);
}

#[test]
fn test_language_config_empty_languages() {
    let language_config = LanguageSettings {
        enabled: true,
        show_versions: true,
        cache_ttl_hours: types::CacheTtlHours::new(1).unwrap(),
        enabled_languages: vec![], // Empty list should work
        display: types::LanguageDisplay::Icon,
        filter: types::LanguageFilter::All,
        detection_mode: types::DetectionMode::default(),
        confidence_threshold: types::ConfidenceThreshold::new(0.0).unwrap(),
        icons: LanguageIcons::default(),
    };

    assert_eq!(language_config.enabled_languages, Vec::<String>::new());
}

#[test]
fn test_language_config_multiple_languages() {
    let languages = vec![
        "rust".to_owned(),
        "python".to_owned(),
        "javascript".to_owned(),
        "go".to_owned(),
        "java".to_owned(),
    ];

    let language_config = LanguageSettings {
        enabled: true,
        show_versions: true,
        cache_ttl_hours: types::CacheTtlHours::new(24).unwrap(),
        enabled_languages: languages,
        display: types::LanguageDisplay::Icon,
        filter: types::LanguageFilter::All,
        detection_mode: types::DetectionMode::default(),
        confidence_threshold: types::ConfidenceThreshold::new(0.0).unwrap(),
        icons: LanguageIcons::default(),
    };

    assert_eq!(language_config.enabled_languages.len(), 5);
    assert!(
        language_config
            .enabled_languages
            .contains(&"rust".to_owned())
    );
    assert!(
        language_config
            .enabled_languages
            .contains(&"python".to_owned())
    );
}

#[test]
fn test_ui_config_theme_variations() {
    let themes = vec!["default", "dark", "light", "custom"];

    for theme in themes {
        let ui_config = UiSettings {
            show_icons: true,
            theme: types::ThemeName::new(theme.to_owned()).unwrap(),
            palette: types::PaletteName::default(),
            directory: DirectorySettings {
                display: types::DirectoryDisplay::Basename,
                max_length: types::MaxPathLength::new(80).unwrap(),
                ..DirectorySettings::default()
            },
            enabled_segments: vec!["clock".to_owned(), "directory".to_owned()],
        };

        assert_eq!(ui_config.theme.as_str(), theme);
    }
}

#[test]
fn test_config_sections_independence() {
    let mut config = Config::default();

    // Disable agent but keep other services enabled
    config.agent.enabled = false;

    // Other sections should remain enabled
    assert!(config.git.enabled);
    assert!(config.language.enabled);
    assert!(config.ui.show_icons);
}

#[test]
fn test_timeout_configurations() {
    let config = Config::default();

    // Different components can have different timeouts
    assert_eq!(config.agent.timeout_seconds.get(), 5);
    assert_eq!(config.git.timeout_seconds.get(), 10);

    // Should be able to configure them independently
    let mut custom_config = config;
    custom_config.agent.timeout_seconds = types::AgentTimeout::new(30).unwrap();
    custom_config.git.timeout_seconds = types::GitTimeout::new(5).unwrap();

    assert_eq!(custom_config.agent.timeout_seconds.get(), 30);
    assert_eq!(custom_config.git.timeout_seconds.get(), 5);
}

#[test]
fn test_supervisor_configuration_options() {
    let supervisor_disabled = AgentSettings {
        enabled: true,
        timeout_seconds: types::AgentTimeout::new(10).unwrap(),
        live_updates: true,
        supervisor: SupervisorSettings {
            enabled: false,
            check_interval_seconds: types::SupervisorCheckInterval::new(30).unwrap(),
            max_restart_attempts: types::SupervisorMaxRestartAttempts::new(3).unwrap(),
        },
    };

    let supervisor_aggressive = AgentSettings {
        enabled: true,
        timeout_seconds: types::AgentTimeout::new(10).unwrap(),
        live_updates: true,
        supervisor: SupervisorSettings {
            enabled: true,
            check_interval_seconds: types::SupervisorCheckInterval::new(5).unwrap(), // Check every 5 seconds
            max_restart_attempts: types::SupervisorMaxRestartAttempts::new(10).unwrap(), // Try many times
        },
    };

    let supervisor_conservative = AgentSettings {
        enabled: true,
        timeout_seconds: types::AgentTimeout::new(10).unwrap(),
        live_updates: true,
        supervisor: SupervisorSettings {
            enabled: true,
            check_interval_seconds: types::SupervisorCheckInterval::new(300).unwrap(), // Check every 5 minutes
            max_restart_attempts: types::SupervisorMaxRestartAttempts::new(1).unwrap(), // Try only once
        },
    };

    assert!(!supervisor_disabled.supervisor.enabled);
    assert_eq!(
        supervisor_aggressive
            .supervisor
            .check_interval_seconds
            .get(),
        5
    );
    assert_eq!(
        supervisor_conservative
            .supervisor
            .max_restart_attempts
            .get(),
        1
    );
}

#[test]
fn test_ui_directory_defaults() {
    let directory = DirectorySettings::default();
    assert_eq!(directory.display, types::DirectoryDisplay::Basename);
    assert_eq!(directory.truncation_length.get(), 3);
    assert_eq!(directory.truncation_symbol.as_str(), "");
    assert_eq!(directory.max_length.get(), 80);
}

#[test]
fn test_ui_directory_sub_table_round_trips() {
    use gpy_agent::config::metadata::{get_config_value, set_config_value};

    let mut config = Config::default();

    set_config_value(&mut config, "ui.directory.display", "truncated")
        .expect("set display=truncated");
    set_config_value(&mut config, "ui.directory.truncation_length", "5")
        .expect("set truncation_length=5");
    set_config_value(&mut config, "ui.directory.truncation_symbol", "…/")
        .expect("set truncation_symbol=…/");
    set_config_value(&mut config, "ui.directory.max_length", "120").expect("set max_length=120");

    assert_eq!(
        config.ui.directory.display,
        types::DirectoryDisplay::Truncated
    );
    assert_eq!(config.ui.directory.truncation_length.get(), 5);
    assert_eq!(config.ui.directory.truncation_symbol.as_str(), "…/");
    assert_eq!(config.ui.directory.max_length.get(), 120);

    assert_eq!(
        get_config_value(&config, "ui.directory.display").expect("get display"),
        "truncated"
    );
    assert_eq!(
        get_config_value(&config, "ui.directory.truncation_length").expect("get length"),
        "5"
    );
    assert_eq!(
        get_config_value(&config, "ui.directory.truncation_symbol").expect("get symbol"),
        "…/"
    );
    assert_eq!(
        get_config_value(&config, "ui.directory.max_length").expect("get max"),
        "120"
    );

    // abbreviated is still accepted (kept, not removed)
    set_config_value(&mut config, "ui.directory.display", "abbreviated")
        .expect("abbreviated still valid");
    assert_eq!(
        config.ui.directory.display,
        types::DirectoryDisplay::Abbreviated
    );

    set_config_value(&mut config, "ui.directory.truncate_to_repo", "true")
        .expect("set truncate_to_repo=true");
    assert!(config.ui.directory.truncate_to_repo);
    assert_eq!(
        get_config_value(&config, "ui.directory.truncate_to_repo").expect("get truncate_to_repo"),
        "true"
    );

    // invalid bounds rejected
    assert!(set_config_value(&mut config, "ui.directory.truncation_length", "0").is_err());
    assert!(set_config_value(&mut config, "ui.directory.truncation_symbol", "bad\nsym").is_err());
}

#[test]
fn test_truncate_to_repo_defaults_off() {
    let config = Config::default();
    assert!(
        !config.ui.directory.truncate_to_repo,
        "truncate_to_repo must default to false"
    );
}
