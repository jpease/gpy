//! Validates per-segment `format` templates so mistakes surface in `gpy doctor`.

use std::collections::HashMap;

use super::{ValidationError, ValidationErrorKind};
use crate::template::{Palette, RenderContext, VariableResolver, check_template_controls, render};
use crate::theme::ThemeConfig;

/// Resolver that returns a non-empty placeholder for every variable, so all
/// conditional branches activate and every styled group is exercised.
struct PlaceholderResolver;

impl VariableResolver for PlaceholderResolver {
    fn resolve(&self, name: &str) -> Option<String> {
        match name {
            // Must parse as a style, else `($style)` falsely fails as an unknown color.
            "style" => Some("fg:white bg:black".to_owned()),
            // Color-valued vars (`fg:$var`, powerline `bg:$bg`) must be parseable colors.
            "color" | "bg" => Some("white".to_owned()),
            _ => Some("x".to_owned()),
        }
    }
}

/// Validate every segment `format` in `theme` against `palette`. Returns the first failure, if any.
///
/// Unknown color tokens (not ANSI, not in `palette`) fail with the segment name.
/// The first of [`segment_template_errors`].
///
/// # Errors
/// Returns `Err` describing the offending segment, the unknown color, and the template.
pub fn validate_segment_templates(
    theme: &ThemeConfig,
    palette: &Palette,
) -> Result<(), ValidationError> {
    segment_template_errors(theme, palette)
        .into_iter()
        .next()
        .map_or(Ok(()), Err)
}

/// Every segment `format` and style value of `theme` that fails against
/// `palette`, formats first, in a fixed order. Empty when the theme is valid.
#[must_use]
pub fn segment_template_errors(theme: &ThemeConfig, palette: &Palette) -> Vec<ValidationError> {
    let s = &theme.segments;
    let mut errors = Vec::new();
    for (name, field, format) in [
        ("git", "format", &s.git.format),
        ("language", "format", &s.language.format),
        ("directory", "format", &s.directory.format),
        ("duration", "format", &s.duration.format),
        ("character", "format", &s.character.format),
        ("clock", "format", &s.clock.format),
        ("status", "format", &s.status.format),
    ] {
        if let Some(template) = format.as_deref()
            && let Err(error) = validate_one(name, field, template, palette)
        {
            errors.push(error);
        }
    }
    validate_style_values(
        "language",
        "segments.language",
        &theme.segments.language.styles,
        palette,
        &mut errors,
    );
    validate_style_values(
        "git",
        "segments.git.git_style",
        &theme.segments.git.git_style,
        palette,
        &mut errors,
    );
    errors
}

/// Validate free-form style strings by rendering each as a one-group template,
/// pushing each failure onto `errors`.
///
/// Keys are visited in sorted order so the reported failures are stable; each
/// names `<prefix>.<key>`.
fn validate_style_values(
    segment: &'static str,
    prefix: &str,
    styles: &HashMap<String, String>,
    palette: &Palette,
    errors: &mut Vec<ValidationError>,
) {
    let resolver = PlaceholderResolver;
    let ctx = RenderContext::new(&resolver).with_palette(palette.clone());
    let mut entries: Vec<(&String, &String)> = styles.iter().collect();
    entries.sort_unstable();
    for (key, value) in entries {
        let checked = check_template_controls(value)
            .and_then(|()| render(&format!("[x]({value})"), &ctx).map(drop));
        if let Err(error) = checked {
            errors.push(ValidationError {
                field_path: format!("{prefix}.{key}"),
                kind: ValidationErrorKind::TemplateRenderFailed {
                    segment,
                    template: escape_controls(value),
                    source: error,
                },
            });
        }
    }
}

/// Reject terminal controls in `format`, render it against a placeholder
/// resolver and `palette`, and report the first error.
///
/// # Errors
/// Returns `Err` with the segment name and template error if either check fails.
fn validate_one(
    segment: &'static str,
    field: &str,
    format: &str,
    palette: &Palette,
) -> Result<(), ValidationError> {
    let resolver = PlaceholderResolver;
    let ctx = RenderContext::new(&resolver).with_palette(palette.clone());
    check_template_controls(format)
        .and_then(|()| render(format, &ctx).map(drop))
        .map_err(|error| ValidationError {
            field_path: format!("segments.{segment}.{field}"),
            kind: ValidationErrorKind::TemplateRenderFailed {
                segment,
                template: escape_controls(format),
                source: error,
            },
        })
}

/// Echo `text` with control characters as `\u{…}` so an error message never replays them.
fn escape_controls(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_control() {
            escaped.extend(ch.escape_unicode());
        } else {
            escaped.push(ch);
        }
    }
    escaped
}

#[cfg(test)]
mod tests {

    use super::{ValidationErrorKind, segment_template_errors, validate_segment_templates};
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
    fn segment_template_errors_reports_every_failing_segment() {
        let mut theme = ThemeConfig::default();
        theme.segments.git.format = Some("[$branch](fg:nonsuch)".to_owned());
        theme.segments.directory.format = Some("[$path](fg:alsonope)".to_owned());

        let errors = segment_template_errors(&theme, &Palette::default());

        let paths: Vec<&str> = errors.iter().map(|e| e.field_path.as_str()).collect();
        assert_eq!(paths, ["segments.git.format", "segments.directory.format"]);
        assert_eq!(
            validate_segment_templates(&theme, &Palette::default())
                .unwrap_err()
                .field_path,
            "segments.git.format",
            "the validating form still stops at the first failure"
        );
    }

