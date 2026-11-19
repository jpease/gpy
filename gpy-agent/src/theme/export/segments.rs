//! Values for the global settings, the prompt, the built-in segments and the delimiters.

use super::shell_colors::ShellColors;
use super::values::{Assignments, set};
use crate::config::{Config, DEFAULT_UI_ROOT_PROMPT_COLOR, DelimiterConfig};
use crate::formatter::separator::Glyphs;
use crate::template::sanitize_control_chars;
use crate::theme::model::{GitState, ThemeConfig};

/// Render a boolean setting as the `1`/`0` the renderer expects.
const fn flag(value: bool) -> &'static str {
    if value { "1" } else { "0" }
}

/// The icon style and the powerline glyph pair.
pub(super) fn insert_global_values(
    assignments: &mut Assignments,
    theme: &ThemeConfig,
    config: &Config,
) {
    let prompt_icon_style = if config.ui.show_icons {
        "nerd"
    } else {
        "ascii"
    };
    let glyphs = Glyphs::from(&config.ui);
    let powerline_start = glyphs.delimiter(
        theme
            .ui
            .segment_open
            .as_ref()
            .map_or("", |cfg| cfg.icon.as_str()),
    );
    let powerline_end = glyphs.delimiter(
        theme
            .ui
            .segment_close
            .as_ref()
            .map_or("", |cfg| cfg.icon.as_str()),
    );

    set(assignments, "__prompt_icons", prompt_icon_style);
    set(
        assignments,
        "__icon_powerline_segment_start",
        powerline_start,
    );
    set(assignments, "__icon_powerline_segment_end", powerline_end);
}

/// Prompt and root-prompt icons/colors plus the layout flags.
/// `__icon_prompt`/`__prompt_color` follow `segments.character`'s
/// `success_symbol`/`success_color`; the symbol is stripped of control
/// characters (`❯` if nothing is left) because it is printed raw.
/// `__root_prompt_color` falls back to [`DEFAULT_UI_ROOT_PROMPT_COLOR`].
pub(super) fn insert_prompt_values(
    assignments: &mut Assignments,
    colors: ShellColors<'_>,
    theme: &ThemeConfig,
) {
    let character = &theme.segments.character;
    let clean = sanitize_control_chars(&character.success_symbol);
    let symbol = if clean.is_empty() {
        "❯".to_owned()
    } else {
        clean.into_owned()
    };
    set(assignments, "__icon_prompt", symbol);
    set(
        assignments,
        "__prompt_color",
        colors.resolve(&character.success_color),
    );

    set(
        assignments,
        "__icon_root_prompt",
        theme.ui.root_prompt_icon.as_str(),
    );
    let root_color = theme
        .ui
        .root_prompt_color
        .as_deref()
        .unwrap_or(DEFAULT_UI_ROOT_PROMPT_COLOR);
    set(
        assignments,
        "__root_prompt_color",
        colors.resolve(root_color),
    );

    set(assignments, "__gpy_two_line", flag(theme.ui.two_line));
    set(assignments, "__gpy_add_newline", flag(theme.ui.add_newline));
}

/// Built-in clock, directory, and duration segment settings.
///
/// `__clock_format` is a presence flag only: the agent re-derives the template.
/// Threshold and bg colors feed the visibility gate and chevrons.
pub(super) fn insert_builtin_segment_values(
    assignments: &mut Assignments,
    colors: ShellColors<'_>,
    theme: &ThemeConfig,
) {
    let segments = &theme.segments;
    let clock = &segments.clock;
    set(
        assignments,
        "__color_clock_bg",
        colors.resolve(&clock.bg_color),
    );
    set(
        assignments,
        "__color_clock_fg",
        colors.resolve(&clock.text_color),
    );
    set(assignments, "__time_format", clock.time_format());
    set(
        assignments,
        "__clock_show_leading_zero",
        flag(clock.show_leading_zero()),
    );
    set(
        assignments,
        "__clock_show_seconds",
        flag(clock.show_seconds()),
    );
    set(
        assignments,
        "__clock_format",
        if clock.format.is_some() { "1" } else { "" },
    );
    set(
        assignments,
        "__duration_threshold_ms",
        segments.duration.show_if_exceeds_ms.to_string(),
    );
    set(
        assignments,
        "__color_directory_bg",
        colors.resolve(&segments.directory.bg_color),
    );
    set(
        assignments,
        "__color_duration_bg",
        colors.resolve(&segments.duration.bg_color),
    );
}

