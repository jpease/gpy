//! Golden import tests: real-ish Starship configs → valid GPY artifacts.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![allow(missing_docs)]

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
    // Language colors folded into the palette as canonical language names.
    assert_eq!(
        artifacts.palette.colors.get("rust").map(ColorSpec::as_str),
        Some("red")
    );

    // Validate with the imported palette so palette-alias colors (e.g. "mauve") resolve.
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
    // Per-language symbol is lossy (lives in [language.icons]); warning contains "symbol".
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
    // The unmodified "Gruvbox Rainbow" preset from Starship's official preset
    // gallery: a powerline config with `bg:`/`fg:` segment styles, custom
    // palette-alias colors, and modules (os, docker_context, conda, pixi, bun,
    // kotlin, haskell, line_break) GPY has no segment for. None of that is
    // representable yet, so the import must degrade to warnings, not fail.
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
    assert_eq!(
        artifacts.theme.segments.username.format.as_deref(),
        Some("[ $username ](bg:color_orange fg:color_fg0)")
    );

    // Every language module shares `style = "bg:color_blue"`: a powerline
    // background token `first_color_token` extracts whole, which `ColorSpec`
    // rejects (it has no `bg:`-prefix stripping and no palette-alias
    // resolution). Pin the count so a future importer improvement that adds
    // either capability shows up here as a visible diff, not silently.
    let invalid_bg_colors = artifacts
        .warnings
        .iter()
        .filter(|w| w.kind == WarningKind::InvalidColor && w.message.contains("bg:color_blue"))
        .count();
    assert_eq!(
        invalid_bg_colors, 8,
        "one per language module sharing style = \"bg:color_blue\""
    );

    let unsupported_modules: Vec<&str> = artifacts
        .warnings
        .iter()
        .filter(|w| w.kind == WarningKind::UnsupportedModule)
        .map(|w| w.message.as_str())
        .collect();
    assert_eq!(unsupported_modules.len(), 8, "{unsupported_modules:?}");
    assert!(unsupported_modules.iter().any(|m| m.contains("'os'")));
    assert!(
        unsupported_modules
            .iter()
            .any(|m| m.contains("'line_break'"))
    );

    assert_eq!(artifacts.warnings.len(), 27, "pin total warning count");
}

#[test]
fn missing_module_tables_fall_back_to_starship_defaults() {
    // #690: a module the source leaves out keeps Starship's default rendering
    // (the builtin `starship` preset), instead of a `None` format that makes
    // git/directory/duration render nothing.
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
            "hostname",
            &imported.hostname.format,
            &preset.hostname.format,
        ),
        (
            "username",
            &imported.username.format,
            &preset.username.format,
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

    // The configured module still overrides the preset.
    assert_eq!(imported.character.success_symbol, ">");

    // The preset's language colors reference palette roles (e.g. swift's
    // `orange`); the emitted palette must resolve them.
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
    // #690: Starship renders `git_status` with its defaults even when only
    // `[git_branch]` is configured.
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
