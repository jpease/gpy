//! Assignments for plugin and community segments: styles, delimiters, variant styles and file paths.
//!
//! [`plugin_segment_variables`] is the one place that names plugin segment
//! variables; export and theme validation ([`first_plugin_variable_collision`])
//! both read it, so validation checks exactly the names export would write.

use super::shell_assignment::{ShellAssignment, append_shell_assignment, append_shell_assignments};
use super::shell_colors::ShellColors;
use super::theme_to_shell::builtin_export_names;
use crate::debug_log;
use crate::formatter::separator::Glyphs;
use crate::plugin::{SegmentName, VariantColorPart};
use crate::shell::Shell;
use crate::theme::model::PluginSegmentTheme;
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::hash::BuildHasher;

/// A plugin segment variable's value: verbatim, a theme colour resolved per shell,
/// or an icon drawn with the export's glyph set.
#[derive(Clone, Copy)]
enum PluginValue<'a> {
    Literal(&'a str),
    Color(&'a str),
    Icon(&'a str),
}

/// One shell variable a plugin segment theme exports, and the theme field it comes from.
struct PluginVariable<'a> {
    name: String,
    /// Key under `[segments.<name>]`; empty for the section itself (`_id`).
    field: Cow<'static, str>,
    value: PluginValue<'a>,
}

impl PluginVariable<'_> {
    /// [`Glyphs::Plain`] exports a private-use icon empty, which the shells read as "no icon".
    fn into_assignment(self, colors: ShellColors<'_>, glyphs: Glyphs) -> ShellAssignment {
        let value = match self.value {
            PluginValue::Literal(value) => Cow::Borrowed(value),
            PluginValue::Color(color) => colors.resolve(color),
            PluginValue::Icon(icon) => Cow::Borrowed(glyphs.icon(icon).unwrap_or("")),
        };
        ShellAssignment::local(self.name, value)
    }
}

/// Every variable one plugin segment section exports, in export order: the
/// namespaced `__gpy_segment_<token>_*` contract plus the flat `__color_<token>_*`
/// and `__icon_<token>` aliases built-ins use.
///
/// Variant styles sort by key: colours as `__gpy_segment_<token>_variant_<variant>_{bg,text}_color`
/// and `__color_<token>_<variant>_{bg,fg}`, then icons as
/// `__gpy_segment_<token>_variant_<variant>_icon` and `__icon_<token>_<variant>`. The
/// `variant_` namespace keeps variant `open`/`close` off the delimiter variables. Names
/// are distinct within one section; across sections `_` joins are ambiguous (segment
/// `a` variant `x` vs segment `a_variant_x`), which [`Claims`] rejects.
fn plugin_segment_variables<'a>(
    segment_name: &'a str,
    segment_token: &str,
    theme: &'a PluginSegmentTheme,
) -> Vec<PluginVariable<'a>> {
    let prefix = format!("__gpy_segment_{segment_token}");
    let mut variables = Vec::new();
    let mut push = |name: String, field: Cow<'static, str>, value: PluginValue<'a>| {
        variables.push(PluginVariable { name, field, value });
    };

    push(
        format!("{prefix}_id"),
        Cow::Borrowed(""),
        PluginValue::Literal(segment_name),
    );
    if let Some(bg_color) = theme.bg_color.as_deref() {
        let value = PluginValue::Color(bg_color);
        push(
            format!("{prefix}_bg_color"),
            Cow::Borrowed("bg_color"),
            value,
        );
        push(
            format!("__color_{segment_token}_bg"),
            Cow::Borrowed("bg_color"),
            value,
        );
    }
    if let Some(text_color) = theme.text_color.as_deref() {
        let value = PluginValue::Color(text_color);
        push(
            format!("{prefix}_text_color"),
            Cow::Borrowed("text_color"),
            value,
        );
        push(
            format!("__color_{segment_token}_fg"),
            Cow::Borrowed("text_color"),
            value,
        );
    }
    if let Some(icon) = theme.icon.as_deref() {
        let value = PluginValue::Icon(icon);
        push(format!("{prefix}_icon"), Cow::Borrowed("icon"), value);
        push(
            format!("__icon_{segment_token}"),
            Cow::Borrowed("icon"),
            value,
        );
    }
    for (side, configured, fields) in [
        (
            "open",
            theme.open.as_ref(),
            ["open.icon", "open.icon_color", "open.bg_color"],
        ),
        (
            "close",
            theme.close.as_ref(),
            ["close.icon", "close.icon_color", "close.bg_color"],
        ),
    ] {
        let Some(delimiter) = configured else {
            continue;
        };
        let [icon_field, icon_color_field, bg_color_field] = fields.map(Cow::Borrowed);
        push(
            format!("{prefix}_{side}_icon"),
            icon_field,
            PluginValue::Icon(&delimiter.icon),
        );
        push(
            format!("{prefix}_{side}_icon_color"),
            icon_color_field,
            PluginValue::Color(&delimiter.icon_color),
        );
        push(
            format!("{prefix}_{side}_bg_color"),
            bg_color_field,
            PluginValue::Color(&delimiter.bg_color),
        );
    }
    push_variant_variables(&mut variables, &prefix, segment_token, theme);
    variables
}