/// Status segment icons, the ok/fail colors and the template presence flag. An
/// icon is unset when not configured, or when [`Glyphs::Plain`] drops it, so
/// the renderer draws its ASCII fallback.
pub(super) fn insert_status_values(
    assignments: &mut Assignments,
    colors: ShellColors<'_>,
    theme: &ThemeConfig,
    glyphs: Glyphs,
) {
    let status = &theme.segments.status;

    for (name, icon) in [
        ("__icon_status_ok", status.ok_icon.as_ref()),
        ("__icon_status_fail", status.fail_icon.as_ref()),
    ] {
        let text = icon.and_then(|configured| glyphs.icon(configured.as_str()));
        assignments.insert(name.to_owned(), text.map(str::to_owned));
    }
    for (name, color) in [
        ("__color_status_ok_bg", &status.ok_bg_color),
        ("__color_status_ok_fg", &status.ok_text_color),
        ("__color_status_fail_bg", &status.fail_bg_color),
        ("__color_status_fail_fg", &status.fail_text_color),
    ] {
        set(assignments, name, colors.resolve(color));
    }
    // Presence flag only: the agent re-derives the template.
    set(
        assignments,
        "__status_format",
        if status.format.is_some() { "1" } else { "" },
    );
}

/// Git/language bg colors only (they render from agent ANSI), for powerline chevrons.
pub(super) fn insert_git_language_color_values(
    assignments: &mut Assignments,
    colors: ShellColors<'_>,
    theme: &ThemeConfig,
) {
    let git = &theme.segments.git;
    set(
        assignments,
        "__color_git_clean_bg",
        colors.resolve(git.get_bg_color(GitState::Clean)),
    );
    set(
        assignments,
        "__color_git_dirty_bg",
        colors.resolve(git.get_bg_color(GitState::Modified)),
    );
    set(
        assignments,
        "__color_language_bg",
        colors.resolve(&theme.segments.language.bg_color),
    );
}

/// Icon-color/bg pair for an optional UI delimiter, defaulting to `white`/`transparent`.
fn delimiter_colors(delimiter: Option<&DelimiterConfig>) -> (&str, &str) {
    delimiter.map_or(("white", "transparent"), |cfg| {
        (cfg.icon_color.as_str(), cfg.bg_color.as_str())
    })
}

/// Delimiter glyphs and colors. With [`Glyphs::Plain`], Nerd Font glyphs are empty.
#[expect(clippy::similar_names, reason = "deliberate fg/bg pairs")]
pub(super) fn insert_delimiter_values(
    assignments: &mut Assignments,
    colors: ShellColors<'_>,
    theme: &ThemeConfig,
    glyphs: Glyphs,
) {
    let ui = &theme.ui;
    for (name, delimiter) in [
        ("__segment_delim_first", ui.get_prompt_open_delimiter()),
        ("__segment_delim_start", ui.get_segment_open_delimiter()),
        ("__segment_delim_end", ui.get_segment_close_delimiter()),
        ("__segment_delim_last", ui.get_prompt_close_delimiter()),
    ] {
        set(assignments, name, glyphs.delimiter(delimiter));
    }

    let (prompt_open_fg, prompt_open_bg) = delimiter_colors(ui.prompt_open.as_ref());
    let (prompt_close_fg, prompt_close_bg) = delimiter_colors(ui.prompt_close.as_ref());
    let (seg_delim_fg, seg_delim_bg) = delimiter_colors(ui.segment_open.as_ref());
    for (name, color) in [
        ("__prompt_open_color", prompt_open_fg),
        ("__prompt_open_bg", prompt_open_bg),
        ("__prompt_close_color", prompt_close_fg),
        ("__prompt_close_bg", prompt_close_bg),
        ("__segment_delimiter_color", seg_delim_fg),
        ("__segment_delimiter_bg", seg_delim_bg),
    ] {
        set(assignments, name, colors.resolve(color));
    }
}
