//! Assignments for the global settings, the prompt, the built-in segments and the delimiters.

use super::shell_assignment::{ShellAssignment, append_shell_assignment, append_shell_assignments};
use super::shell_colors::ShellColors;
use crate::config::{
    Config, DEFAULT_UI_PROMPT_COLOR, DEFAULT_UI_ROOT_PROMPT_COLOR, DelimiterConfig,
};
use crate::formatter::separator::Glyphs;
use crate::shell::Shell;
use crate::theme::model::{GitState, ThemeConfig};

/// Render a boolean setting as the `1`/`0` the shell integration expects.
const fn flag(value: bool) -> &'static str {
    if value { "1" } else { "0" }
}

/// `GPY_*` env exports mirroring `config`; all values are flags/validated enums.
fn config_export_assignments(config: &Config) -> Vec<ShellAssignment> {
    let agent = &config.agent;
    let supervisor = &agent.supervisor;
    vec![
        ShellAssignment::export("GPY_AGENT_ENABLED", flag(agent.enabled)),
        ShellAssignment::export("GPY_AGENT_LIVE_UPDATES", flag(agent.live_updates)),
        ShellAssignment::export("GPY_AGENT_SUPERVISOR_ENABLED", flag(supervisor.enabled)),
        ShellAssignment::export(
            "GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS",
            supervisor.check_interval_seconds.to_string(),
        ),
        ShellAssignment::export(
            "GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS",
            supervisor.max_restart_attempts.to_string(),
        ),
        ShellAssignment::export(
            "GPY_AGENT_TIMEOUT_SECONDS",
            agent.timeout_seconds.to_string(),
        ),
        ShellAssignment::export("GPY_LANGUAGE_ENABLED", flag(config.language.enabled)),
        ShellAssignment::export(
            "GPY_LANGUAGE_SHOW_VERSIONS",
            flag(config.language.show_versions),
        ),
        ShellAssignment::export("GPY_GIT_ENABLED", flag(config.git.enabled)),
        ShellAssignment::export("GPY_GIT_SHOW_UPSTREAM", flag(config.git.show_upstream)),
        ShellAssignment::export(
            "GPY_GIT_MAX_BRANCH_LENGTH",
            config.git.max_branch_length.to_string(),
        ),
        ShellAssignment::export(
            "GPY_UI_DIRECTORY_MAX_LENGTH",
            config.ui.directory.max_length.to_string(),
        ),
        ShellAssignment::export(
            "GPY_UI_DIRECTORY_DISPLAY",
            config.ui.directory.display.to_string(),
        ),
        ShellAssignment::export("GPY_UI_SHOW_ICONS", flag(config.ui.show_icons)),
        ShellAssignment::export("GPY_UI_TRANSIENT_PROMPT", flag(config.ui.transient_prompt)),
    ]
}

/// Agent/git/language/UI settings, the theme name, and the powerline glyph pair.
pub(super) fn append_global_assignments(
    output: &mut String,
    shell: Shell,
    theme: &ThemeConfig,
    theme_name: &str,
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

    let mut assignments = vec![ShellAssignment::local("__gpy_theme_name", theme_name)];
    assignments.extend(config_export_assignments(config));
    assignments.extend([
        ShellAssignment::local("__prompt_icons", prompt_icon_style),
        ShellAssignment::local("__icon_powerline_segment_start", powerline_start),
        ShellAssignment::local("__icon_powerline_segment_end", powerline_end),
    ]);
    append_shell_assignments(output, shell, assignments);
}

/// Prompt and root-prompt icons/colors plus the two-line layout flag.
/// `__prompt_color`/`__root_prompt_color` fall back to
/// [`DEFAULT_UI_PROMPT_COLOR`]/[`DEFAULT_UI_ROOT_PROMPT_COLOR`].
pub(super) fn append_prompt_assignments(
    output: &mut String,
    colors: ShellColors<'_>,
    theme: &ThemeConfig,
) {
    let shell = colors.shell;

    append_shell_assignment(
        output,
        shell,
        ShellAssignment::local("__icon_prompt", theme.ui.prompt_icon.as_str()),
    );

    let prompt_color = theme
        .ui
        .prompt_color
        .as_deref()
        .unwrap_or(DEFAULT_UI_PROMPT_COLOR);
    append_shell_assignment(
        output,
        shell,
        ShellAssignment::local("__prompt_color", colors.resolve(prompt_color)),
    );

    append_shell_assignment(
        output,
        shell,
        ShellAssignment::local("__icon_root_prompt", theme.ui.root_prompt_icon.as_str()),
    );
    let root_color = theme
        .ui
        .root_prompt_color
        .as_deref()
        .unwrap_or(DEFAULT_UI_ROOT_PROMPT_COLOR);
    append_shell_assignment(
        output,
        shell,
        ShellAssignment::local("__root_prompt_color", colors.resolve(root_color)),
    );

    // Always emit 0/1 so disabling in-session re-exports 0.
    append_shell_assignment(
        output,
        shell,
        ShellAssignment::local("__gpy_two_line", flag(theme.ui.two_line)),
    );

    append_shell_assignment(
        output,
        shell,
        ShellAssignment::local("__gpy_add_newline", flag(theme.ui.add_newline)),
    );
}