/// Variant colour then variant icon variables, each sorted by key.
fn push_variant_variables<'a>(
    variables: &mut Vec<PluginVariable<'a>>,
    prefix: &str,
    segment_token: &str,
    theme: &'a PluginSegmentTheme,
) {
    for (key, color) in &theme.variant_colors {
        let flat_suffix = match key.part {
            VariantColorPart::Bg => "bg",
            VariantColorPart::Text => "fg",
        };
        let field: Cow<'static, str> = Cow::Owned(key.to_string());
        variables.push(PluginVariable {
            name: format!("{prefix}_variant_{key}"),
            field: field.clone(),
            value: PluginValue::Color(color),
        });
        variables.push(PluginVariable {
            name: format!("__color_{segment_token}_{}_{flat_suffix}", key.variant),
            field,
            value: PluginValue::Color(color),
        });
    }
    for (variant, icon) in &theme.variant_icons {
        let field: Cow<'static, str> = Cow::Owned(format!("{variant}_icon"));
        variables.push(PluginVariable {
            name: format!("{prefix}_variant_{variant}_icon"),
            field: field.clone(),
            value: PluginValue::Icon(icon),
        });
        variables.push(PluginVariable {
            name: format!("__icon_{segment_token}_{variant}"),
            field,
            value: PluginValue::Icon(icon),
        });
    }
}

/// Two theme fields that would export the same shell variable, or a plugin
/// field that would overwrite a built-in theme export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginVariableCollision {
    /// The shared shell variable name.
    pub variable: String,
    /// The theme field that claimed `variable` first, e.g. `segments.a.open.icon`;
    /// `None` when `variable` is a built-in theme export.
    pub first: Option<String>,
    /// The later (plugin) theme field, e.g. `segments.a_open.icon`.
    pub second: String,
}

impl fmt::Display for PluginVariableCollision {
    #[allow(
        clippy::use_debug,
        reason = "theme keys and variable names are quoted `{:?}`-escaped in this error text"
    )]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.first {
            Some(first) => write!(
                f,
                "{first:?} and {:?} both export shell variable {:?}; rename one of the segments or variants",
                self.second, self.variable
            ),
            None => write!(
                f,
                "{:?} exports shell variable {:?}, which is a built-in theme export; rename the segment or variant",
                self.second, self.variable
            ),
        }
    }
}

/// `segments.<section>[.<field>]`.
fn field_path(section: &str, field: &str) -> String {
    if field.is_empty() {
        format!("segments.{section}")
    } else {
        format!("segments.{section}.{field}")
    }
}

/// Exported variable names claimed so far, with the section and field that claimed each.
#[derive(Default)]
struct Claims<'a> {
    owners: HashMap<String, (&'a str, Cow<'static, str>)>,
}

impl<'a> Claims<'a> {
    /// Claim every name in `variables` for `section`, or claim none and return the
    /// first name an earlier section already holds.
    fn claim(
        &mut self,
        section: &'a str,
        variables: &[PluginVariable<'_>],
    ) -> Result<(), PluginVariableCollision> {
        if let Some(variable) = variables
            .iter()
            .find(|variable| builtin_export_names().contains(&variable.name))
        {
            return Err(PluginVariableCollision {
                variable: variable.name.clone(),
                first: None,
                second: field_path(section, &variable.field),
            });
        }
        if let Some((variable, (first_section, first_field))) = variables
            .iter()
            .find_map(|variable| Some((variable, self.owners.get(&variable.name)?)))
        {
            return Err(PluginVariableCollision {
                variable: variable.name.clone(),
                first: Some(field_path(first_section, first_field)),
                second: field_path(section, &variable.field),
            });
        }
        for variable in variables {
            self.owners
                .insert(variable.name.clone(), (section, variable.field.clone()));
        }
        Ok(())
    }
}

/// Plugin segment sections in sorted key order, with their parsed names. Theme
/// validation rejects keys that are not segment names; export skips them.
fn sorted_sections<S>(
    plugins: &HashMap<String, PluginSegmentTheme, S>,
) -> Vec<(&str, SegmentName, &PluginSegmentTheme)> {
    let mut sections: Vec<_> = plugins
        .iter()
        .filter_map(|(raw_name, theme)| {
            SegmentName::new(raw_name)
                .ok()
                .map(|name| (raw_name.as_str(), name, theme))
        })
        .collect();
    sections.sort_unstable_by_key(|(raw_name, ..)| *raw_name);
    sections
}

/// The first pair of plugin segment theme fields, in sorted section order, that
/// would export the same shell variable.
#[must_use]
pub fn first_plugin_variable_collision<S: BuildHasher>(
    plugins: &HashMap<String, PluginSegmentTheme, S>,
) -> Option<PluginVariableCollision> {
    let mut claims = Claims::default();
    sorted_sections(plugins)
        .into_iter()
        .find_map(|(raw_name, name, theme)| {
            let variables = plugin_segment_variables(name.as_str(), &name.variable_token(), theme);
            claims.claim(raw_name, &variables).err()
        })
}

/// Plugin/community segment style exports, in sorted segment-name order. A
/// section whose variables collide with an earlier section's is skipped whole, so
/// every name is assigned once; theme validation already rejects such themes.
pub(super) fn append_plugin_segment_assignments(
    output: &mut String,
    colors: ShellColors<'_>,
    glyphs: Glyphs,
    plugins: &HashMap<String, PluginSegmentTheme>,
) {
    let mut claims = Claims::default();
    for (raw_name, name, theme) in sorted_sections(plugins) {
        let variables = plugin_segment_variables(name.as_str(), &name.variable_token(), theme);
        if let Err(collision) = claims.claim(raw_name, &variables) {
            debug_log!("theme", "Skipped plugin segment {raw_name:?}: {collision}");
            continue;
        }
        append_shell_assignments(
            output,
            colors.shell,
            variables
                .into_iter()
                .map(|variable| variable.into_assignment(colors, glyphs)),
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
