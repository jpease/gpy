//! Theme schema validation.

use super::colors::{validate_color, validate_icon};
use super::{ValidationError, ValidationErrorKind};
use crate::Result;
use crate::plugin::{SegmentName, first_token_collision};
use crate::theme::ThemeConfig;

/// Validate the fully parsed theme configuration.
///
/// # Errors
/// Returns a configuration error if any theme field fails validation.
pub fn validate_theme_config(theme: &ThemeConfig, source: &str) -> Result<()> {
    // Plugin properties are untyped maps, so validate by `*_color`/`*_icon` naming.
    validate_plugin_segment_themes(&theme.segments.plugin, source)
}

/// Validate plugin-specific segment themes.
///
/// # Errors
/// Returns a configuration error if a section name is not a segment name, two
/// sections share a shell variable token, or a plugin-specific color or icon is invalid.
fn validate_plugin_segment_themes(
    plugins: &std::collections::HashMap<String, crate::theme::PluginSegmentTheme>,
    source: &str,
) -> Result<()> {
    validate_plugin_segment_names(plugins.keys())?;
    for (name, theme) in plugins {
        let field_prefix = format!("segments.{name}");
        for (key, value) in &theme.properties {
            let field = format!("{field_prefix}.{key}");
            if key.ends_with("_color") {
                validate_color(value, &field, source)?;
            }
            if key.ends_with("_icon") {
                validate_icon(value, &field, source)?;
            }
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

    #[test]
    fn plugin_section_names_must_be_segment_names() {
        let theme = theme_with_sections("[segments.\"k8s ctx\"]\n");
        assert!(validate_theme_config(&theme, "test").is_err());
        let tolerated = theme_with_sections("[segments.my--seg]\n[segments.seg_]\n");
        assert!(validate_theme_config(&tolerated, "test").is_ok());
    }
}
