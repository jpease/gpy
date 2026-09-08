//! Assemble translated fragments into serialized GPY palette + theme TOML.

use crate::import::starship::model::StarshipConfig;
use crate::import::starship::modules::{
    translate_character, translate_directory, translate_directory_layout, translate_duration,
    translate_git, translate_hostname, translate_languages, translate_time, translate_username,
};
use crate::import::starship::palette::{selected_palette, translate_palette};
use crate::import::starship::{ImportError, Result, Warnings, layout};
use crate::palette::config::PaletteConfig;
use crate::theme::{RecommendedDirectory, RecommendedUi, SegmentThemes, ThemeConfig, UiTheme};

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
#[must_use]
pub fn build(model: &StarshipConfig, name: &str) -> ImportArtifacts {
    let mut warnings = Warnings::new();

    let language = translate_languages(model, &mut warnings);
    let mut segments = SegmentThemes {
        language: language.theme,
        ..Default::default()
    };
    if let Some(table) = model.module_table("directory") {
        segments.directory = translate_directory(table, &mut warnings);
    }
    let git = translate_git(
        model.module_table("git_branch"),
        model.module_table("git_status"),
        model.module_table("git_state"),
        &mut warnings,
    );
    segments.git = git;
    if let Some(table) = model.module_table("cmd_duration") {
        segments.duration = translate_duration(table, &mut warnings);
    }
    if let Some(table) = model.module_table("character") {
        segments.character = translate_character(table, &mut warnings);
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

    let ui = build_ui(model.module_table("directory"), &mut warnings);
    let theme = ThemeConfig { ui, segments };

    let selected = selected_palette(model, &mut warnings);
    let palette = translate_palette(name, selected, &language.palette_colors, &mut warnings);

    let segments_order = model
        .format
        .as_deref()
        .map(|format| layout::derive_segments(format, &mut warnings))
        .unwrap_or_default();

    ImportArtifacts {
        theme,
        palette,
        segments: segments_order,
        warnings,
        palette_name: name.to_owned(),
    }
}

/// A flat UI (transparent delimiters), approximating Starship's default look.
fn flat_ui() -> UiTheme {
    UiTheme::default()
}

/// Build the theme's `[ui]` block, including an advisory
/// `[ui.recommended.directory]` truncation-layout block when `directory_table`
/// (Starship's `[directory]` module, if present) specifies one.
fn build_ui(directory_table: Option<&toml::value::Table>, warnings: &mut Warnings) -> UiTheme {
    let mut ui = flat_ui();
    let Some(table) = directory_table else {
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

    use super::build;
    use crate::config::types::ColorSpec;
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
        let artifacts = build(&model, "demo");

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
            Some("[$path](bold cyan)[$read_only](bold red) ")
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
        let artifacts = build(&model, "demo");
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
        let artifacts = build(&model, "demo");
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
        let artifacts = build(&model, "demo");
        assert_eq!(
            artifacts.segments,
            vec!["directory", "git", "language", "duration", "character"]
        );
    }
}
