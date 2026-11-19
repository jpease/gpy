//! [`prompt_shell_values`]: a theme and config as the named values `gpy prompt` reads.

use super::plugin_segments::insert_plugin_segment_values;
use super::segments::{
    insert_builtin_segment_values, insert_delimiter_values, insert_git_language_color_values,
    insert_global_values, insert_prompt_values, insert_status_values,
};
use super::shell_colors::ShellColors;
use crate::config::Config;
use crate::formatter::separator::Glyphs;
use crate::shell::Shell;
use crate::template::Palette;
use crate::theme::model::ThemeConfig;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

/// Values by name while they are collected. `None` marks a name the theme
/// leaves unset (an erased icon): it is reserved but has no value.
pub(super) type Assignments = HashMap<String, Option<String>>;

/// Assign `value` to `name`.
pub(super) fn set(
    assignments: &mut Assignments,
    name: impl Into<String>,
    value: impl Into<String>,
) {
    assignments.insert(name.into(), Some(value.into()));
}

/// The built-in values: global settings, prompt, built-in segments, status,
/// git/language colours and delimiters.
fn insert_builtin_values(
    assignments: &mut Assignments,
    theme: &ThemeConfig,
    config: &Config,
    colors: ShellColors<'_>,
    glyphs: Glyphs,
) {
    insert_global_values(assignments, theme, config);
    insert_prompt_values(assignments, colors, theme);
    insert_builtin_segment_values(assignments, colors, theme);
    insert_status_values(assignments, colors, theme, glyphs);
    insert_git_language_color_values(assignments, colors, theme);
    insert_delimiter_values(assignments, colors, theme, glyphs);
}

/// Every name the built-in values use, derived by running
/// [`insert_builtin_values`] itself so the set cannot drift from it. Each
/// built-in name is unconditional (values vary, names do not), so a default
/// theme and config name them all. Plugin segment themes must not use any of
/// these.
pub(crate) fn builtin_value_names() -> &'static HashSet<String> {
    static NAMES: LazyLock<HashSet<String>> = LazyLock::new(|| {
        let palette = Palette::default();
        let colors = ShellColors {
            shell: Shell::Bash,
            palette: &palette,
        };
        let mut assignments = Assignments::new();
        insert_builtin_values(
            &mut assignments,
            &ThemeConfig::default(),
            &Config::default(),
            colors,
            Glyphs::Nerd,
        );
        assignments.into_keys().collect()
    });
    &NAMES
}

/// The values the prompt renderer reads for `theme` under `config`, colours
/// resolved through `palette`: the built-in values, then every plugin segment
/// style. A name the theme leaves unset has no entry.
#[must_use]
pub fn prompt_shell_values(
    theme: &ThemeConfig,
    config: &Config,
    palette: &Palette,
) -> HashMap<String, String> {
    let colors = ShellColors {
        shell: Shell::Bash,
        palette,
    };
    let glyphs = Glyphs::from(&config.ui);
    let mut assignments = Assignments::new();
    insert_builtin_values(&mut assignments, theme, config, colors, glyphs);
    insert_plugin_segment_values(&mut assignments, colors, glyphs, &theme.segments.plugin);
    assignments
        .into_iter()
        .filter_map(|(name, value)| Some((name, value?)))
        .collect()
}
