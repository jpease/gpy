//! Assemble translated fragments into serialized GPY palette + theme TOML.

use crate::config::LanguageTheme;
use crate::config::defaults::{STARSHIP_PALETTE_CONTENT, STARSHIP_THEME_CONTENT};
use crate::import::starship::model::StarshipConfig;
use crate::import::starship::modules::{
    translate_character, translate_directory, translate_directory_layout, translate_duration,
    translate_git, translate_hostname, translate_languages, translate_time, translate_username,
};
use crate::import::starship::palette::{selected_palette, translate_palette};
use crate::import::starship::{ImportError, Result, Warnings, layout};
use crate::palette::config::PaletteConfig;
use crate::template::{Color, parse_color};
use crate::theme::{
    DirectoryTheme, DurationTheme, GitTheme, RecommendedDirectory, RecommendedUi, SegmentThemes,
    ThemeConfig, UiTheme,
};

/// The translated output of an import: typed values, ready to serialize or
/// inspect directly.
#[derive(Debug, Clone)]
pub struct ImportArtifacts {
    /// The translated theme, ready to serialize or inspect directly.
    pub theme: ThemeConfig,
    /// The translated palette, ready to serialize or inspect directly.
    pub palette: PaletteConfig,
    /// Recommended `enabled_segments` order.
    pub segments: Vec<String>,
    /// Non-fatal warnings collected during translation.
    pub warnings: Warnings,
    /// The artifact base name.
    pub palette_name: String,
}

impl ImportArtifacts {
    /// Serialize the theme to TOML with a provenance header.
    ///
    /// # Errors
    ///
    /// Returns [`ImportError::Parse`] if `theme` fails to serialize (should
    /// not happen for a well-formed `ThemeConfig`, but `toml::to_string` is
    /// fallible).
    pub fn theme_toml(&self) -> Result<String> {
        let body = toml::to_string(&self.theme).map_err(|error| ImportError::Parse {
            message: error.to_string(),
        })?;
        Ok(format!("{}{body}", header("theme", &self.palette_name)))
    }

    /// Serialize the palette to TOML with a provenance header.
    ///
    /// # Errors
    ///
    /// Returns [`ImportError::Parse`] if `palette` fails to serialize.
    pub fn palette_toml(&self) -> Result<String> {
        let body = toml::to_string(&self.palette).map_err(|error| ImportError::Parse {
            message: error.to_string(),
        })?;
        Ok(format!("{}{body}", header("palette", &self.palette_name)))
    }
}

/// Translate `model` into palette + theme artifacts named `name`.
///
/// Every segment starts from the builtin `starship` preset, which encodes
/// Starship's default rendering; a module the source configures replaces its
/// segment, and a module the source leaves out keeps the preset's look.
///
/// # Errors
///
/// Returns [`ImportError::Preset`] if the embedded `starship` preset theme or
/// palette fails to parse.
pub fn build(model: &StarshipConfig, name: &str) -> Result<ImportArtifacts> {
    let mut warnings = Warnings::new();
    let preset = load_preset()?;

    let selected = selected_palette(model, &mut warnings);
    let mut segments = preset.segments;
    let language = translate_languages(
        model,
        std::mem::take(&mut segments.language),
        selected,
        &mut warnings,
    );
    segments.language = language.theme;
    overlay_preset_modules(&mut segments, model, &mut warnings);
    if let Some(table) = model.module_table("character") {
        segments.character = translate_character(table, selected, &mut warnings);
    }
    if let Some(table) = model.module_table("time") {
        segments.clock = translate_time(table, &mut warnings);
    }
    if let Some(table) = model.module_table("hostname") {
        segments.hostname = translate_hostname(table, &mut warnings);
    }
    if let Some(table) = model.module_table("username") {
        segments.username = translate_username(table, &mut warnings);
    }

    let ui = build_ui(model, &mut warnings);
    let theme = ThemeConfig { ui, segments };

    let mut palette = translate_palette(name, selected, &language.palette_colors, &mut warnings);
    add_preset_language_roles(&mut palette, &theme.segments.language, &preset.palette);

    let segments_order = model
        .format
        .as_deref()
        .map(|format| layout::derive_segments(format, &mut warnings))
        .unwrap_or_default();

    Ok(ImportArtifacts {
        theme,
        palette,
        segments: segments_order,
        warnings,
        palette_name: name.to_owned(),
    })
}

