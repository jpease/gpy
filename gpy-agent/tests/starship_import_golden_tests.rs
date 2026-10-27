//! Golden import tests: real-ish Starship configs → valid GPY artifacts.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

use gpy_agent::config::types::ColorSpec;
use gpy_agent::config::validation::templates::validate_segment_templates;
use gpy_agent::import::starship::{ImportArtifacts, WarningKind, build, parse};
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/starship")
}

fn import(fixture_name: &str, name: &str) -> ImportArtifacts {
    let path = fixtures_dir().join(fixture_name);
    let input = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("fixture not found: {}", path.display()));
    let model = parse(&input).expect("parse starship fixture");
    build(&model, name).expect("build import artifacts")
}

#[test]
fn default_config_emits_valid_validatable_theme() {
    let artifacts = import("import_default.toml", "default-import");

    validate_segment_templates(&artifacts.theme, &gpy_agent::template::Palette::default())
        .expect("imported theme validates against default palette");

    assert_eq!(
        artifacts.theme.segments.directory.format.as_deref(),
        Some("[$path](bold cyan)[$read_only](red) ")
    );
    assert_eq!(
        artifacts.theme.segments.git.format.as_deref(),
        Some(r"on [$symbol$branch](bold purple) ([\[$status$ahead_behind\]](bold red) )")
    );
    assert_eq!(
        artifacts.theme.segments.character.success_color.as_str(),
        "green"
    );
    assert_eq!(
        artifacts.segments,
        vec!["directory", "git", "duration", "character"]
    );
}

#[test]
fn preset_config_emits_valid_palette_and_language_colors() {
    let artifacts = import("import_preset.toml", "catppuccin");

    assert_eq!(
        artifacts.palette.colors.get("mauve").map(ColorSpec::as_str),
        Some("#cba6f7")
    );
    assert_eq!(
        artifacts.palette.colors.get("rust").map(ColorSpec::as_str),
        Some("red")
    );

    // Use the imported palette so aliases like "mauve" resolve.
    let template_palette = artifacts.palette.to_template_palette();
    validate_segment_templates(&artifacts.theme, &template_palette)
        .expect("imported theme validates against imported palette");

    assert_eq!(
        artifacts
            .theme
            .segments
            .language
            .overrides
            .get("node_bg_color")
            .map(ColorSpec::as_str),
        Some("green")
    );
    assert_eq!(
        artifacts
            .theme
            .segments
            .language
            .overrides
            .get("go_bg_color")
            .map(ColorSpec::as_str),
        Some("cyan")
    );

    assert_eq!(
        artifacts.segments,
        vec!["directory", "git", "language", "duration", "character"]
    );
    // Per-language symbol is lossy (lives in [language.icons]).
    assert!(
        artifacts
            .warnings
            .iter()
            .any(|w| w.message.contains("symbol")),
        "expected a lossy-symbol warning but got: {:?}",
        artifacts
            .warnings
            .iter()
            .map(|w| &w.message)
            .collect::<Vec<_>>()
    );
}