/// Built-in clock, directory, and duration segment settings.
///
/// Directory/duration render from agent ANSI. `__clock_format` is a presence flag only
/// (see hostname). Threshold and bg colors feed the shell's visibility gate and chevrons.
pub(super) fn append_builtin_segment_assignments(
    output: &mut String,
    colors: ShellColors<'_>,
    theme: &ThemeConfig,
) {
    let shell = colors.shell;
    append_shell_assignments(
        output,
        shell,
        [
            ShellAssignment::local(
                "__color_clock_bg",
                colors.resolve(&theme.segments.clock.bg_color),
            ),
            ShellAssignment::local(
                "__color_clock_fg",
                colors.resolve(&theme.segments.clock.text_color),
            ),
        ],
    );
    append_shell_assignment(
        output,
        shell,
        ShellAssignment::local("__time_format", theme.segments.clock.time_format()),
    );
    append_shell_assignments(
        output,
        shell,
        [
            ShellAssignment::local(
                "__clock_show_leading_zero",
                flag(theme.segments.clock.show_leading_zero()),
            ),
            ShellAssignment::local(
                "__clock_show_seconds",
                flag(theme.segments.clock.show_seconds()),
            ),
            ShellAssignment::local(
                "__clock_format",
                if theme.segments.clock.format.is_some() {
                    "1"
                } else {
                    ""
                },
            ),
            ShellAssignment::local(
                "__duration_threshold_ms",
                theme.segments.duration.show_if_exceeds_ms.to_string(),
            ),
            ShellAssignment::local(
                "__color_directory_bg",
                colors.resolve(&theme.segments.directory.bg_color),
            ),
            ShellAssignment::local(
                "__color_duration_bg",
                colors.resolve(&theme.segments.duration.bg_color),
            ),
        ],
    );
}

/// Hostname segment colors, trimming, and template-path flags.
/// `format`/`icon` are always assigned (empty when `None`) so the shell can `test -n`;
/// [`Glyphs::Plain`] empties a private-use icon.
pub(super) fn append_hostname_assignments(
    output: &mut String,
    colors: ShellColors<'_>,
    theme: &ThemeConfig,
    glyphs: Glyphs,
) {
    let shell = colors.shell;

    append_shell_assignments(
        output,
        shell,
        [
            ShellAssignment::local(
                "__color_hostname_bg",
                colors.resolve(&theme.segments.hostname.bg_color),
            ),
            ShellAssignment::local(
                "__color_hostname_fg",
                colors.resolve(&theme.segments.hostname.text_color),
            ),
            ShellAssignment::local(
                "__hostname_trim_at",
                theme.segments.hostname.trim_at.as_str(),
            ),
            ShellAssignment::local(
                "__hostname_show_always",
                flag(theme.segments.hostname.show_always),
            ),
            // PRESENCE FLAG ONLY: the agent re-derives the template server-side.
            // Exporting the raw format would allow shell injection and break `set -u`.
            ShellAssignment::local(
                "__hostname_format",
                if theme.segments.hostname.format.is_some() {
                    "1"
                } else {
                    ""
                },
            ),
            ShellAssignment::local(
                "__icon_hostname",
                theme
                    .segments
                    .hostname
                    .icon
                    .as_ref()
                    .and_then(|icon| glyphs.icon(icon.as_str()))
                    .unwrap_or(""),
            ),
        ],
    );
}

/// Username segment colors (root, then user) and template-path flags.
/// `__username_format` is a presence flag only, same rationale as `__hostname_format`,
/// set when either the root or the user format exists; the icon is handled as the hostname's.
pub(super) fn append_username_assignments(
    output: &mut String,
    colors: ShellColors<'_>,
    theme: &ThemeConfig,
    glyphs: Glyphs,
) {
    let shell = colors.shell;
    let username = &theme.segments.username;

    append_shell_assignments(
        output,
        shell,
        [
            ShellAssignment::local("__color_username_bg", colors.resolve(&username.bg_color)),
            ShellAssignment::local("__color_username_fg", colors.resolve(&username.text_color)),
            ShellAssignment::local(
                "__color_username_user_bg",
                colors.resolve(&username.user_bg_color),
            ),
            ShellAssignment::local(
                "__color_username_user_fg",
                colors.resolve(&username.user_text_color),
            ),
            ShellAssignment::local("__username_show_always", flag(username.show_always)),
            ShellAssignment::local(
                "__username_format",
                if username.format.is_some() || username.user_format.is_some() {
                    "1"
                } else {
                    ""
                },
            ),
            ShellAssignment::local(
                "__icon_username",
                username
                    .icon
                    .as_ref()
                    .and_then(|icon| glyphs.icon(icon.as_str()))
                    .unwrap_or(""),
            ),
        ],
    );
}