    #[test]
    fn err_for_terminal_control_in_format_without_echoing_it() {
        let mut theme = ThemeConfig::default();
        theme.segments.directory.format = Some("\u{1b}]0;title\u{7}$path".to_owned());
        let err = validate_segment_templates(&theme, &Palette::default()).unwrap_err();
        assert_eq!(err.field_path, "segments.directory.format");
        assert!(matches!(
            err.kind,
            ValidationErrorKind::TemplateRenderFailed {
                source: crate::template::TemplateError::ControlCharacter { code: 0x1b, .. },
                ..
            }
        ));
        let message = err.to_string();
        assert!(
            !message.chars().any(char::is_control),
            "message replays control characters: {message:?}"
        );
    }

    #[test]
    fn ok_for_newline_in_format() {
        let mut theme = ThemeConfig::default();
        theme.segments.directory.format = Some("[$path](bold cyan)\n".to_owned());
        assert!(validate_segment_templates(&theme, &Palette::default()).is_ok());
    }

    #[test]
    fn err_for_terminal_control_in_style_value() {
        let mut theme = ThemeConfig::default();
        theme
            .segments
            .git
            .git_style
            .insert("conflicts".into(), "fg:red\u{7}".into());
        let err = validate_segment_templates(&theme, &Palette::default()).unwrap_err();
        assert_eq!(err.field_path, "segments.git.git_style.conflicts");
        assert!(!err.to_string().chars().any(char::is_control));
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
    fn err_for_invalid_language_style_override() {
        let mut theme = ThemeConfig::default();
        theme.segments.language.format = Some("[$symbol]($attr fg:$color)".into());
        theme
            .segments
            .language
            .styles
            .insert("rust_style".into(), "fg:#12".into());
        let err = validate_segment_templates(&theme, &Palette::default()).unwrap_err();
        assert_eq!(err.field_path, "segments.language.rust_style");
    }

    #[test]
    fn err_for_invalid_git_style_value() {
        let mut theme = ThemeConfig::default();
        theme
            .segments
            .git
            .git_style
            .insert("conflicts".into(), "fg:#12".into());
        let err = validate_segment_templates(&theme, &Palette::default()).unwrap_err();
        assert_eq!(err.field_path, "segments.git.git_style.conflicts");
    }

    #[test]
    fn ok_for_empty_and_attribute_only_language_styles() {
        let mut theme = ThemeConfig::default();
        theme.segments.language.format = Some("[$symbol]($attr fg:$color)".into());
        theme
            .segments
            .language
            .styles
            .insert("rust_style".into(), String::new());
        theme
            .segments
            .language
            .styles
            .insert("java_style".into(), "dimmed".into());
        assert!(validate_segment_templates(&theme, &Palette::default()).is_ok());
    }

    #[test]
    fn every_builtin_theme_validates_against_every_builtin_palette() {
        use crate::palette::config::PaletteConfig;
        use crate::palette::manager::BUILTIN_PALETTES;
        use crate::theme::manager::BUILTIN_THEMES;

        for (theme_name, theme_content) in BUILTIN_THEMES {
            let theme: ThemeConfig = toml::from_str(theme_content).expect("builtin theme parses");
            for (palette_name, palette_content) in BUILTIN_PALETTES {
                let palette: PaletteConfig =
                    toml::from_str(palette_content).expect("builtin palette parses");
                if let Err(error) =
                    validate_segment_templates(&theme, &palette.to_template_palette())
                {
                    panic!("theme {theme_name} x palette {palette_name}: {error}");
                }
            }
        }
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
        // `$bg` must resolve to a valid color or validation false-fails.
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

    /// Wording is user-visible in `gpy theme check`.
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

    /// Callers can locate the failing field without parsing the message.
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

    #[test]
    fn ok_for_clock_none_and_valid() {
        let mut theme = ThemeConfig::default();
        assert!(theme.segments.clock.format.is_none());
        assert!(validate_segment_templates(&theme, &Palette::default()).is_ok());

        theme.segments.clock.format = Some("[$time](fg:white)".to_owned());
        assert!(validate_segment_templates(&theme, &Palette::default()).is_ok());
    }

    #[test]
    fn err_for_broken_clock_format() {
        let mut theme = ThemeConfig::default();
        theme.segments.clock.format = Some("[$time(white)".to_owned());
        let syntax_err = validate_segment_templates(&theme, &Palette::default()).unwrap_err();
        assert_eq!(syntax_err.field_path, "segments.clock.format");
        assert!(syntax_err.to_string().contains("clock"));

        theme.segments.clock.format = Some("[$time](fg:nonsuch)".to_owned());
        let color_err = validate_segment_templates(&theme, &Palette::default()).unwrap_err();
        assert_eq!(color_err.field_path, "segments.clock.format");
        assert!(color_err.to_string().contains("clock"));
        assert!(color_err.to_string().contains("nonsuch"));
    }

    #[test]
    fn clock_palette_color_and_conditional() {
        use crate::template::Color;
        use std::collections::HashMap;

        let mut theme = ThemeConfig::default();
        theme.segments.clock.format = Some("([$time](fg:custom_accent))".to_owned());

        // Without palette role -> error
        let err = validate_segment_templates(&theme, &Palette::default()).unwrap_err();
        assert_eq!(err.field_path, "segments.clock.format");
        assert!(err.to_string().contains("custom_accent"));

        // With palette role -> ok
        let mut map = HashMap::new();
        map.insert("custom_accent".to_owned(), Color::Named("cyan".to_owned()));
        let palette = Palette::new(map);
        assert!(validate_segment_templates(&theme, &palette).is_ok());
    }
}