#[test]
fn real_world_powerline_preset_survives_import_with_documented_lossy_warnings() {
    // Unmodified Starship "Gruvbox Rainbow" preset: powerline styles, palette
    // aliases, and unsupported modules must degrade to warnings, not fail.
    let artifacts = import("preset_gruvbox_rainbow.toml", "gruvbox-rainbow");

    let template_palette = artifacts.palette.to_template_palette();
    validate_segment_templates(&artifacts.theme, &template_palette)
        .expect("imported theme validates against imported palette");

    assert_eq!(
        artifacts.segments,
        vec![
            "username",
            "directory",
            "git",
            "language",
            "clock",
            "character"
        ]
    );
    assert_eq!(
        artifacts
            .palette
            .colors
            .get("color_blue")
            .map(ColorSpec::as_str),
        Some("#458588")
    );
    assert_eq!(
        artifacts.theme.segments.directory.format.as_deref(),
        Some("[ $path ](fg:color_fg0 bg:color_yellow)")
    );
    // `username` is a script segment: the style's colours, resolved through the
    // palette, draw a pill.
    let username = artifacts
        .theme
        .segments
        .plugin
        .get("username")
        .expect("username segment");
    assert_eq!(
        username.bg_color.as_deref(),
        Some("#d65d0e"),
        "{username:?}"
    );
    assert_eq!(username.text_color.as_deref(), Some("#fbf1c7"));

    // `bg:`-only language style can't apply to the shared format: lossy, not invalid.
    let count = |kind: WarningKind, needle: &str| {
        let matches = |w: &&gpy_agent::import::starship::Warning| {
            w.kind == kind && w.message.contains(needle)
        };
        artifacts.warnings.iter().filter(matches).count()
    };
    assert_eq!(count(WarningKind::LossyMapping, "background style"), 8);
    assert_eq!(count(WarningKind::InvalidColor, ""), 0);

    let unsupported_modules: Vec<&str> = artifacts
        .warnings
        .iter()
        .filter(|w| w.kind == WarningKind::UnsupportedModule)
        .map(|w| w.message.as_str())
        .collect();
    assert_eq!(unsupported_modules.len(), 7, "{unsupported_modules:?}");
    assert!(unsupported_modules.iter().any(|m| m.contains("'os'")));
    assert!(
        !unsupported_modules
            .iter()
            .any(|m| m.contains("'line_break'")),
        "line_break maps to ui.two_line, not a warning"
    );
    assert!(artifacts.theme.ui.two_line);

    // `username`'s `format` (no pill equivalent) and `show_always` (a plugin
    // setting) are each reported.
    assert_eq!(
        count(WarningKind::LossyMapping, "username: format"),
        1,
        "{:?}",
        artifacts.warnings
    );
    assert_eq!(
        count(WarningKind::UnrepresentableOption, "username: show_always"),
        1
    );
    assert_eq!(artifacts.warnings.len(), 34, "pin total warning count");
}

#[test]
fn missing_module_tables_fall_back_to_starship_defaults() {
    // Omitted modules fall back to the builtin `starship` preset, not `None`.
    let artifacts = import("import_character_only.toml", "onlychar");
    let builtin = gpy_agent::theme::parse(
        gpy_agent::config::defaults::STARSHIP_THEME_CONTENT,
        "starship",
    )
    .expect("builtin starship theme parses");

    let imported = &artifacts.theme.segments;
    let preset = &builtin.segments;
    for (segment, got, want) in [
        ("git", &imported.git.format, &preset.git.format),
        (
            "directory",
            &imported.directory.format,
            &preset.directory.format,
        ),
        (
            "duration",
            &imported.duration.format,
            &preset.duration.format,
        ),
        (
            "language",
            &imported.language.format,
            &preset.language.format,
        ),
    ] {
        assert!(got.is_some(), "{segment} format must be set");
        assert_eq!(got, want, "{segment} format must match the builtin preset");
    }
    for segment in ["hostname", "username"] {
        let style = |segments: &gpy_agent::theme::SegmentThemes| {
            toml::to_string(segments.plugin.get(segment).expect("plugin segment"))
                .expect("serialize")
        };
        assert_eq!(
            style(imported),
            style(preset),
            "{segment} style must match the builtin preset"
        );
    }

    assert_eq!(imported.character.success_symbol, ">");

    // Preset language colors reference palette roles that must resolve.
    assert_eq!(
        artifacts
            .palette
            .colors
            .get("orange")
            .map(ColorSpec::as_str),
        Some("202")
    );
    validate_segment_templates(&artifacts.theme, &artifacts.palette.to_template_palette())
        .expect("imported theme validates against its own palette");
}

#[test]
fn git_branch_only_keeps_default_status_group() {
    let model = parse("[git_branch]\nsymbol = \"x\"\n").expect("parse inline snippet");
    let artifacts = build(&model, "branch-only").expect("build import artifacts");
    let format = artifacts
        .theme
        .segments
        .git
        .format
        .expect("git format is set");
    assert!(
        format.contains("$status"),
        "git format lost the default status group: {format}"
    );
}

#[test]
fn directory_read_only_maps_to_read_only_icon() {
    let model = parse("format = \"$directory\"\n[directory]\nread_only = \"RO\"\n").unwrap();
    let artifacts = build(&model, "read-only").expect("build");

    assert_eq!(
        artifacts
            .theme
            .segments
            .directory
            .read_only_icon
            .as_ref()
            .map(gpy_agent::config::types::Icon::as_str),
        Some("RO")
    );
    assert!(
        artifacts
            .theme_toml()
            .expect("theme serializes")
            .contains("read_only_icon = \"RO\""),
    );
}

#[test]
fn directory_without_read_only_leaves_icon_unset() {
    let artifacts = import("import_default.toml", "default-import");
    assert!(artifacts.theme.segments.directory.read_only_icon.is_none());
}