/// Status segment icons, the ok/fail colors and the template presence flag. An
/// icon is erased when unset, or when [`Glyphs::Plain`] drops it, so the shell
/// draws its ASCII fallback.
pub(super) fn append_status_assignments(
    output: &mut String,
    colors: ShellColors<'_>,
    theme: &ThemeConfig,
    glyphs: Glyphs,
) {
    let shell = colors.shell;
    let status = &theme.segments.status;

    for (name, icon) in [
        ("__icon_status_ok", status.ok_icon.as_ref()),
        ("__icon_status_fail", status.fail_icon.as_ref()),
    ] {
        append_shell_assignment(
            output,
            shell,
            icon.and_then(|configured| glyphs.icon(configured.as_str()))
                .map_or_else(
                    || ShellAssignment::unset(name),
                    |text| ShellAssignment::local(name, text),
                ),
        );
    }
    append_shell_assignments(
        output,
        shell,
        [
            ShellAssignment::local(
                "__color_status_ok_bg",
                colors.resolve(&theme.segments.status.ok_bg_color),
            ),
            ShellAssignment::local(
                "__color_status_ok_fg",
                colors.resolve(&theme.segments.status.ok_text_color),
            ),
            ShellAssignment::local(
                "__color_status_fail_bg",
                colors.resolve(&theme.segments.status.fail_bg_color),
            ),
            ShellAssignment::local(
                "__color_status_fail_fg",
                colors.resolve(&theme.segments.status.fail_text_color),
            ),
            // PRESENCE FLAG ONLY, like `__hostname_format`: the agent re-derives the template.
            ShellAssignment::local(
                "__status_format",
                if theme.segments.status.format.is_some() {
                    "1"
                } else {
                    ""
                },
            ),
        ],
    );
}

/// Git/language bg colors only (they render from agent ANSI), for powerline chevrons.
pub(super) fn append_git_language_color_assignments(
    output: &mut String,
    colors: ShellColors<'_>,
    theme: &ThemeConfig,
) {
    let git = &theme.segments.git;

    append_shell_assignments(
        output,
        colors.shell,
        [
            ShellAssignment::local(
                "__color_git_clean_bg",
                colors.resolve(git.get_bg_color(GitState::Clean)),
            ),
            ShellAssignment::local(
                "__color_git_dirty_bg",
                colors.resolve(git.get_bg_color(GitState::Modified)),
            ),
            ShellAssignment::local(
                "__color_language_bg",
                colors.resolve(&theme.segments.language.bg_color),
            ),
        ],
    );
}

/// Icon-color/bg pair for an optional UI delimiter, defaulting to `white`/`transparent`.
fn delimiter_colors(delimiter: Option<&DelimiterConfig>) -> (&str, &str) {
    delimiter.map_or(("white", "transparent"), |cfg| {
        (cfg.icon_color.as_str(), cfg.bg_color.as_str())
    })
}

/// Delimiter glyphs and colors. With [`Glyphs::Plain`], Nerd Font glyphs export empty.
#[expect(clippy::similar_names, reason = "deliberate fg/bg pairs")]
pub(super) fn append_delimiter_assignments(
    output: &mut String,
    colors: ShellColors<'_>,
    theme: &ThemeConfig,
    glyphs: Glyphs,
) {
    let shell = colors.shell;

    let (prompt_open_fg, prompt_open_bg) = delimiter_colors(theme.ui.prompt_open.as_ref());
    let (prompt_close_fg, prompt_close_bg) = delimiter_colors(theme.ui.prompt_close.as_ref());
    let (seg_delim_fg, seg_delim_bg) = delimiter_colors(theme.ui.segment_open.as_ref());
    append_shell_assignments(
        output,
        shell,
        [
            ShellAssignment::local(
                "__segment_delim_first",
                glyphs.delimiter(theme.ui.get_prompt_open_delimiter()),
            ),
            ShellAssignment::local(
                "__segment_delim_start",
                glyphs.delimiter(theme.ui.get_segment_open_delimiter()),
            ),
            ShellAssignment::local(
                "__segment_delim_end",
                glyphs.delimiter(theme.ui.get_segment_close_delimiter()),
            ),
            ShellAssignment::local(
                "__segment_delim_last",
                glyphs.delimiter(theme.ui.get_prompt_close_delimiter()),
            ),
            ShellAssignment::local("__prompt_open_color", colors.resolve(prompt_open_fg)),
            ShellAssignment::local("__prompt_open_bg", colors.resolve(prompt_open_bg)),
            ShellAssignment::local("__prompt_close_color", colors.resolve(prompt_close_fg)),
            ShellAssignment::local("__prompt_close_bg", colors.resolve(prompt_close_bg)),
            ShellAssignment::local("__segment_delimiter_color", colors.resolve(seg_delim_fg)),
            ShellAssignment::local("__segment_delimiter_bg", colors.resolve(seg_delim_bg)),
            ShellAssignment::local("__prompt_base_bg", "normal"),
            ShellAssignment::local("__prompt_base_fg", "normal"),
        ],
    );
}
