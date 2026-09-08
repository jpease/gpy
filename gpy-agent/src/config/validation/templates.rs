//! Validates per-segment `format` templates so authoring mistakes surface in
//! `gpy doctor` / `gpy theme` check rather than silently degrading the prompt.

use super::{ValidationError, ValidationErrorKind};
use crate::template::{Palette, RenderContext, VariableResolver, render};
use crate::theme::ThemeConfig;

/// Resolver that returns a non-empty placeholder for every variable, so all
/// conditional branches activate and every styled group is exercised.
struct PlaceholderResolver;

impl VariableResolver for PlaceholderResolver {
    fn resolve(&self, name: &str) -> Option<String> {
        match name {
            // Style-valued vars must resolve to a parseable style string. Otherwise
            // `($style)` indirection makes parse_style("x") -> Palette("x") -> UnknownColor,
            // false-failing validation on the idiomatic Starship pattern that the live
            // path renders correctly.
            "style" => Some("fg:white bg:black".to_owned()),
            // Color-valued vars: bare color tokens used as `fg:$var`/`bg:$var`.
            // Must be a parseable ANSI color for the same reason as `style` above.
            // `bg` is the segment background color injected by powerline format strings
            // (e.g. `[ ](fg:prev_bg bg:$bg)` and `([$sep_close](fg:$bg))`).
            "color" | "bg" => Some("white".to_owned()),
            _ => Some("x".to_owned()),
        }
    }
}

/// Validate every segment `format` in `theme` against `palette`. Returns the first failure, if any.
///
/// A color token that is neither a standard ANSI name nor a `palette` entry produces an
/// `Err` that names the offending segment.
///
/// # Errors
///
/// Returns `Err` describing the offending segment, the unknown color, and the template.
pub fn validate_segment_templates(
    theme: &ThemeConfig,
    palette: &Palette,
) -> Result<(), ValidationError> {
    if let Some(format) = theme.segments.git.format.as_deref() {
        validate_one("git", format, palette)?;
    }
    if let Some(format) = theme.segments.language.format.as_deref() {
        validate_one("language", format, palette)?;
    }
    if let Some(format) = theme.segments.directory.format.as_deref() {
        validate_one("directory", format, palette)?;
    }
    if let Some(format) = theme.segments.duration.format.as_deref() {
        validate_one("duration", format, palette)?;
    }
    if let Some(format) = theme.segments.character.format.as_deref() {
        validate_one("character", format, palette)?;
    }
    Ok(())
}