/// Overlay the `directory`, git and `cmd_duration` modules `model` configures
/// onto the preset `segments`.
///
/// A configured module replaces what Starship's own config controls (the
/// template, and the duration threshold); everything Starship has no key for
/// (colors, git status icons, `show_counts`) keeps the preset's value, so a
/// partial table never resets it to a GPY default.
fn overlay_preset_modules(
    segments: &mut SegmentThemes,
    model: &StarshipConfig,
    warnings: &mut Warnings,
) {
    if let Some(table) = model.module_table("directory") {
        let imported = translate_directory(table, warnings);
        segments.directory = DirectoryTheme {
            format: imported.format,
            ..std::mem::take(&mut segments.directory)
        };
    }
    let git_branch = model.module_table("git_branch");
    let git_status = model.module_table("git_status");
    let git = translate_git(
        git_branch,
        git_status,
        model.module_table("git_state"),
        warnings,
    );
    if git_branch.is_some() || git_status.is_some() {
        segments.git = GitTheme {
            format: git.format,
            ..std::mem::take(&mut segments.git)
        };
    }
    if let Some(table) = model.module_table("cmd_duration") {
        let imported = translate_duration(table, warnings);
        segments.duration = DurationTheme {
            format: imported.format,
            show_if_exceeds_ms: imported.show_if_exceeds_ms,
            show_milliseconds: imported.show_milliseconds,
            ..std::mem::take(&mut segments.duration)
        };
    }
}

/// The builtin `starship` preset's segments and palette: the import baseline.
struct Preset {
    /// Segment themes encoding Starship's default rendering per module.
    segments: SegmentThemes,
    /// The palette that resolves the color roles those segments reference.
    palette: PaletteConfig,
}

/// Parse the embedded `starship` preset theme and palette.
///
/// Only the preset's `[segments]` are kept: its `[ui]` (including
/// `[ui.recommended]`, which would recommend `palette = "starship"`) is not
/// part of an import.
///
/// # Errors
///
/// Returns [`ImportError::Preset`] if either embedded file fails to parse.
fn load_preset() -> Result<Preset> {
    let theme = crate::theme::parse(STARSHIP_THEME_CONTENT, "starship").map_err(|error| {
        ImportError::Preset {
            message: error.to_string(),
        }
    })?;
    let palette: PaletteConfig =
        toml::from_str(STARSHIP_PALETTE_CONTENT).map_err(|error| ImportError::Preset {
            message: error.to_string(),
        })?;
    Ok(Preset {
        segments: theme.segments,
        palette,
    })
}

/// Add the preset palette's value for every palette role (e.g. `orange`,
/// `bright_magenta`) that a `language` color references but `palette` lacks.
///
/// The preset's per-language colors use palette roles rather than literal
/// colors; without these entries the template engine rejects the role as an
/// unknown color and the language segment renders nothing.
fn add_preset_language_roles(
    palette: &mut PaletteConfig,
    language: &LanguageTheme,
    preset_palette: &PaletteConfig,
) {
    for spec in language.overrides.values() {
        if let Ok(Color::Palette(role)) = parse_color(spec.as_str())
            && !palette.colors.contains_key(&role)
            && let Some(value) = preset_palette.colors.get(&role)
        {
            palette.colors.insert(role, value.clone());
        }
    }
}

/// A flat UI (transparent delimiters), approximating Starship's default look.
fn flat_ui() -> UiTheme {
    UiTheme::default()
}

