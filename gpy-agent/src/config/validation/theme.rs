//! Theme schema validation.
//!
//! This module validates user and plugin theme files after TOML parsing but
//! before they are accepted by [`crate::theme::ThemeManager`]. It keeps
//! color/icon validation, segment style checks, and user-facing diagnostics
//! together so invalid theme files fail with actionable messages.

use super::colors::{validate_color, validate_icon};
use crate::Result;
use crate::theme::ThemeConfig;

/// Validate the fully parsed theme configuration.
///
/// # Errors
///
/// Returns a configuration error if any theme field fails validation.
pub fn validate_theme_config(theme: &ThemeConfig, source: &str) -> Result<()> {
    // Types like ColorSpec and Icon handle validation during deserialization for known fields.
    // However, plugin properties are untyped HashMaps and still need manual validation
    // based on naming conventions (*_color, *_icon).
    validate_plugin_segment_themes(&theme.segments.plugin, source)
}

/// Validate plugin-specific segment themes.
///
/// # Errors
///
/// Returns a configuration error if any plugin-specific color or icon is invalid.
fn validate_plugin_segment_themes(
    plugins: &std::collections::HashMap<String, crate::theme::PluginSegmentTheme>,
    source: &str,
) -> Result<()> {
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
