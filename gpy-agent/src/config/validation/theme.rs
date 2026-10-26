//! Theme schema validation.

use super::colors::validate_color;
use super::{ValidationError, ValidationErrorKind};
use crate::Result;
use crate::plugin::{SegmentName, first_token_collision};
use crate::theme::ThemeConfig;
use crate::theme::export::first_plugin_variable_collision;

/// Validate the fully parsed theme configuration.
///
/// # Errors
/// Returns a configuration error if any theme field fails validation.
pub fn validate_theme_config(theme: &ThemeConfig, source: &str) -> Result<()> {
    // Variant colours are typed keys; their values still go through the palette check.
    validate_plugin_segment_themes(&theme.segments.plugin, source)
}

/// Validate plugin-specific segment themes.
///
/// # Errors
/// Returns a configuration error if a section name is not a segment name, two
/// sections share a shell variable token, two fields would export the same
/// shell variable, or a variant colour is invalid.
fn validate_plugin_segment_themes(
    plugins: &std::collections::HashMap<String, crate::theme::PluginSegmentTheme>,
    source: &str,
) -> Result<()> {
    validate_plugin_segment_names(plugins.keys())?;
    if let Some(collision) = first_plugin_variable_collision(plugins) {
        return Err(ValidationError {
            field_path: "segments".to_owned(),
            kind: ValidationErrorKind::PluginVariableCollision { collision },
        }
        .into());
    }
    for (name, theme) in plugins {
        for (key, value) in &theme.variant_colors {
            validate_color(value, &format!("segments.{name}.{key}"), source)?;
        }
    }
    Ok(())
}

/// Every `[segments.<name>]` plugin section must name a segment, and no two may
/// export to the same shell variables (`Foo`/`foo`, `a-b`/`a_b`).
///
/// # Errors
/// Returns the first invalid or colliding section name, in sorted order.
#[allow(
    clippy::map_err_ignore,
    reason = "ValidationError has no source field; InvalidSegmentName already carries the rejected key"
)]
fn validate_plugin_segment_names<'a>(keys: impl Iterator<Item = &'a String>) -> Result<()> {
    let mut sorted: Vec<&String> = keys.collect();
    sorted.sort_unstable();
    let mut names: Vec<SegmentName> = Vec::with_capacity(sorted.len());
    for key in sorted {
        let name = SegmentName::new(key).map_err(|_| ValidationError {
            field_path: format!("segments.{key}"),
            kind: ValidationErrorKind::InvalidSegmentName { value: key.clone() },
        })?;
        if names.contains(&name) {
            return Err(ValidationError {
                field_path: format!("segments.{key}"),
                kind: ValidationErrorKind::InvalidValue {
                    expected: "segment section (duplicates another section's name ignoring case)",
                    value: key.clone(),
                },
            }
            .into());
        }
        names.push(name);
    }
    first_token_collision(&names).map_or(Ok(()), |collision| {
        Err(ValidationError {
            field_path: "segments".to_owned(),
            kind: ValidationErrorKind::SegmentTokenCollision { collision },
        }
        .into())
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::missing_panics_doc)]

    use super::validate_theme_config;
    use crate::theme::ThemeConfig;

    fn theme_with_sections(sections: &str) -> ThemeConfig {
        toml::from_str(&format!("[ui]\n[segments]\n{sections}")).unwrap()
    }

    #[test]
    fn plugin_sections_with_colliding_tokens_are_rejected() {
        let theme = theme_with_sections("[segments.k8s-ctx]\n[segments.k8s_ctx]\n");
        let err = validate_theme_config(&theme, "test")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("k8s-ctx") && err.contains("k8s_ctx"),
            "error must name both sections: {err}"
        );
    }

    /// Every `_`-joined name ambiguity from #1011: the error quotes both fields.
    #[test]
    fn plugin_sections_whose_variables_collide_are_rejected() {
        for (sections, first, second) in [
            (
                "[segments.a.open]\nicon = \"<\"\n[segments.a_open]\nicon = \"*\"\n",
                "\"segments.a.open.icon\"",
                "\"segments.a_open.icon\"",
            ),
            (
                "[segments.a_variant]\nb_bg_color = \"red\"\n[segments.a]\nvariant_b_bg_color = \"red\"\n",
                "\"segments.a.variant_b_bg_color\"",
                "\"segments.a_variant.b_bg_color\"",
            ),
            (
                "[segments.a]\nx_bg_color = \"red\"\n[segments.a_variant_x]\nbg_color = \"red\"\n",
                "\"segments.a.x_bg_color\"",
                "\"segments.a_variant_x.bg_color\"",
            ),
            (
                "[segments.a]\nb_c_text_color = \"red\"\n[segments.a_b]\nc_text_color = \"red\"\n",
                "\"segments.a.b_c_text_color\"",
                "\"segments.a_b.c_text_color\"",
            ),
        ] {
            let err = validate_theme_config(&theme_with_sections(sections), "test")
                .unwrap_err()
                .to_string();
            assert!(
                err.contains(first) && err.contains(second),
                "error must quote {first} and {second}: {err}"
            );
        }
    }

    #[test]
    fn plugin_sections_with_distinct_variables_are_accepted() {
        let theme = theme_with_sections(
            "[segments.a]\nbg_color = \"red\"\nok_bg_color = \"red\"\n[segments.a.open]\nicon = \"<\"\n\
             [segments.b_c]\nicon = \"*\"\nok_text_color = \"red\"\n[segments.aws-profile]\nbg_color = \"red\"\n",
        );
        assert!(validate_theme_config(&theme, "test").is_ok());
    }

    #[test]
    fn plugin_section_names_must_be_segment_names() {
        let theme = theme_with_sections("[segments.\"k8s ctx\"]\n");
        assert!(validate_theme_config(&theme, "test").is_err());
        let tolerated = theme_with_sections("[segments.my--seg]\n[segments.seg_]\n");
        assert!(validate_theme_config(&tolerated, "test").is_ok());
    }
}