/// Render `format` against a placeholder resolver and `palette`, and report the first error.
///
/// # Errors
///
/// Returns `Err` with the segment name and template error if rendering fails.
fn validate_one(
    segment: &'static str,
    format: &str,
    palette: &Palette,
) -> Result<(), ValidationError> {
    let resolver = PlaceholderResolver;
    let ctx = RenderContext::new(&resolver).with_palette(palette.clone());
    render(format, &ctx)
        .map(|_| ())
        .map_err(|error| ValidationError {
            field_path: format!("segments.{segment}.format"),
            kind: ValidationErrorKind::TemplateRenderFailed {
                segment,
                template: format.to_owned(),
                source: error,
            },
        })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]

    use super::{ValidationErrorKind, validate_segment_templates};
    use crate::template::Palette;
    use crate::theme::ThemeConfig;

    #[test]
    fn ok_when_no_format() {
        let theme = ThemeConfig::default();
        assert!(validate_segment_templates(&theme, &Palette::default()).is_ok());
    }

    #[test]
    fn ok_for_valid_format() {
        let mut theme = ThemeConfig::default();
        theme.segments.git.format = Some("on [$branch](bold green)".to_owned());
        assert!(validate_segment_templates(&theme, &Palette::default()).is_ok());
    }

    #[test]
    fn err_for_unbalanced_bracket() {
        let mut theme = ThemeConfig::default();
        theme.segments.git.format = Some("on [$branch(green)".to_owned());
        let err = validate_segment_templates(&theme, &Palette::default())
            .unwrap_err()
            .to_string();
        assert!(err.contains("git"));
    }

    #[test]
    fn err_for_unknown_color() {
        let mut theme = ThemeConfig::default();
        theme.segments.git.format = Some("[$branch](fg:nonsuch)".to_owned());
        let err = validate_segment_templates(&theme, &Palette::default())
            .unwrap_err()
            .to_string();
        assert!(err.contains("git"));
        assert!(
            err.contains("nonsuch"),
            "error must name the unknown color: {err}"
        );
    }

    #[test]
    fn ok_for_valid_language_format() {
        let mut theme = ThemeConfig::default();
        theme.segments.language.format = Some("[$symbol $version]($style)".to_owned());
        assert!(validate_segment_templates(&theme, &Palette::default()).is_ok());
    }

    #[test]
    fn err_for_broken_language_format() {
        let mut theme = ThemeConfig::default();
        theme.segments.language.format = Some("[$version(green)".to_owned());
        let err = validate_segment_templates(&theme, &Palette::default())
            .unwrap_err()
            .to_string();
        assert!(err.contains("language"));
    }

    #[test]
    fn ok_for_style_indirection() {
        let mut theme = ThemeConfig::default();
        theme.segments.git.format = Some("on [$symbol$branch]($style)".to_owned());
        assert!(validate_segment_templates(&theme, &Palette::default()).is_ok());
    }

    #[test]
    fn ok_for_valid_directory_format() {
        let mut theme = ThemeConfig::default();
        theme.segments.directory.format = Some("[$path]($style)".to_owned());
        assert!(validate_segment_templates(&theme, &Palette::default()).is_ok());
    }

    #[test]
    fn err_for_broken_directory_format() {
        let mut theme = ThemeConfig::default();
        theme.segments.directory.format = Some("[$path(green)".to_owned());
        let err = validate_segment_templates(&theme, &Palette::default())
            .unwrap_err()
            .to_string();
        assert!(err.contains("directory"));
    }

    #[test]
    fn ok_for_valid_duration_format() {
        let mut theme = ThemeConfig::default();
        theme.segments.duration.format = Some("[$duration]($style)".to_owned());
        assert!(validate_segment_templates(&theme, &Palette::default()).is_ok());
    }

    #[test]
    fn err_for_broken_duration_format() {
        let mut theme = ThemeConfig::default();
        theme.segments.duration.format = Some("[$duration(green)".to_owned());
        let err = validate_segment_templates(&theme, &Palette::default())
            .unwrap_err()
            .to_string();
        assert!(err.contains("duration"));
    }

    #[test]
    fn ok_for_valid_character_format() {
        let mut theme = ThemeConfig::default();
        theme.segments.character.format = Some("[$symbol]($style)".to_owned());
        assert!(validate_segment_templates(&theme, &Palette::default()).is_ok());
    }

    #[test]
    fn err_for_broken_character_format() {
        let mut theme = ThemeConfig::default();
        theme.segments.character.format = Some("[$symbol(green)".to_owned());
        let err = validate_segment_templates(&theme, &Palette::default())
            .unwrap_err()
            .to_string();
        assert!(err.contains("character"));
    }

    #[test]
    fn ok_for_palette_defined_color() {
        use crate::template::Color;
        use std::collections::HashMap;
        let mut theme = ThemeConfig::default();
        theme.segments.git.format = Some("[$branch](fg:accent)".to_owned());
        let mut map = HashMap::new();
        map.insert("accent".to_owned(), Color::Named("blue".to_owned()));
        let palette = Palette::new(map);
        assert!(validate_segment_templates(&theme, &palette).is_ok());
    }

    #[test]
    fn ok_for_powerline_bg_variable_in_format() {
        // `$bg` is a color-valued variable (segment background) used in powerline
        // format strings like `[ ](fg:prev_bg bg:$bg)` and `([$sep_close](fg:$bg))`.
        // The PlaceholderResolver must return a valid color for it so validation
        // does not false-fail with "unknown color: x".
        let mut theme = ThemeConfig::default();
        theme.segments.git.format =
            Some("[ ](fg:prev_bg bg:$bg)[$branch]($style)([$sep_close](fg:$bg))".to_owned());
        assert!(
            validate_segment_templates(&theme, &Palette::default()).is_ok(),
            "powerline $bg format must pass validation"
        );
    }

    #[test]
    fn err_for_color_absent_from_palette_and_standard() {
        let mut theme = ThemeConfig::default();
        theme.segments.git.format = Some("[$branch](fg:accent)".to_owned());
        // Empty palette + `accent` is not a standard name → must fail.
        let err = validate_segment_templates(&theme, &Palette::default())
            .unwrap_err()
            .to_string();
        assert!(err.contains("git"), "error must name the segment: {err}");
        assert!(
            err.contains("accent"),
            "error must name the unknown color: {err}"
        );
    }

    /// The structured error must still render byte-for-byte the wording
    /// validation failures have always had (#600). `Error::config` stringifies
    /// this, so a change here is user-visible in `gpy theme check`.
    #[test]
    fn display_text_is_unchanged_by_the_structured_error() {
        let mut theme = ThemeConfig::default();
        theme.segments.git.format = Some("[$branch](fg:nonsuch)".to_owned());

        let err = validate_segment_templates(&theme, &Palette::default()).unwrap_err();

        assert_eq!(
            err.to_string(),
            "segment 'git': unknown color: nonsuch (template: [$branch](fg:nonsuch))"
        );
    }

    /// The point of the struct: callers can locate the failing field without
    /// parsing the message.
    #[test]
    fn field_path_locates_the_failing_segment_format() {
        let mut theme = ThemeConfig::default();
        theme.segments.duration.format = Some("[$duration](fg:nonsuch)".to_owned());

        let err = validate_segment_templates(&theme, &Palette::default()).unwrap_err();

        assert_eq!(err.field_path, "segments.duration.format");
        let ValidationErrorKind::TemplateRenderFailed {
            segment, template, ..
        } = &err.kind
        else {
            panic!("expected TemplateRenderFailed, got {:?}", err.kind);
        };
        assert_eq!(*segment, "duration");
        assert_eq!(template, "[$duration](fg:nonsuch)");
    }
}
