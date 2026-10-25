//! Assignments for plugin and community segments: styles, delimiters, variant colours and file paths.

use super::shell_assignment::{ShellAssignment, append_shell_assignment, append_shell_assignments};
use super::shell_colors::ShellColors;
use crate::config::DelimiterConfig;
use crate::plugin::{SegmentName, VariantColorPart};
use crate::shell::Shell;
use crate::theme::model::{PluginSegmentTheme, ThemeConfig};
use std::collections::BTreeMap;

/// Style assignments for one plugin segment, emitted both namespaced
/// (`__gpy_segment_<token>_*`) and flat (`__color_<token>_*`) so either contract works.
fn plugin_segment_style_assignments(
    colors: ShellColors<'_>,
    segment_name: &str,
    segment_token: &str,
    segment_theme: &PluginSegmentTheme,
) -> Vec<ShellAssignment> {
    let gpy_prefix = format!("__gpy_segment_{segment_token}");
    let mut assignments = vec![ShellAssignment::local(
        format!("{gpy_prefix}_id"),
        segment_name,
    )];

    if let Some(bg_color) = segment_theme.bg_color.as_deref() {
        let resolved_bg = colors.resolve(bg_color);
        assignments.push(ShellAssignment::local(
            format!("{gpy_prefix}_bg_color"),
            resolved_bg.as_ref(),
        ));
        assignments.push(ShellAssignment::local(
            format!("__color_{segment_token}_bg"),
            resolved_bg,
        ));
    }

    if let Some(text_color) = segment_theme.text_color.as_deref() {
        let resolved_text = colors.resolve(text_color);
        assignments.push(ShellAssignment::local(
            format!("{gpy_prefix}_text_color"),
            resolved_text.as_ref(),
        ));
        assignments.push(ShellAssignment::local(
            format!("__color_{segment_token}_fg"),
            resolved_text,
        ));
    }

    if let Some(icon) = segment_theme.icon.as_ref() {
        assignments.push(ShellAssignment::local(
            format!("{gpy_prefix}_icon"),
            icon.as_str(),
        ));
        assignments.push(ShellAssignment::local(
            format!("__icon_{segment_token}"),
            icon.as_str(),
        ));
    }

    assignments.extend(plugin_segment_delimiter_assignments(
        colors,
        &gpy_prefix,
        "open",
        segment_theme.open.as_ref(),
    ));
    assignments.extend(plugin_segment_delimiter_assignments(
        colors,
        &gpy_prefix,
        "close",
        segment_theme.close.as_ref(),
    ));

    assignments
}

/// Delimiter triple for one `side` (`"open"`/`"close"`) of a plugin segment, if set.
fn plugin_segment_delimiter_assignments(
    colors: ShellColors<'_>,
    gpy_prefix: &str,
    side: &str,
    delimiter: Option<&DelimiterConfig>,
) -> Vec<ShellAssignment> {
    let Some(config) = delimiter else {
        return Vec::new();
    };
    vec![
        ShellAssignment::local(format!("{gpy_prefix}_{side}_icon"), config.icon.as_str()),
        ShellAssignment::local(
            format!("{gpy_prefix}_{side}_icon_color"),
            colors.resolve(&config.icon_color),
        ),
        ShellAssignment::local(
            format!("{gpy_prefix}_{side}_bg_color"),
            colors.resolve(&config.bg_color),
        ),
    ]
}

/// Variant colours, sorted: `__gpy_segment_<token>_<variant>_{bg,text}_color`
/// plus the flat `__color_<token>_<variant>_{bg,fg}` alias built-ins use.
fn plugin_segment_variant_assignments(
    colors: ShellColors<'_>,
    segment_token: &str,
    segment_theme: &PluginSegmentTheme,
) -> Vec<ShellAssignment> {
    let mut assignments = Vec::new();
    for (key, color) in &segment_theme.variant_colors {
        let resolved = colors.resolve(color);
        let flat_suffix = match key.part {
            VariantColorPart::Bg => "bg",
            VariantColorPart::Text => "fg",
        };
        assignments.push(ShellAssignment::local(
            format!("__gpy_segment_{segment_token}_{key}"),
            resolved.as_ref(),
        ));
        assignments.push(ShellAssignment::local(
            format!("__color_{segment_token}_{}_{flat_suffix}", key.variant),
            resolved,
        ));
    }
    assignments
}

/// Plugin/community segment style exports, in sorted segment-name order.
pub(super) fn append_plugin_segment_assignments(
    output: &mut String,
    colors: ShellColors<'_>,
    theme: &ThemeConfig,
) {
    let mut plugin_segments: Vec<_> = theme.segments.plugin.iter().collect();
    plugin_segments.sort_unstable_by_key(|(left_name, _)| *left_name);

    for (raw_name, segment_theme) in plugin_segments {
        // Theme validation rejects keys that are not segment names.
        let Ok(segment_name) = SegmentName::new(raw_name) else {
            continue;
        };
        let segment_token = segment_name.variable_token();

        append_shell_assignments(
            output,
            colors.shell,
            plugin_segment_style_assignments(
                colors,
                segment_name.as_str(),
                &segment_token,
                segment_theme,
            ),
        );
        append_shell_assignments(
            output,
            colors.shell,
            plugin_segment_variant_assignments(colors, &segment_token, segment_theme),
        );
    }
}

/// Export plugin segment file paths (Fish only) so fish can source community segments.
pub(super) fn append_plugin_segment_file_assignments(
    output: &mut String,
    shell: Shell,
    plugin_segment_files: &BTreeMap<String, String>,
) {
    if !matches!(shell, Shell::Fish) {
        return;
    }
    for (segment_name, segment_file) in plugin_segment_files {
        let Ok(parsed_name) = SegmentName::new(segment_name) else {
            continue;
        };
        append_shell_assignment(
            output,
            shell,
            ShellAssignment::local(
                format!("__gpy_plugin_segment_file_{}", parsed_name.variable_token()),
                segment_file.as_str(),
            ),
        );
    }
}