/// Build the theme's `[ui]` block.
///
/// `add_newline` defaults to `true`; the layout is two-line when `format` is
/// absent (Starship's default `$all` ends in `$line_break$character`) or
/// references `$line_break`. An advisory `[ui.recommended.directory]`
/// truncation-layout block is added when the `[directory]` module specifies one.
fn build_ui(model: &StarshipConfig, warnings: &mut Warnings) -> UiTheme {
    let mut ui = flat_ui();
    ui.add_newline = model.add_newline.unwrap_or(true);
    ui.two_line = model.format.as_deref().is_none_or(layout::has_line_break);
    let Some(table) = model.module_table("directory") else {
        return ui;
    };
    let layout = translate_directory_layout(table, warnings);
    if layout != RecommendedDirectory::default() {
        ui.recommended = Some(RecommendedUi {
            directory: Some(layout),
            ..Default::default()
        });
    }
    ui
}

/// Provenance header comment for an emitted artifact.
fn header(kind: &str, name: &str) -> String {
    format!(
        "# GPY {kind} '{name}' imported from starship.toml by `gpy theme import`.\n# Edit freely; re-import to regenerate.\n\n"
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    use super::{ImportArtifacts, build};
    use crate::config::types::ColorSpec;
    use crate::import::starship::WarningKind;
    use crate::import::starship::model::parse;

    const SAMPLE: &str = r##"
format = "$directory$git_branch$git_status$rust$cmd_duration$character"
palette = "demo"

[palettes.demo]
accent = "#88c0d0"

[directory]
format = "[$path]($style)[$read_only]($read_only_style) "
style = "bold cyan"

[git_branch]
symbol = " "
style = "bold purple"

[git_status]
style = "bold red"

[rust]
symbol = " "
style = "bold red"

[cmd_duration]
min_time = 2000
style = "bold yellow"

[character]
success_symbol = "[❯](bold green)"
error_symbol = "[❯](bold red)"
"##;

    #[test]
    fn build_emits_parseable_palette_and_theme() {
        let model = parse(SAMPLE).unwrap();
        let artifacts = build(&model, "demo").expect("build");

        // Serialized-text checks: the provenance header is only observable in
        // the rendered TOML.
        assert!(
            artifacts
                .palette_toml()
                .expect("palette serializes")
                .contains("imported from starship.toml")
        );
        assert!(
            artifacts
                .theme_toml()
                .expect("theme serializes")
                .contains("imported from starship.toml")
        );

        // Typed checks: compare the translated values directly, no re-parse.
        assert_eq!(
            artifacts.theme.segments.directory.format.as_deref(),
            Some("[$path](bold cyan)[$read_only](red) ")
        );
        assert_eq!(
            artifacts.theme.segments.git.format.as_deref(),
            Some(r"on [$symbol$branch](bold purple) ([\[$status$ahead_behind\]](bold red) )")
        );
        assert_eq!(
            artifacts.theme.segments.duration.show_if_exceeds_ms,
            2000_u64
        );
        assert_eq!(
            artifacts.theme.segments.character.success_color.as_str(),
            "green"
        );

        assert_eq!(
            artifacts
                .palette
                .colors
                .get("accent")
                .map(ColorSpec::as_str),
            Some("#88c0d0")
        );
        assert_eq!(
            artifacts.palette.colors.get("rust").map(ColorSpec::as_str),
            Some("red")
        );
    }

    #[test]
    fn build_carries_recommended_directory_layout() {
        let source = "\n[directory]\ntruncation_length = 3\ntruncation_symbol = \"…\"\n";
        let model = parse(source).unwrap();
        let artifacts = build(&model, "demo").expect("build");
        let theme_toml = artifacts.theme_toml().expect("theme serializes");
        assert!(
            theme_toml.contains("[ui.recommended.directory]"),
            "theme_toml missing recommended directory block: {theme_toml}"
        );
        assert!(
            theme_toml.contains("truncation_length = 3"),
            "theme_toml missing truncation_length: {theme_toml}"
        );
        assert!(
            theme_toml.contains("truncation_symbol = \"…\""),
            "theme_toml missing truncation_symbol: {theme_toml}"
        );

        // Typed check: compare the recommendation directly, no re-parse.
        let recommended = artifacts
            .theme
            .ui
            .recommended
            .as_ref()
            .expect("recommended block present")
            .directory
            .as_ref()
            .expect("recommended directory present");
        assert_eq!(
            recommended
                .truncation_length
                .map(crate::config::types::DirectoryTruncationLength::get),
            Some(3_usize)
        );
        assert_eq!(
            recommended
                .truncation_symbol
                .as_ref()
                .map(crate::config::types::DirectoryTruncationSymbol::as_str),
            Some("…")
        );
    }

    #[test]
    fn build_recommends_truncated_display_without_explicit_truncation_settings() {
        // #355: the recommendation is always emitted whenever a `[directory]`
        // table is present, since Starship's real default behavior (truncate
        // to trailing path components, anchor at the repo root) applies even
        // when the source sets neither `truncation_length` nor
        // `truncation_symbol` explicitly — SAMPLE's `[directory]` table only
        // sets `format`/`style`, matching that case.
        let model = parse(SAMPLE).unwrap();
        let artifacts = build(&model, "demo").expect("build");
        let theme_toml = artifacts.theme_toml().expect("theme serializes");
        assert!(
            theme_toml.contains("[ui.recommended.directory]"),
            "recommendation should be emitted even without explicit truncation settings: {theme_toml}"
        );
        assert!(
            theme_toml.contains(r#"display = "truncated""#),
            "recommendation should default display to truncated: {theme_toml}"
        );
        assert!(
            theme_toml.contains("truncate_to_repo = true"),
            "recommendation should default truncate_to_repo to true: {theme_toml}"
        );
        assert!(
            !theme_toml.contains("truncation_length"),
            "truncation_length should stay absent when Starship doesn't set it: {theme_toml}"
        );
    }

    #[test]
    fn build_derives_recommended_segments() {
        let model = parse(SAMPLE).unwrap();
        let artifacts = build(&model, "demo").expect("build");
        assert_eq!(
            artifacts.segments,
            vec!["directory", "git", "language", "duration", "character"]
        );
    }

    #[test]
    fn add_newline_false_is_carried_into_ui() {
        let model = parse("add_newline = false\n").unwrap();
        let artifacts = build(&model, "demo").expect("build");
        assert!(!artifacts.theme.ui.add_newline);
    }

    #[test]
    fn add_newline_defaults_to_true() {
        let model = parse("[character]\n").unwrap();
        let artifacts = build(&model, "demo").expect("build");
        assert!(artifacts.theme.ui.add_newline);
    }

    #[test]
    fn line_break_sets_two_line_without_warning() {
        let model = parse("format = \"$directory$line_break$character\"\n").unwrap();
        let artifacts = build(&model, "demo").expect("build");
        assert!(artifacts.theme.ui.two_line);
        assert!(
            artifacts
                .warnings
                .iter()
                .all(|warning| !warning.message.contains("line_break")),
            "{:?}",
            artifacts.warnings
        );
        assert_eq!(artifacts.segments, vec!["directory", "character"]);
    }

    #[test]
    fn missing_format_defaults_to_two_line() {
        let model = parse("[character]\n").unwrap();
        let artifacts = build(&model, "demo").expect("build");
        assert!(artifacts.theme.ui.two_line);
    }

    #[test]
    fn format_without_line_break_is_single_line() {
        let model = parse("format = \"$directory$character\"\n").unwrap();
        let artifacts = build(&model, "demo").expect("build");
        assert!(!artifacts.theme.ui.two_line);
    }

    #[test]
    fn escaped_line_break_does_not_set_two_line() {
        let model = parse("format = \"$directory\\\\$line_break$character\"\n").unwrap();
        let artifacts = build(&model, "demo").expect("build");
        assert!(!artifacts.theme.ui.two_line);
    }

    #[test]
    fn build_keeps_preset_fields_starship_has_no_key_for_in_partial_tables() {
        // #734: a present `[git_branch]` / `[directory]` / `[cmd_duration]` table
        // replaces only the template (and the duration threshold); the preset's
        // git status icons, `show_counts = false` and segment colors survive.
        let model = parse(
            "[git_branch]\nsymbol = \"b \"\n[directory]\ntruncation_length = 3\n[cmd_duration]\nmin_time = 500\n",
        )
        .unwrap();
        let theme = build(&model, "demo").expect("build").theme;
        let git = &theme.segments.git;
        assert_eq!(git.show_counts, Some(false));
        assert_eq!(git.unstaged_icon.as_deref(), Some("!"));
        assert_eq!(git.staged_icon.as_deref(), Some("+"));
        assert_eq!(git.untracked_icon.as_deref(), Some("?"));
        assert_eq!(git.conflicts_icon.as_deref(), Some("="));
        assert_eq!(git.text_color.as_str(), "magenta");
        assert_eq!(theme.segments.directory.text_color.as_str(), "cyan");
        assert_eq!(theme.segments.directory.bg_color.as_str(), "transparent");
        assert_eq!(theme.segments.duration.text_color.as_str(), "yellow");
        assert_eq!(theme.segments.duration.bg_color.as_str(), "transparent");
        assert_eq!(theme.segments.duration.show_if_exceeds_ms, 500_u64);
    }

    fn language_override<'a>(artifacts: &'a ImportArtifacts, key: &str) -> Option<&'a str> {
        artifacts
            .theme
            .segments
            .language
            .overrides
            .get(key)
            .map(ColorSpec::as_str)
    }

    fn invalid_color_warnings(artifacts: &ImportArtifacts) -> Vec<&str> {
        artifacts
            .warnings
            .iter()
            .filter(|w| w.kind == WarningKind::InvalidColor)
            .map(|w| w.message.as_str())
            .collect()
    }

    fn lossy_warnings(artifacts: &ImportArtifacts) -> Vec<&str> {
        artifacts
            .warnings
            .iter()
            .filter(|w| w.kind == WarningKind::LossyMapping)
            .map(|w| w.message.as_str())
            .collect()
    }

    #[test]
    fn language_fg_prefixed_style_is_accepted() {
        let model = parse("[nodejs]\nstyle = \"fg:green\"\n").unwrap();
        let artifacts = build(&model, "demo").expect("build");
        assert_eq!(
            language_override(&artifacts, "node_bg_color"),
            Some("green")
        );
        assert!(
            invalid_color_warnings(&artifacts).is_empty(),
            "{:?}",
            artifacts.warnings
        );
    }

    #[test]
    fn language_palette_alias_resolves_via_selected_palette() {
        let model = parse(
            "palette = \"p\"\n[python]\nstyle = \"bold peach\"\n[palettes.p]\npeach = \"#fab387\"\n",
        )
        .unwrap();
        let artifacts = build(&model, "demo").expect("build");
        assert_eq!(
            language_override(&artifacts, "python_bg_color"),
            Some("#fab387")
        );
        assert_eq!(
            artifacts.palette.colors.get("peach").map(ColorSpec::as_str),
            Some("#fab387")
        );
        assert!(invalid_color_warnings(&artifacts).is_empty());
    }

    #[test]
    fn language_bg_only_style_is_lossy_not_invalid() {
        let model = parse("[rust]\nstyle = \"bg:#212736\"\n").unwrap();
        let artifacts = build(&model, "demo").expect("build");
        assert!(invalid_color_warnings(&artifacts).is_empty());
        let lossy = lossy_warnings(&artifacts);
        assert_eq!(lossy.len(), 1_usize, "{lossy:?}");
        assert!(lossy.iter().all(|m| m.contains("language rust")));
        // The bg is not applied: the color stays the preset's, as without the module.
        let untouched = build(&parse("").unwrap(), "demo").expect("build");
        assert_eq!(
            artifacts
                .theme
                .segments
                .language
                .overrides
                .get("rust_bg_color"),
            untouched
                .theme
                .segments
                .language
                .overrides
                .get("rust_bg_color")
        );
    }

    #[test]
    fn language_own_format_is_reported_lossy() {
        let model = parse(
            "[nodejs]\nstyle = \"fg:green\"\nformat = \"via [$symbol($version )]($style)\"\n",
        )
        .unwrap();
        let artifacts = build(&model, "demo").expect("build");
        let lossy = lossy_warnings(&artifacts);
        assert_eq!(lossy.len(), 1_usize, "{lossy:?}");
        assert!(lossy.iter().all(|m| m.contains("language nodejs")));
        assert!(lossy.iter().all(|m| m.contains("format")));
    }

    #[test]
    fn language_fg_and_bg_style_keeps_fg_and_warns_about_bg() {
        let model = parse("[rust]\nstyle = \"bg:blue fg:white\"\n").unwrap();
        let artifacts = build(&model, "demo").expect("build");
        assert_eq!(
            language_override(&artifacts, "rust_bg_color"),
            Some("white")
        );
        assert_eq!(lossy_warnings(&artifacts).len(), 1_usize);
        assert!(invalid_color_warnings(&artifacts).is_empty());
    }

    #[test]
    fn character_fg_prefixed_style_is_accepted() {
        let model = parse("[character]\nsuccess_symbol = \"[❯](fg:green)\"\n").unwrap();
        let artifacts = build(&model, "demo").expect("build");
        assert_eq!(
            artifacts.theme.segments.character.success_color.as_str(),
            "green"
        );
        assert!(invalid_color_warnings(&artifacts).is_empty());
    }

    #[test]
    fn character_palette_alias_resolves_via_selected_palette() {
        let model = parse(
            "palette = \"p\"\n[character]\nsuccess_symbol = \"[❯](bold peach)\"\nerror_symbol = \"[✗](fg:peach)\"\n[palettes.p]\npeach = \"#fab387\"\n",
        )
        .unwrap();
        let artifacts = build(&model, "demo").expect("build");
        let character = &artifacts.theme.segments.character;
        assert_eq!(character.success_color.as_str(), "#fab387");
        assert_eq!(character.error_color.as_str(), "#fab387");
        assert!(invalid_color_warnings(&artifacts).is_empty());
    }

    #[test]
    fn character_shared_background_lands_in_the_format() {
        let model = parse(
            "[character]\nsuccess_symbol = \"[❯](bg:blue fg:white)\"\nerror_symbol = \"[✗](bg:blue fg:red)\"\n",
        )
        .unwrap();
        let artifacts = build(&model, "demo").expect("build");
        let character = &artifacts.theme.segments.character;
        assert_eq!(character.success_color.as_str(), "white");
        assert_eq!(
            character.format.as_deref(),
            Some("[$symbol]($style bg:blue) ")
        );
        assert!(invalid_color_warnings(&artifacts).is_empty());
    }

    #[test]
    fn character_diverging_background_is_lossy() {
        let model =
            parse("[character]\nsuccess_symbol = \"[❯](bold bg:blue fg:white)\"\n").unwrap();
        let artifacts = build(&model, "demo").expect("build");
        assert_eq!(
            artifacts.theme.segments.character.format.as_deref(),
            Some("[$symbol](bold $style) ")
        );
        assert_eq!(lossy_warnings(&artifacts).len(), 1_usize);
    }
}
