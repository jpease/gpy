//! Language aliases (`rs`, `js`, `golang`, …) written in `[language.icons]`,
//! `language.enabled_languages` and a theme's `enabled_languages` apply as
//! their canonical language.
#![allow(clippy::unwrap_used, reason = "test code")]
#![allow(clippy::expect_used, reason = "test code")]

mod common;

use common::cli_harness::CliTestEnv;
use gpy_agent::config::language_icons::LanguageTheme;
use gpy_agent::config::loader::parse_config_toml;
use gpy_agent::config::types::Icon;
use gpy_agent::language::detector::DetectedLanguage;
use gpy_agent::language::display::select_display_languages;

fn icon(config: &gpy_agent::config::Config, key: &str) -> Option<String> {
    config
        .language
        .icons
        .icons
        .get(key)
        .map(|icon: &Icon| icon.as_str().to_owned())
}

#[test]
fn alias_icon_key_loads_under_canonical_name() {
    let config = parse_config_toml("[language.icons]\nrs = \"X\"\n").unwrap();
    assert_eq!(icon(&config, "rust").as_deref(), Some("X"));
    assert_eq!(icon(&config, "rs"), None);
}

#[test]
fn canonical_icon_key_wins_over_alias() {
    let rust = parse_config_toml("[language.icons]\nrs = \"A\"\nrust = \"B\"\n").unwrap();
    assert_eq!(icon(&rust, "rust").as_deref(), Some("B"));

    let node = parse_config_toml("[language.icons]\njs = \"A\"\nts = \"B\"\n").unwrap();
    assert_eq!(icon(&node, "node").as_deref(), Some("A"));
}

#[test]
fn alias_enabled_languages_are_canonical_after_load() {
    let config = parse_config_toml(
        "[language]\nenabled_languages = [\"golang\", \"TS\", \"rs\", \"rust\"]\n",
    )
    .unwrap();
    assert_eq!(config.language.enabled_languages, ["go", "node", "rust"]);
}

#[test]
fn theme_enabled_languages_accept_aliases() {
    let detected = [DetectedLanguage {
        name: "go".to_owned(),
        confidence: 1.0,
        file_count: 1,
        total_bytes: 1,
    }];
    let theme = LanguageTheme {
        enabled_languages: Some(vec!["golang".to_owned()]),
        ..LanguageTheme::default()
    };
    let config = parse_config_toml("").unwrap();
    let selected = select_display_languages(&detected, &theme, &config.language);
    assert_eq!(selected.len(), 1);
}

#[test]
fn alias_icon_is_rendered_by_oneshot() {
    let env = CliTestEnv::with_config(
        "[language]\nshow_versions = false\n[language.icons]\nrs = \"X\"\n",
    )
    .unwrap();
    let repo = env.root().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("Cargo.toml"), "").unwrap();
    let repo_arg = repo.to_str().expect("utf-8 temp path");

    let result = env
        .run_gpy_agent(&["oneshot", "lang", "--cwd", repo_arg, "--format", "ansi"])
        .expect("run gpy-agent");
    assert!(result.stdout.contains('X'), "{result:?}");
}

#[test]
fn alias_icon_key_survives_unrelated_set_and_is_readable() {
    let original = "[language.icons]\nrs = \"X\"\n";
    let env = CliTestEnv::with_config(original).unwrap();

    let set = env
        .run_gpy(&["config", "set", "ui.show_icons", "false"])
        .expect("run gpy");
    set.assert_success("config set ui.show_icons false");

    let written = std::fs::read_to_string(env.config_path()).unwrap();
    assert!(
        written.contains("[language.icons]\nrs = \"X\"\n"),
        "alias key must not be rewritten: {written}"
    );
    assert!(!written.contains("rust ="), "{written}");

    for key in ["language.icons.rs", "language.icons.rust"] {
        let got = env.run_gpy(&["config", "get", key]).expect("run gpy");
        got.assert_success(key);
        assert_eq!(got.stdout.trim(), "X", "{key}: {got:?}");
    }
}
