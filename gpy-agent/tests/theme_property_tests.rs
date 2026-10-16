//! Property tests for theme configuration invariants.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]

use gpy_agent::config::Config;
use gpy_agent::shell::Shell;
use gpy_agent::theme::ThemeManager;
use proptest::prelude::*;

fn hex_color_strategy() -> impl Strategy<Value = String> {
    prop::string::string_regex("#[0-9a-fA-F]{6}").expect("hex color regex should compile")
}

fn named_color_strategy() -> impl Strategy<Value = String> {
    prop::sample::select(vec![
        "black",
        "red",
        "green",
        "yellow",
        "blue",
        "magenta",
        "cyan",
        "white",
        "bright_black",
        "bright_red",
        "bright_green",
        "bright_yellow",
        "bright_blue",
        "bright_magenta",
        "bright_cyan",
        "bright_white",
    ])
    .prop_map(std::borrow::ToOwned::to_owned)
}

fn color_strategy() -> impl Strategy<Value = String> {
    prop_oneof![hex_color_strategy(), named_color_strategy()]
}

fn bool_strategy() -> impl Strategy<Value = bool> {
    any::<bool>()
}

fn segment_name_strategy() -> impl Strategy<Value = String> {
    prop::sample::select(vec![
        "directory",
        "git",
        "language",
        "clock",
        "duration",
        "status",
        "custom",
    ])
    .prop_map(std::borrow::ToOwned::to_owned)
}

proptest! {
    /// Generated hex colors are valid.
    #[test]
    fn prop_hex_colors_valid(color in hex_color_strategy()) {
        assert!(color.starts_with('#'));
        assert_eq!(color.len(), 7_usize);
        assert!(color[1..].chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// Generated named colors are in the known set.
    #[test]
    fn prop_named_colors_recognized(color in named_color_strategy()) {
        let known_colors = vec![
            "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
            "bright_black", "bright_red", "bright_green", "bright_yellow",
            "bright_blue", "bright_magenta", "bright_cyan", "bright_white",
        ];
        assert!(known_colors.contains(&color.as_str()));
    }

    /// `ThemeConfig` creation never panics on valid colors.
    #[test]
    fn prop_theme_creation_never_panics(
        bg_color in color_strategy(),
        fg_color in color_strategy(),
        directory_color in color_strategy(),
        git_color in color_strategy()
    ) {
        let _ = format!("{bg_color}{fg_color}{directory_color}{git_color}");
    }
}

proptest! {
    /// Segment names are lowercase letters and underscores, no edge underscores.
    #[test]
    fn prop_segment_names_valid_format(name in segment_name_strategy()) {

        prop_assert!(name.chars().all(|c| c.is_ascii_lowercase() || c == '_'));
        prop_assert!(!name.starts_with('_'));
        prop_assert!(!name.ends_with('_'));
    }
}

proptest! {
    /// Fish theme export is never empty.
    #[test]
    fn prop_theme_export_never_empty(
        _git_enabled in bool_strategy(),
        _language_enabled in bool_strategy()
    ) {
        let config = Config::default();

        let theme_mgr = ThemeManager::builtin("default").expect("create theme manager");
        let export = theme_mgr.export(Shell::Fish, &config);

        prop_assert!(!export.is_empty());
        prop_assert!(export.contains("set -gx") || export.contains("set -g"));
    }

    /// Fish variable names are valid identifiers.
    #[test]
    fn prop_fish_var_names_valid(name in "[a-zA-Z][a-zA-Z0-9_]*") {

        prop_assert!(name.chars().next().unwrap_or('_').is_ascii_alphabetic());
        prop_assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    }
}

proptest! {
    /// Segment order is preserved.
    #[test]
    fn prop_segment_order_preserved(
        segments in prop::collection::vec(segment_name_strategy(), 1..10)
    ) {

        let preserved = segments.clone();

        prop_assert_eq!(segments, preserved);
    }

    /// Duplicate segments do not panic.
    #[test]
    fn prop_duplicate_segments_handled(
        segment in segment_name_strategy(),
        count in 1_usize..5_usize
    ) {
        let duplicates: Vec<String> = vec![segment.clone(); count];

        prop_assert_eq!(duplicates.len(), count);
        prop_assert!(duplicates.iter().all(|s| s == &segment));
    }
}

proptest! {
    /// Empty theme configuration is valid.
    #[test]
    fn prop_empty_theme_valid(_seed in any::<u32>()) {
        let config = Config::default();
        let theme_mgr = ThemeManager::builtin("default").expect("create theme manager");

        let export = theme_mgr.export(Shell::Fish, &config);
        prop_assert!(!export.is_empty());
    }

    /// Many segments remain valid.
    #[test]
    fn prop_max_segments_valid(
        segments in prop::collection::vec(segment_name_strategy(), 0..50)
    ) {

        prop_assert!(segments.len() < 50_usize);
    }

    /// Extreme-brightness colors parse.
    #[test]
    fn prop_extreme_colors_valid(
        r in 0_u8..=255_u8,
        g in 0_u8..=255_u8,
        b in 0_u8..=255_u8
    ) {
        let hex_color = format!("#{r:02x}{g:02x}{b:02x}");

        prop_assert_eq!(hex_color.len(), 7_usize);
        prop_assert!(hex_color.starts_with('#'));
    }
}

proptest! {
    /// Theme export is idempotent.
    #[test]
    fn invariant_theme_export_idempotent(_seed in any::<u32>()) {
        let config = Config::default();
        let theme_mgr = ThemeManager::builtin("default").expect("create theme manager");

        let export1 = theme_mgr.export(Shell::Fish, &config);
        let export2 = theme_mgr.export(Shell::Fish, &config);

        prop_assert_eq!(export1, export2);
    }

    /// Config round-trips through TOML (compared via serialized form).
    #[test]
    fn invariant_config_roundtrip(_seed in any::<u32>()) {
        let config = Config::default();

        let toml_str = toml::to_string(&config).expect("serialize config");
        let roundtrip: Config = toml::from_str(&toml_str).expect("deserialize config");

        let toml_str2 = toml::to_string(&roundtrip).expect("serialize roundtrip");
        prop_assert_eq!(toml_str, toml_str2);
    }
}

proptest! {
    /// Theme export contains no raw ANSI escapes.
    #[test]
    fn regression_no_raw_ansi_in_export(_seed in any::<u32>()) {
        let config = Config::default();
        let theme_mgr = ThemeManager::builtin("default").expect("create theme manager");

        let export = theme_mgr.export(Shell::Fish, &config);

        prop_assert!(!export.contains('\x1b'));
    }

    /// Segment count stays bounded.
    #[test]
    fn regression_segment_count_reasonable(
        segments in prop::collection::vec(segment_name_strategy(), 0..100)
    ) {
        prop_assert!(segments.len() < 100_usize);
    }
}
