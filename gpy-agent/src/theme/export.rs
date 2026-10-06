//! Shell-export serialization: converts a [`ThemeConfig`] into shell variable
//! assignments (`set -g …` / `export …`) for Fish, Bash, and Zsh.
//!
//! This module is intentionally decoupled from [`crate::theme::manager`]:
//! [`theme_to_shell`] and its helpers are free functions operating only on
//! borrowed data (`&ThemeConfig`, `&Config`, `Shell`), so they are testable
//! without constructing a [`crate::theme::manager::ThemeManager`].

use crate::config::{Config, DelimiterConfig};
use crate::shell::{Shell, VariableSyntax};
use crate::theme::model::{PluginSegmentTheme, ThemeConfig};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy)]
enum AssignmentScope {
    Local,
    Export,
    Unset,
}

#[derive(Debug, Clone)]
struct ShellAssignment {
    scope: AssignmentScope,
    name: String,
    value: String,
}

impl ShellAssignment {
    fn local(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            scope: AssignmentScope::Local,
            name: name.into(),
            value: value.into(),
        }
    }

    fn export(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            scope: AssignmentScope::Export,
            name: name.into(),
            value: value.into(),
        }
    }

    fn unset(name: impl Into<String>) -> Self {
        Self {
            scope: AssignmentScope::Unset,
            name: name.into(),
            value: String::new(),
        }
    }

    fn render(self, syntax: VariableSyntax) -> String {
        match self.scope {
            AssignmentScope::Unset => syntax.format_unset(&self.name),
            AssignmentScope::Local => syntax.format(&self.name, &self.value),
            AssignmentScope::Export => syntax.format_export(&self.name, &self.value),
        }
    }
}

fn normalize_variable_token(raw: &str) -> String {
    let mut normalized = String::new();
    let mut prev_was_underscore = false;

    for ch in raw.chars() {
        let is_valid = ch.is_ascii_alphanumeric() || ch == '_';
        let next = if is_valid {
            ch.to_ascii_lowercase()
        } else {
            '_'
        };

        if next == '_' {
            if prev_was_underscore {
                continue;
            }
            prev_was_underscore = true;
        } else {
            prev_was_underscore = false;
        }

        normalized.push(next);
    }

    normalized.trim_matches('_').to_owned()
}

fn append_shell_assignment(
    output: &mut String,
    syntax: VariableSyntax,
    assignment: ShellAssignment,
) {
    output.push_str(&assignment.render(syntax));
    output.push('\n');
}

fn append_shell_assignments(
    output: &mut String,
    syntax: VariableSyntax,
    assignments: impl IntoIterator<Item = ShellAssignment>,
) {
    for assignment in assignments {
        append_shell_assignment(output, syntax, assignment);
    }
}

/// Escape a theme-provided string value for safe interpolation into a
/// double-quoted shell variable assignment (fish `set -g x "…"`, bash/zsh
/// `export x="…"`).
///
/// Theme values (icons, delimiters, `time_format`, `trim_at`, the theme
/// name, and plugin-segment ids/icons) are attacker-influenceable and are
/// only validated against control characters upstream — never against shell
/// metacharacters. Without escaping, a value such as `$(rm -rf ~)` or a
/// backtick command substitution would execute the moment the exported
/// block is sourced by the user's shell. See issue #305.
///
/// Escaping order is significant:
/// 1. Backslashes are doubled *first*, so backslashes introduced by later
///    steps are not themselves re-escaped, and a lone trailing backslash
///    becomes `\\` and can no longer escape the closing quote (which would
///    otherwise produce an unterminated-string syntax error).
/// 2. `$` is neutralized (`\$`) to defeat `$(…)` command substitution and
///    `$var` expansion in all three shells.
/// 3. Backtick command substitution is neutralized (`` \` ``) for bash/zsh.
///    Fish has no backtick command-substitution syntax, so a literal
///    backtick is inert there and is left untouched to preserve value
///    fidelity (escaping it would leak a spurious backslash into the value).
/// 4. The closing `"` is escaped *last*, preserving the historical
///    behavior for the plain-`"`-only case that existing tests assert on.
pub(crate) fn escape_shell_value(text: &str, shell: Shell) -> String {
    let backslash_escaped = text.replace('\\', "\\\\");
    let dollar_escaped = backslash_escaped.replace('$', "\\$");
    let backtick_escaped = match shell {
        Shell::Fish => dollar_escaped,
        Shell::Bash | Shell::Zsh => dollar_escaped.replace('`', "\\`"),
    };
    backtick_escaped.replace('"', "\\\"")
}

/// Emit the two-line provenance header plus its trailing blank line.
///
/// `theme_name` is interpolated raw: it lands inside a `#` comment, where no
/// shell performs expansion, so it deliberately does not go through
/// [`escape_shell_value`]. The `__gpy_theme_name` *assignment* emitted by
/// [`append_global_assignments`] is escaped.
#[expect(
    clippy::expect_used,
    reason = "every .expect(\"string write\") is a writeln!/write! into a String via std::fmt::Write, which is infallible in practice"
)]
fn append_header(output: &mut String, shell: Shell, theme_name: &str) {
    use std::fmt::Write;

    writeln!(output, "# GPY Theme: {theme_name}").expect("string write");
    writeln!(
        output,
        "# Generated by gpy-agent for {}",
        match shell {
            Shell::Fish => "Fish",
            Shell::Zsh => "Zsh",
            Shell::Bash => "Bash",
        }
    )
    .expect("string write");
    output.push('\n');
}

/// Render a boolean setting as the `1`/`0` the shell integration expects.
const fn flag(value: bool) -> &'static str {
    if value { "1" } else { "0" }
}

/// The `GPY_*` environment exports mirroring `config` into the shell.
///
/// These are the only exported (as opposed to shell-local) assignments in the
/// whole block, and none of them carries free text — every value is a `0`/`1`
/// flag or a validated numeric/enum setting — so none needs escaping.
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
    ]
}

/// Agent/git/language/UI settings sourced from `config`, the theme name, and
/// the powerline glyph pair the shell uses to draw chevrons.
fn append_global_assignments(
    output: &mut String,
    shell: Shell,
    theme: &ThemeConfig,
    theme_name: &str,
    config: &Config,
) {
    let syntax = shell.variable_syntax();
    let escape = |text: &str| escape_shell_value(text, shell);

    let prompt_icon_style = if config.ui.show_icons {
        "nerd"
    } else {
        "ascii"
    };
    let powerline_start = theme
        .ui
        .segment_open
        .as_ref()
        .map_or("", |cfg| cfg.icon.as_str());
    let powerline_end = theme
        .ui
        .segment_close
        .as_ref()
        .map_or("", |cfg| cfg.icon.as_str());

    let mut assignments = vec![ShellAssignment::local(
        "__gpy_theme_name",
        escape(theme_name),
    )];
    assignments.extend(config_export_assignments(config));
    assignments.extend([
        ShellAssignment::local("__prompt_icons", prompt_icon_style),
        ShellAssignment::local("__icon_powerline_segment_start", escape(powerline_start)),
        ShellAssignment::local("__icon_powerline_segment_end", escape(powerline_end)),
    ]);
    append_shell_assignments(output, syntax, assignments);
}

/// Prompt and root-prompt icons/colors plus the two-line layout flag.
///
/// The `*_color` assignments are emitted twice in different shapes: the
/// `__gpy_ui_*_icon_color` pair is skipped entirely when the theme leaves the
/// color unset, while `__prompt_color`/`__root_prompt_color` always render,
/// falling back to green/red.
fn append_prompt_assignments(output: &mut String, shell: Shell, theme: &ThemeConfig) {
    let syntax = shell.variable_syntax();
    let escape = |text: &str| escape_shell_value(text, shell);

    let prompt_icon = &theme.ui.prompt_icon;
    append_shell_assignments(
        output,
        syntax,
        [
            ShellAssignment::local("__gpy_ui_prompt_icon", escape(prompt_icon.as_str())),
            ShellAssignment::local("__icon_prompt", escape(prompt_icon.as_str())),
        ],
    );

    if let Some(c) = &theme.ui.prompt_color {
        append_shell_assignment(
            output,
            syntax,
            ShellAssignment::local("__gpy_ui_prompt_icon_color", c.as_str()),
        );
    }
    let prompt_color = theme.ui.prompt_color.as_deref().unwrap_or("green");
    append_shell_assignment(
        output,
        syntax,
        ShellAssignment::local("__prompt_color", prompt_color),
    );

    let root_icon = &theme.ui.root_prompt_icon;
    append_shell_assignments(
        output,
        syntax,
        [
            ShellAssignment::local("__gpy_ui_root_prompt_icon", escape(root_icon.as_str())),
            ShellAssignment::local("__icon_root_prompt", escape(root_icon.as_str())),
        ],
    );

    if let Some(c) = &theme.ui.root_prompt_color {
        append_shell_assignment(
            output,
            syntax,
            ShellAssignment::local("__gpy_ui_root_prompt_icon_color", c.as_str()),
        );
    }
    let root_color = theme.ui.root_prompt_color.as_deref().unwrap_or("red");
    append_shell_assignment(
        output,
        syntax,
        ShellAssignment::local("__root_prompt_color", root_color),
    );

    // Opt-in two-line layout (bash/zsh). Always emit 0/1 so disabling it
    // in-session re-exports 0 and the shell resumes single-line rendering.
    append_shell_assignment(
        output,
        syntax,
        ShellAssignment::local("__gpy_two_line", flag(theme.ui.two_line)),
    );

    // Blank line before each prompt. Always emit 0/1 for the same reason as
    // two_line: turning it off in-session must re-export 0 so the shells stop
    // emitting the separator rather than keeping the last enabled value.
    append_shell_assignment(
        output,
        syntax,
        ShellAssignment::local("__gpy_add_newline", flag(theme.ui.add_newline)),
    );
}

/// Built-in clock, directory, and duration segment settings.
///
/// directory/duration/character render solely from agent-provided ANSI;
/// `__gpy_*_format` toggles were retired in #199 and remain retired.
/// Clock + status keep their shell-side rendering and color exports.
/// `__duration_threshold_ms` stays: the shell uses it for the
/// detect/visibility gate before delegating the render to the agent.
/// `__color_directory_bg` and `__color_duration_bg` are re-added so the
/// shell can track `__gpy_last_segment_bg` for powerline chevron rendering.
fn append_builtin_segment_assignments(output: &mut String, shell: Shell, theme: &ThemeConfig) {
    let syntax = shell.variable_syntax();
    let escape = |text: &str| escape_shell_value(text, shell);
    let bool_flag = |v: Option<bool>| flag(v.unwrap_or(false));

    append_shell_assignments(
        output,
        syntax,
        [
            ShellAssignment::local("__color_clock_bg", theme.segments.clock.bg_color.as_str()),
            ShellAssignment::local("__color_clock_fg", theme.segments.clock.text_color.as_str()),
        ],
    );
    append_shell_assignment(
        output,
        syntax,
        ShellAssignment::local(
            "__time_format",
            escape(theme.segments.clock.time_format.as_deref().unwrap_or("12")),
        ),
    );
    append_shell_assignments(
        output,
        syntax,
        [
            ShellAssignment::local(
                "__clock_show_leading_zero",
                bool_flag(theme.segments.clock.show_leading_zero),
            ),
            ShellAssignment::local(
                "__clock_show_seconds",
                bool_flag(theme.segments.clock.show_seconds),
            ),
            ShellAssignment::local(
                "__duration_threshold_ms",
                theme.segments.duration.show_if_exceeds_ms.to_string(),
            ),
            ShellAssignment::local(
                "__color_directory_bg",
                theme.segments.directory.bg_color.as_str(),
            ),
            ShellAssignment::local(
                "__color_duration_bg",
                theme.segments.duration.bg_color.as_str(),
            ),
        ],
    );
}

/// Hostname segment colors, trimming, and template-path flags.
///
/// The pure-fish path uses `bg`/`fg`/`trim_at`/`show_always`;
/// `format`/`icon` select the agent-rendered template path when `Some`.
/// `format` and `icon` are always assigned (empty string when `None`) so the
/// shell can branch on `test -n "$__hostname_format"` without needing `set -q`.
fn append_hostname_assignments(output: &mut String, shell: Shell, theme: &ThemeConfig) {
    let syntax = shell.variable_syntax();
    let escape = |text: &str| escape_shell_value(text, shell);

    append_shell_assignments(
        output,
        syntax,
        [
            ShellAssignment::local(
                "__color_hostname_bg",
                theme.segments.hostname.bg_color.as_str(),
            ),
            ShellAssignment::local(
                "__color_hostname_fg",
                theme.segments.hostname.text_color.as_str(),
            ),
            ShellAssignment::local(
                "__hostname_trim_at",
                escape(&theme.segments.hostname.trim_at),
            ),
            ShellAssignment::local(
                "__hostname_show_always",
                flag(theme.segments.hostname.show_always),
            ),
            // PRESENCE FLAG ONLY — never the raw format string. The shell
            // side only ever does `test -n "$__hostname_format"`; it never
            // reads the content. The agent re-derives the real
            // Starship-compatible template server-side from the theme
            // when it handles the `hostname` IPC/oneshot request, so the
            // exported value here only needs to signal "a format is
            // configured" ("1") vs "not configured" (""). Exporting the
            // raw format string (even after the `"`-only `escape()`
            // above) would let `$`/backtick/`$(...)` in a theme's format
            // survive into a double-quoted, eval'd shell assignment —
            // a shell-injection risk when a shared theme is sourced, and
            // an unbound-variable hazard (`$symbol`, `$style`, ...) under
            // a user's `set -u`/`nounset`.
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
                    .map_or_else(String::new, |icon| escape(icon.as_str())),
            ),
        ],
    );
}

/// Username segment colors and template-path flags.
///
/// The pure-shell path uses `bg`/`fg`/`show_always`; `format`/`icon`
/// select the agent-rendered template path when `Some`. `__username_format`
/// is a PRESENCE FLAG ONLY (never the raw format string) — same rationale
/// and injection/`set -u` safety as `__hostname_format` above. The shell
/// only ever does `test -n "$__username_format"`; the agent re-derives the
/// real Starship-compatible template server-side.
fn append_username_assignments(output: &mut String, shell: Shell, theme: &ThemeConfig) {
    let syntax = shell.variable_syntax();
    let escape = |text: &str| escape_shell_value(text, shell);

    append_shell_assignments(
        output,
        syntax,
        [
            ShellAssignment::local(
                "__color_username_bg",
                theme.segments.username.bg_color.as_str(),
            ),
            ShellAssignment::local(
                "__color_username_fg",
                theme.segments.username.text_color.as_str(),
            ),
            ShellAssignment::local(
                "__username_show_always",
                flag(theme.segments.username.show_always),
            ),
            ShellAssignment::local(
                "__username_format",
                if theme.segments.username.format.is_some() {
                    "1"
                } else {
                    ""
                },
            ),
            ShellAssignment::local(
                "__icon_username",
                theme
                    .segments
                    .username
                    .icon
                    .as_ref()
                    .map_or_else(String::new, |icon| escape(icon.as_str())),
            ),
        ],
    );
}

/// Status segment icons (erased when the theme leaves them unset) and the
/// four ok/fail color assignments (always emitted).
fn append_status_assignments(output: &mut String, shell: Shell, theme: &ThemeConfig) {
    let syntax = shell.variable_syntax();
    let escape = |text: &str| escape_shell_value(text, shell);

    append_shell_assignment(
        output,
        syntax,
        theme.segments.status.ok_icon.as_ref().map_or_else(
            || ShellAssignment::unset("__icon_status_ok"),
            |icon| ShellAssignment::local("__icon_status_ok", escape(icon.as_str())),
        ),
    );
    append_shell_assignment(
        output,
        syntax,
        theme.segments.status.fail_icon.as_ref().map_or_else(
            || ShellAssignment::unset("__icon_status_fail"),
            |icon| ShellAssignment::local("__icon_status_fail", escape(icon.as_str())),
        ),
    );
    append_shell_assignments(
        output,
        syntax,
        [
            ShellAssignment::local(
                "__color_status_ok_bg",
                theme.segments.status.ok_bg_color.as_str(),
            ),
            ShellAssignment::local(
                "__color_status_ok_fg",
                theme.segments.status.ok_text_color.as_str(),
            ),
            ShellAssignment::local(
                "__color_status_fail_bg",
                theme.segments.status.fail_bg_color.as_str(),
            ),
            ShellAssignment::local(
                "__color_status_fail_fg",
                theme.segments.status.fail_text_color.as_str(),
            ),
        ],
    );
}

/// Background colors for the git and language segments.
///
/// Both render solely from agent-provided ANSI, so most
/// `__color_git_*` / `__color_language_*` exports were retired in #199.
/// The bg variants are re-added (#206) so the shell can track
/// `__gpy_last_segment_bg` for powerline chevrons.
fn append_git_language_color_assignments(output: &mut String, shell: Shell, theme: &ThemeConfig) {
    let syntax = shell.variable_syntax();

    append_shell_assignments(
        output,
        syntax,
        [
            ShellAssignment::local(
                "__color_git_clean_bg",
                theme
                    .segments
                    .git
                    .clean_bg_color
                    .as_deref()
                    .unwrap_or(theme.segments.git.bg_color.as_str()),
            ),
            ShellAssignment::local(
                "__color_git_dirty_bg",
                theme
                    .segments
                    .git
                    .dirty_bg_color
                    .as_deref()
                    .unwrap_or(theme.segments.git.bg_color.as_str()),
            ),
            ShellAssignment::local(
                "__color_language_bg",
                theme.segments.language.bg_color.as_str(),
            ),
        ],
    );
}

/// Icon-color/background pair for one optional UI delimiter, falling back to
/// the historical `white`/`transparent` defaults when the theme omits it.
fn delimiter_colors(delimiter: Option<&DelimiterConfig>) -> (&str, &str) {
    delimiter.map_or(("white", "transparent"), |cfg| {
        (cfg.icon_color.as_str(), cfg.bg_color.as_str())
    })
}

/// The four delimiter glyphs and the prompt/segment delimiter colors.
#[expect(
    clippy::similar_names,
    reason = "fg/bg color pairs (seg_delim_fg/seg_delim_bg, prompt_open_fg/prompt_open_bg, prompt_close_fg/prompt_close_bg) are deliberately paired names, not accidental near-collisions"
)]
fn append_delimiter_assignments(output: &mut String, shell: Shell, theme: &ThemeConfig) {
    let syntax = shell.variable_syntax();
    let escape = |text: &str| escape_shell_value(text, shell);

    let (prompt_open_fg, prompt_open_bg) = delimiter_colors(theme.ui.prompt_open.as_ref());
    let (prompt_close_fg, prompt_close_bg) = delimiter_colors(theme.ui.prompt_close.as_ref());
    let (seg_delim_fg, seg_delim_bg) = delimiter_colors(theme.ui.segment_open.as_ref());
    append_shell_assignments(
        output,
        syntax,
        [
            ShellAssignment::local(
                "__segment_delim_first",
                escape(theme.ui.get_prompt_open_delimiter()),
            ),
            ShellAssignment::local(
                "__segment_delim_start",
                escape(theme.ui.get_segment_open_delimiter()),
            ),
            ShellAssignment::local(
                "__segment_delim_end",
                escape(theme.ui.get_segment_close_delimiter()),
            ),
            ShellAssignment::local(
                "__segment_delim_last",
                escape(theme.ui.get_prompt_close_delimiter()),
            ),
            ShellAssignment::local("__prompt_open_color", prompt_open_fg),
            ShellAssignment::local("__prompt_open_bg", prompt_open_bg),
            ShellAssignment::local("__prompt_close_color", prompt_close_fg),
            ShellAssignment::local("__prompt_close_bg", prompt_close_bg),
            ShellAssignment::local("__segment_delimiter_color", seg_delim_fg),
            ShellAssignment::local("__segment_delimiter_bg", seg_delim_bg),
            ShellAssignment::local("__prompt_base_bg", "normal"),
            ShellAssignment::local("__prompt_base_fg", "normal"),
        ],
    );
}

/// Style assignments for one plugin/community segment: its id, colors, icon,
/// and optional open/close delimiters.
///
/// Each color and icon is emitted twice — once namespaced under
/// `__gpy_segment_<token>_*`, once under the flat `__color_<token>_*` /
/// `__icon_<token>` names the built-in segments already use — so community
/// segments can read either contract.
fn plugin_segment_style_assignments(
    shell: Shell,
    segment_name: &str,
    segment_token: &str,
    segment_theme: &PluginSegmentTheme,
) -> Vec<ShellAssignment> {
    let escape = |text: &str| escape_shell_value(text, shell);
    let gpy_prefix = format!("__gpy_segment_{segment_token}");
    let mut assignments = vec![ShellAssignment::local(
        format!("{gpy_prefix}_id"),
        escape(segment_name),
    )];

    if let Some(bg_color) = segment_theme.bg_color.as_deref() {
        assignments.push(ShellAssignment::local(
            format!("{gpy_prefix}_bg_color"),
            bg_color,
        ));
        assignments.push(ShellAssignment::local(
            format!("__color_{segment_token}_bg"),
            bg_color,
        ));
    }

    if let Some(text_color) = segment_theme.text_color.as_deref() {
        assignments.push(ShellAssignment::local(
            format!("{gpy_prefix}_text_color"),
            text_color,
        ));
        assignments.push(ShellAssignment::local(
            format!("__color_{segment_token}_fg"),
            text_color,
        ));
    }

    if let Some(icon) = segment_theme.icon.as_ref() {
        assignments.push(ShellAssignment::local(
            format!("{gpy_prefix}_icon"),
            escape(icon.as_str()),
        ));
        assignments.push(ShellAssignment::local(
            format!("__icon_{segment_token}"),
            escape(icon.as_str()),
        ));
    }

    assignments.extend(plugin_segment_delimiter_assignments(
        shell,
        &gpy_prefix,
        "open",
        segment_theme.open.as_ref(),
    ));
    assignments.extend(plugin_segment_delimiter_assignments(
        shell,
        &gpy_prefix,
        "close",
        segment_theme.close.as_ref(),
    ));

    assignments
}

/// The `_<side>_icon` / `_<side>_icon_color` / `_<side>_bg_color` triple for
/// one side of a plugin segment's delimiters, or nothing when that side is
/// unset. `side` is `"open"` or `"close"`.
fn plugin_segment_delimiter_assignments(
    shell: Shell,
    gpy_prefix: &str,
    side: &str,
    delimiter: Option<&DelimiterConfig>,
) -> Vec<ShellAssignment> {
    let Some(config) = delimiter else {
        return Vec::new();
    };
    vec![
        ShellAssignment::local(
            format!("{gpy_prefix}_{side}_icon"),
            escape_shell_value(config.icon.as_str(), shell),
        ),
        ShellAssignment::local(
            format!("{gpy_prefix}_{side}_icon_color"),
            config.icon_color.as_str(),
        ),
        ShellAssignment::local(
            format!("{gpy_prefix}_{side}_bg_color"),
            config.bg_color.as_str(),
        ),
    ]
}

/// Free-form `properties` of one plugin/community segment, in sorted key
/// order.
///
/// A property is always emitted namespaced as `__gpy_segment_<token>_<prop>`.
/// A `*_color` or `*_icon` property additionally gets the flat
/// `__color_<token>_<base>` / `__icon_<token>_<base>` alias, so a community
/// segment can consume the same variable shape as a built-in one.
fn plugin_segment_property_assignments(
    shell: Shell,
    segment_token: &str,
    segment_theme: &PluginSegmentTheme,
) -> Vec<ShellAssignment> {
    let escape = |text: &str| escape_shell_value(text, shell);
    let gpy_prefix = format!("__gpy_segment_{segment_token}");
    let mut assignments = Vec::new();

    let mut property_names: Vec<_> = segment_theme.properties.keys().collect();
    property_names.sort_unstable();
    for property_name in property_names {
        let Some(value) = segment_theme
            .properties
            .get(property_name)
            .map(String::as_str)
        else {
            continue;
        };
        let property_token = normalize_variable_token(property_name);
        if property_token.is_empty() {
            continue;
        }

        assignments.push(ShellAssignment::local(
            format!("{gpy_prefix}_{property_token}"),
            escape(value),
        ));

        if property_name.ends_with("_color") {
            let base = property_name.trim_end_matches("_color");
            let base_token = normalize_variable_token(base);
            if !base_token.is_empty() {
                assignments.push(ShellAssignment::local(
                    format!("__color_{segment_token}_{base_token}"),
                    value,
                ));
            }
        }
        if property_name.ends_with("_icon") {
            let base = property_name.trim_end_matches("_icon");
            let base_token = normalize_variable_token(base);
            if !base_token.is_empty() {
                assignments.push(ShellAssignment::local(
                    format!("__icon_{segment_token}_{base_token}"),
                    escape(value),
                ));
            }
        }
    }

    assignments
}

/// Generic/community segment style exports, in sorted segment-name order.
///
/// Built-in segment variable contracts remain unchanged and continue to be
/// emitted by the phases above for backward compatibility.
fn append_plugin_segment_assignments(output: &mut String, shell: Shell, theme: &ThemeConfig) {
    let syntax = shell.variable_syntax();

    let mut plugin_segments: Vec<_> = theme.segments.plugin.iter().collect();
    plugin_segments.sort_unstable_by_key(|(left_name, _)| *left_name);

    for (segment_name, segment_theme) in plugin_segments {
        let segment_token = normalize_variable_token(segment_name);
        if segment_token.is_empty() {
            continue;
        }

        append_shell_assignments(
            output,
            syntax,
            plugin_segment_style_assignments(shell, segment_name, &segment_token, segment_theme),
        );
        append_shell_assignments(
            output,
            syntax,
            plugin_segment_property_assignments(shell, &segment_token, segment_theme),
        );
    }
}

/// Export plugin-provided segment file paths so fish can source community
/// segments without hardcoded path allowlists.
fn append_plugin_segment_file_assignments(
    output: &mut String,
    shell: Shell,
    plugin_segment_files: &BTreeMap<String, String>,
) {
    let syntax = shell.variable_syntax();
    let escape = |text: &str| escape_shell_value(text, shell);

    for (segment_name, segment_file) in plugin_segment_files {
        let segment_token = normalize_variable_token(segment_name);
        if segment_token.is_empty() {
            continue;
        }
        append_shell_assignment(
            output,
            syntax,
            ShellAssignment::local(
                format!("__gpy_plugin_segment_file_{segment_token}"),
                escape(segment_file),
            ),
        );
    }
}

/// The enabled-segment list, in the one place `theme_to_shell` branches on
/// per-shell array syntax.
///
/// `git` and `language` are dropped from the list when their subsystem is
/// disabled in `config`; every other segment passes through untouched.
#[expect(
    clippy::expect_used,
    reason = "every .expect(\"string write\") is a writeln!/write! into a String via std::fmt::Write, which is infallible in practice"
)]
fn append_enabled_segments(output: &mut String, shell: Shell, config: &Config) {
    use std::fmt::Write;

    let filtered: Vec<String> = config
        .ui
        .enabled_segments
        .iter()
        .filter(|s| match s.as_str() {
            "git" => config.git.enabled,
            "language" => config.language.enabled,
            _ => true,
        })
        .cloned()
        .collect();
    let segments = filtered.join(" ");

    // Each shell gets the list in the shape its render loop consumes:
    // Fish and Zsh iterate a real list/array, Bash's `__gpy_render_prompt`
    // word-splits a space-separated string (`for segment in
    // $__enabled_segments`, matching constants.bash's default). The Bash
    // arm used to emit an array, and `$__enabled_segments` expands to an
    // array's first element only, so every Bash prompt rendered just the
    // first enabled segment (found by tests/bash/e2e_agent_autostart, #646).
    match shell {
        Shell::Fish => {
            writeln!(output, "set -g __enabled_segments {segments}").expect("string write");
        }
        Shell::Zsh => {
            writeln!(output, "typeset -g -a __enabled_segments=({segments})")
                .expect("string write");
        }
        Shell::Bash => {
            writeln!(output, "export __enabled_segments=\"{segments}\"").expect("string write");
        }
    }
}

/// The language-detection marker file names, sorted, as the list the shell
/// language pre-filters iterate with builtin file tests (#785).
///
/// The agent's `marker_file_names()` is the single source: the shells keep no
/// hand-written list. Names are static, shell-safe file names, so they need no
/// escaping. Fish gets a real list, Zsh and Bash a real array (never exported:
/// Bash cannot export arrays, and the export is sourced into the shell itself).
#[expect(
    clippy::expect_used,
    reason = "every .expect(\"string write\") is a writeln!/write! into a String via std::fmt::Write, which is infallible in practice"
)]
fn append_lang_marker_files(output: &mut String, shell: Shell) {
    use std::fmt::Write;

    let mut names: Vec<&str> = crate::language::metadata::marker_file_names()
        .iter()
        .copied()
        .collect();
    names.sort_unstable();
    let joined = names.join(" ");

    match shell {
        Shell::Fish => {
            writeln!(output, "set -g __gpy_lang_marker_files {joined}").expect("string write");
        }
        Shell::Zsh => {
            writeln!(output, "typeset -g -a __gpy_lang_marker_files=({joined})")
                .expect("string write");
        }
        Shell::Bash => {
            writeln!(output, "__gpy_lang_marker_files=({joined})").expect("string write");
        }
    }
}

/// The built-in segments a powerline chevron cares about, paired with the
/// `__color_*_bg` variable each one's background comes from -- in the exact
/// order Bash/Zsh emit `__gpy_segment_bg`'s case arms.
///
/// This is the SAME data `append_builtin_segment_assignments`,
/// `append_hostname_assignments`, `append_username_assignments`, and
/// `append_git_language_color_assignments` above already use to name these
/// variables; `append_segment_bg_function` below is the one place that reads
/// it to generate the Bash/Zsh case table (#614), replacing the four
/// hand-copied `case "$segment" in ... esac` tables that used to be split
/// across `bash/core/init.bash` and `zsh/core/init.zsh`. Not the same set as
/// [`crate::plugin::BuiltinSegment`] (git/clock/duration/
/// language/directory/status): that type's "six builtin segments" is a
/// config/CLI concept, while a chevron background is a distinct, Bash/Zsh-only
/// rendering concern that also covers hostname/username but has no `status`
/// entry (the status segment owns no powerline background of its own).
pub(crate) const SEGMENT_BG_VARS: &[(&str, &str)] = &[
    ("git", "__color_git_clean_bg"),
    ("language", "__color_language_bg"),
    ("directory", "__color_directory_bg"),
    ("duration", "__color_duration_bg"),
    ("clock", "__color_clock_bg"),
    ("hostname", "__color_hostname_bg"),
    ("username", "__color_username_bg"),
];

/// Emit a generated `__gpy_segment_bg` function for Bash and Zsh (#614).
///
/// Two calling conventions, sharing this one case table (built from
/// `SEGMENT_BG_VARS`) instead of the four hand-copied ones this replaces:
///   `__gpy_segment_bg SEGMENT`           -> value on stdout (external
///                                          callers, existing tests)
///   `__gpy_segment_bg SEGMENT OUT_VAR`   -> value written into `$OUT_VAR`
///                                          via `printf -v` so Bash/Zsh's
///                                          render-loop lookup never forks a
///                                          subshell on the hot path (mirrors
///                                          the fork-free `printf -v` pattern
///                                          already used elsewhere in those
///                                          shells' `ipc.bash`/`ipc.zsh`)
///
/// A no-op for Fish: Fish tracks `__gpy_last_segment_bg` directly on each
/// segment implementation rather than through a shared table, so its export
/// is untouched by this addition.
#[expect(
    clippy::expect_used,
    reason = "every .expect(\"string write\") is a writeln!/write! into a String via std::fmt::Write, which is infallible in practice"
)]
fn append_segment_bg_function(output: &mut String, shell: Shell) {
    use std::fmt::Write;

    if matches!(shell, Shell::Fish) {
        return;
    }

    output.push('\n');
    writeln!(output, "__gpy_segment_bg() {{").expect("string write");
    writeln!(output, "    if [[ -n \"$2\" ]]; then").expect("string write");
    writeln!(output, "        case \"$1\" in").expect("string write");
    for (segment, var) in SEGMENT_BG_VARS {
        writeln!(
            output,
            "            {segment}) printf -v \"$2\" '%s' \"${{{var}:-}}\" ;;"
        )
        .expect("string write");
    }
    writeln!(output, "            *) printf -v \"$2\" '%s' \"\" ;;").expect("string write");
    writeln!(output, "        esac").expect("string write");
    writeln!(output, "    else").expect("string write");
    writeln!(output, "        case \"$1\" in").expect("string write");
    for (segment, var) in SEGMENT_BG_VARS {
        writeln!(
            output,
            "            {segment}) printf '%s' \"${{{var}:-}}\" ;;"
        )
        .expect("string write");
    }
    writeln!(output, "            *) printf '%s' \"\" ;;").expect("string write");
    writeln!(output, "        esac").expect("string write");
    writeln!(output, "    fi").expect("string write");
    writeln!(output, "}}").expect("string write");
}

/// Serialize a theme into a block of shell assignments for `shell`.
///
/// This is the top-level ordering function and nothing more: the sequence of
/// calls below *is* the export contract. Section order, variable names, and
/// quoting are byte-for-byte stable, and `src/theme/snapshots/` holds golden
/// exports for all three shells that fail on any drift (#623). Every
/// attacker-influenceable free-text value passes through
/// [`escape_shell_value`], the single escaping boundary; colors are emitted
/// unescaped because they are validated against the color schema upstream.
pub(crate) fn theme_to_shell(
    theme: &ThemeConfig,
    theme_name: &str,
    config: &Config,
    shell: Shell,
    plugin_segment_files: &BTreeMap<String, String>,
) -> String {
    let mut output = String::new();

    append_header(&mut output, shell, theme_name);
    append_global_assignments(&mut output, shell, theme, theme_name, config);
    append_prompt_assignments(&mut output, shell, theme);

    append_builtin_segment_assignments(&mut output, shell, theme);
    append_hostname_assignments(&mut output, shell, theme);
    append_username_assignments(&mut output, shell, theme);
    append_status_assignments(&mut output, shell, theme);
    append_git_language_color_assignments(&mut output, shell, theme);
    append_delimiter_assignments(&mut output, shell, theme);

    append_plugin_segment_assignments(&mut output, shell, theme);

    append_plugin_segment_file_assignments(&mut output, shell, plugin_segment_files);
    append_enabled_segments(&mut output, shell, config);
    append_lang_marker_files(&mut output, shell);
    append_segment_bg_function(&mut output, shell);

    output
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    use super::{SEGMENT_BG_VARS, escape_shell_value, theme_to_shell};
    use crate::config::Config;
    use crate::shell::Shell;
    use crate::theme::model::ThemeConfig;
    use std::collections::BTreeMap;
    use std::process::Command;

    // ---- Full-export golden coverage (issue #623) ----------------------------

    /// Theme name carrying shell metacharacters.
    ///
    /// It pins the escaping applied to the header comment and to
    /// `__gpy_theme_name`.
    const GOLDEN_THEME_NAME: &str = "night$owl\"";

    /// A theme that populates every export phase.
    ///
    /// It covers prompt icons and colors,
    /// all four UI delimiters, the built-in clock/directory/duration/status/
    /// git/language segments, hostname and username (both with a `format`
    /// present so the presence flags render as `1`), and two plugin segments
    /// exercising delimiters, custom properties, and the `_color`/`_icon`
    /// property suffix aliases. Several values embed `$`, backticks, quotes,
    /// and a trailing backslash so the goldens also pin escaping behavior.
    const GOLDEN_THEME_TOML: &str = r##"
[ui]
prompt_icon = "➜$p"
root_prompt_icon = "#`r`"
prompt_color = "magenta"
root_prompt_color = "yellow"
two_line = true

[ui.prompt_open]
icon = "["
icon_color = "cyan"
bg_color = "blue"

[ui.prompt_close]
icon = "]"
icon_color = "green"
bg_color = "red"

[ui.segment_open]
icon = ""
icon_color = "white"
bg_color = "black"

[ui.segment_close]
icon = ""
icon_color = "black"
bg_color = "white"

[segments.clock]
bg_color = "blue"
text_color = "white"
time_format = "%H:%M$(date)"
show_leading_zero = true
show_seconds = true

[segments.directory]
bg_color = "cyan"
text_color = "black"

[segments.duration]
bg_color = "yellow"
text_color = "black"
show_if_exceeds_ms = 2500

[segments.hostname]
format = "on [$hostname]($style)"
icon = "host\\"
bg_color = "green"
text_color = "black"
trim_at = ".local\""
show_always = true

[segments.username]
format = "[$user]($style)"
icon = "user`x`"
bg_color = "red"
text_color = "white"
show_always = true

[segments.status]
ok_icon = "ok$1"
fail_icon = "fail`2`"
ok_bg_color = "green"
ok_text_color = "black"
fail_bg_color = "red"
fail_text_color = "white"

[segments.git]
bg_color = "purple"
clean_bg_color = "green"
dirty_bg_color = "red"

[segments.language]
bg_color = "blue"

[segments.kubernetes]
icon = "k8s$ctx"
bg_color = "blue"
text_color = "white"
context_color = "bright-blue"
namespace_icon = "ns`n`"
plain_property = "value\\"

[segments."aws profile"]
icon = "aws"
bg_color = "yellow"
text_color = "black"

[segments."aws profile".open]
icon = "<"
icon_color = "white"
bg_color = "black"

[segments."aws profile".close]
icon = ">"
icon_color = "black"
bg_color = "white"
"##;

    /// A config that moves every exported setting off its default.
    ///
    /// It drops `language` from the enabled-segment list via
    /// `language.enabled = false` while keeping `git` (which stays because
    /// `git.enabled = true`), so the goldens pin both arms of the
    /// enabled-segment filter.
    const GOLDEN_CONFIG_TOML: &str = r#"
[agent]
enabled = false
live_updates = false
timeout_seconds = 7

[agent.supervisor]
enabled = false
check_interval_seconds = 45
max_restart_attempts = 9

[git]
enabled = true
show_upstream = false
max_branch_length = 17

[language]
enabled = false
show_versions = false

[ui]
show_icons = false
enabled_segments = ["clock", "username", "hostname", "directory", "git", "language", "duration", "status"]

[ui.directory]
display = "truncated"
max_length = 42
"#;

    fn golden_theme() -> ThemeConfig {
        toml::from_str(GOLDEN_THEME_TOML).expect("golden theme fixture should parse")
    }

    fn golden_config() -> Config {
        toml::from_str(GOLDEN_CONFIG_TOML).expect("golden config fixture should parse")
    }

    /// Plugin-provided segment files, including one whose name normalizes to
    /// an empty token (and must therefore be skipped) and one whose path
    /// carries a shell metacharacter.
    fn golden_plugin_segment_files() -> BTreeMap<String, String> {
        BTreeMap::from([
            (
                "kubernetes".to_owned(),
                "/usr/share/gpy/segments/kubernetes.fish".to_owned(),
            ),
            (
                "aws profile".to_owned(),
                "/usr/share/gpy/segments/aws$profile.fish".to_owned(),
            ),
            ("---".to_owned(), "/skipped/empty-token.fish".to_owned()),
        ])
    }

    fn golden_export(shell: Shell) -> String {
        theme_to_shell(
            &golden_theme(),
            GOLDEN_THEME_NAME,
            &golden_config(),
            shell,
            &golden_plugin_segment_files(),
        )
    }

    /// The exported marker list is exactly `marker_file_names()`, sorted (#785).
    #[test]
    fn export_lists_exactly_the_sorted_marker_file_names() {
        let mut expected: Vec<&str> = crate::language::metadata::marker_file_names()
            .iter()
            .copied()
            .collect();
        expected.sort_unstable();
        for shell in [Shell::Fish, Shell::Zsh, Shell::Bash] {
            let export = golden_export(shell);
            let line = export
                .lines()
                .find(|l| l.contains("__gpy_lang_marker_files"))
                .expect("marker export line");
            let body = match shell {
                Shell::Fish => line.strip_prefix("set -g __gpy_lang_marker_files "),
                Shell::Zsh => line
                    .strip_prefix("typeset -g -a __gpy_lang_marker_files=(")
                    .and_then(|r| r.strip_suffix(')')),
                Shell::Bash => line
                    .strip_prefix("__gpy_lang_marker_files=(")
                    .and_then(|r| r.strip_suffix(')')),
            }
            .expect("marker line shape");
            let got: Vec<&str> = body.split(' ').collect();
            assert_eq!(got, expected, "{shell}");
        }
    }

    #[test]
    fn fish_export_matches_golden() {
        insta::assert_snapshot!("fish_export", golden_export(Shell::Fish));
    }

    #[test]
    fn bash_export_matches_golden() {
        insta::assert_snapshot!("bash_export", golden_export(Shell::Bash));
    }

    #[test]
    fn zsh_export_matches_golden() {
        insta::assert_snapshot!("zsh_export", golden_export(Shell::Zsh));
    }

    /// Bash and Zsh exports carry a generated `__gpy_segment_bg` function.
    ///
    /// #614: covers every built-in segment the shells' render loops track a
    /// powerline chevron background for, built from the single Rust table
    /// (`SEGMENT_BG_VARS`) instead of four hand-copied `case` tables split
    /// across `bash/core/init.bash`/`zsh/core/init.zsh`. Fish tracks
    /// `__gpy_last_segment_bg` per-segment instead and gets no such function.
    #[test]
    fn bash_and_zsh_export_include_segment_bg_function_for_every_builtin_segment() {
        for shell in [Shell::Bash, Shell::Zsh] {
            let export = golden_export(shell);
            assert!(
                export.contains("__gpy_segment_bg()"),
                "{shell:?} export must define __gpy_segment_bg(), got:\n{export}"
            );
            for (segment, var) in SEGMENT_BG_VARS {
                let arm = format!("{segment}) printf");
                assert!(
                    export.contains(&arm),
                    "{shell:?} export's __gpy_segment_bg must have a case arm for \
                     segment '{segment}', got:\n{export}"
                );
                let var_ref = format!("${{{var}:-}}");
                assert!(
                    export.contains(&var_ref),
                    "{shell:?} export's __gpy_segment_bg case arm for '{segment}' must \
                     reference {var_ref}, got:\n{export}"
                );
            }
        }
    }

    #[test]
    fn fish_export_has_no_segment_bg_function() {
        let export = golden_export(Shell::Fish);
        assert!(
            !export.contains("__gpy_segment_bg"),
            "Fish export must be untouched by the Bash/Zsh __gpy_segment_bg addition, got:\n{export}"
        );
    }

    /// Companion golden for the *absent* arms the populated fixture misses.
    ///
    /// Namely: `prompt_color`/`root_prompt_color` unset (their assignments
    /// are skipped entirely), `clock.time_format` unset (renders the `12`
    /// default), `status.ok_icon`/`status.fail_icon` unset (rendered as
    /// erase statements), `hostname.format`/`username.format` unset (presence
    /// flags render empty), and no plugin segments or plugin files at all.
    #[test]
    fn default_theme_export_matches_golden() {
        let export = theme_to_shell(
            &ThemeConfig::default(),
            "default",
            &Config::default(),
            Shell::Fish,
            &BTreeMap::new(),
        );
        insta::assert_snapshot!("default_theme_fish_export", export);
    }

    // ---- Shell-injection escaping (issue #305) test support ------------------

    fn shell_bin(shell: Shell) -> &'static str {
        match shell {
            Shell::Fish => "fish",
            Shell::Bash => "bash",
            Shell::Zsh => "zsh",
        }
    }

    /// Skip guard so the suite still passes on hosts missing a given shell,
    /// mirroring the `--version` availability check in `tests/*integration*.rs`.
    ///
    /// The exit status matters, not just the spawn. `output().is_ok()` is true
    /// for any child that *started*, whatever it did next, so a `bash.exe` on
    /// `PATH` that launches and then fails passed this guard — and every
    /// assertion downstream failed with `sourcing ... errored`, identically,
    /// whatever the payload. That is what took down four tests on
    /// `windows-latest` while a real Git Bash on a Windows host passed all
    /// four (#528): the failures were never about `escape_shell_value`.
    ///
    /// The diagnostics below print which binary was found and what it did.
    /// `cargo test` shows them only for a failing test, which is exactly when
    /// a CI log needs to say whether the shell under test was the real one.
    fn shell_available(shell: Shell) -> bool {
        let bin = shell_bin(shell);
        match Command::new(bin).arg("--version").output() {
            Ok(output) if output.status.success() => {
                let banner = String::from_utf8_lossy(&output.stdout);
                let first_line = banner.lines().next().unwrap_or("(no version banner)");
                eprintln!("using {shell:?}: `{bin} --version` -> {first_line}");
                true
            }
            Ok(output) => {
                eprintln!(
                    "skipping {shell:?}: `{bin} --version` spawned but exited {}",
                    output.status
                );
                false
            }
            Err(error) => {
                eprintln!("skipping {shell:?}: cannot run `{bin}`: {error}");
                false
            }
        }
    }

    /// The skip contract for a missing shell (#650).
    ///
    /// Locally the shell is skipped; under `CI` a missing shell fails the
    /// test, because the injection-safety suite covered zero shells on
    /// `windows-latest` for months without anyone noticing. A runner that
    /// genuinely ships no shell declares it with `GPY_CI_NO_SHELLS=1`, as
    /// the Windows gate in `.github/workflows/pr-gate.yml` does.
    fn skip_shell_or_fail_under_ci(shell: Shell) {
        eprintln!("SKIP: {shell:?} is not available");
        let ci = std::env::var_os("CI").is_some_and(|value| !value.is_empty());
        let no_shells_declared = std::env::var_os("GPY_CI_NO_SHELLS").is_some();
        assert!(
            !ci || no_shells_declared,
            "{shell:?} is not available under CI; install it or set GPY_CI_NO_SHELLS=1 (#650)"
        );
    }

    /// A collision-free marker path a successful injection would `touch`.
    fn unique_marker(tag: &str) -> std::path::PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0_u128, |d| d.as_nanos());
        std::env::temp_dir().join(format!(
            "gpy_test_pwned_{tag}_{}_{nanos}",
            std::process::id()
        ))
    }

    /// Build the exact double-quoted assignment the exporter emits for `raw`,
    /// source it in the real shell, and echo the resulting variable back.
    /// Returns `(sourcing_succeeded, echoed_value)`.
    fn source_value(shell: Shell, raw: &str) -> (bool, String) {
        let syntax = shell.variable_syntax();
        let escaped = escape_shell_value(raw, shell);
        let assignment = syntax.format("__gpy_injection_probe", &escaped);
        let script = format!("{assignment}\nprintf '%s' \"$__gpy_injection_probe\"\n");
        let output = Command::new(shell_bin(shell))
            .arg("-c")
            .arg(&script)
            .output()
            .expect("spawning shell should succeed");
        (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        )
    }

    #[test]
    fn escape_shell_value_neutralizes_metacharacters() {
        // `$(…)` command substitution is neutralized for all shells.
        assert_eq!(escape_shell_value("$(x)", Shell::Fish), "\\$(x)");
        assert_eq!(escape_shell_value("$(x)", Shell::Bash), "\\$(x)");
        assert_eq!(escape_shell_value("$(x)", Shell::Zsh), "\\$(x)");
        // Backslash is doubled (so a trailing backslash cannot escape the quote).
        assert_eq!(escape_shell_value("a\\", Shell::Fish), "a\\\\");
        // Double quote is escaped last (preserves historical behavior).
        assert_eq!(escape_shell_value("a\"b", Shell::Fish), "a\\\"b");
        // Backtick: escaped for bash/zsh, left literal for fish (no backtick
        // substitution in fish; escaping would add a spurious backslash).
        assert_eq!(escape_shell_value("`x`", Shell::Bash), "\\`x\\`");
        assert_eq!(escape_shell_value("`x`", Shell::Zsh), "\\`x\\`");
        assert_eq!(escape_shell_value("`x`", Shell::Fish), "`x`");
        // Ordering: backslash doubled *before* dollar is escaped, so the
        // backslash we insert for `$` is not itself re-doubled.
        assert_eq!(escape_shell_value("\\$", Shell::Fish), "\\\\\\$");
    }

    #[test]
    fn theme_values_do_not_execute_command_substitution_when_sourced() {
        for shell in [Shell::Fish, Shell::Bash, Shell::Zsh] {
            if !shell_available(shell) {
                skip_shell_or_fail_under_ci(shell);
                continue;
            }
            let marker = unique_marker("cmdsub");
            let _ = std::fs::remove_file(&marker);
            assert!(!marker.exists(), "marker must not pre-exist");
            let payload = format!("$(touch {})", marker.display());

            let (ok, echoed) = source_value(shell, &payload);

            assert!(ok, "{shell:?}: sourcing the escaped assignment errored");
            assert!(
                !marker.exists(),
                "{shell:?}: command substitution EXECUTED — marker was created"
            );
            assert_eq!(
                echoed, payload,
                "{shell:?}: value should round-trip as inert literal text"
            );
            let _ = std::fs::remove_file(&marker);
        }
    }

    #[test]
    fn theme_values_do_not_execute_backtick_substitution_when_sourced() {
        // Fish has no backtick command substitution, so this vector is bash/zsh.
        for shell in [Shell::Bash, Shell::Zsh] {
            if !shell_available(shell) {
                skip_shell_or_fail_under_ci(shell);
                continue;
            }
            let marker = unique_marker("backtick");
            let _ = std::fs::remove_file(&marker);
            assert!(!marker.exists(), "marker must not pre-exist");
            let payload = format!("`touch {}`", marker.display());

            let (ok, echoed) = source_value(shell, &payload);

            assert!(ok, "{shell:?}: sourcing the escaped assignment errored");
            assert!(
                !marker.exists(),
                "{shell:?}: backtick substitution EXECUTED — marker was created"
            );
            assert_eq!(
                echoed, payload,
                "{shell:?}: value should round-trip as inert literal text"
            );
            let _ = std::fs::remove_file(&marker);
        }
    }

    #[test]
    fn theme_values_with_shell_metacharacters_source_cleanly() {
        let values = [
            "back\\slash",
            "dollar$var",
            "tick`cmd`",
            "quote\"end",
            "line1\nline2",
            "mix $(x) `y` \"z\" \\ end",
        ];
        for shell in [Shell::Fish, Shell::Bash, Shell::Zsh] {
            if !shell_available(shell) {
                skip_shell_or_fail_under_ci(shell);
                continue;
            }
            for raw in values {
                let (ok, echoed) = source_value(shell, raw);
                assert!(ok, "{shell:?}: sourcing value {raw:?} errored");
                assert_eq!(
                    echoed, raw,
                    "{shell:?}: value {raw:?} did not round-trip losslessly"
                );
            }
        }
    }

    #[test]
    fn trailing_backslash_value_does_not_break_sourcing() {
        // A lone trailing backslash previously escaped the closing quote,
        // producing an unterminated-string syntax error (fish especially).
        let raw = "icon\\";
        for shell in [Shell::Fish, Shell::Bash, Shell::Zsh] {
            if !shell_available(shell) {
                skip_shell_or_fail_under_ci(shell);
                continue;
            }
            let (ok, echoed) = source_value(shell, raw);
            assert!(
                ok,
                "{shell:?}: trailing-backslash value broke sourcing (unterminated string)"
            );
            assert_eq!(
                echoed, raw,
                "{shell:?}: trailing backslash did not round-trip"
            );
        }
    }

    /// Shell script body printing every `__*` variable as sorted-by-caller
    /// `name=value` lines (names only; no `__*` variable is skipped).
    const fn dump_script(shell: Shell) -> &'static str {
        match shell {
            Shell::Fish => "for v in (set -n | string match '__*'); echo $v=$$v; end",
            Shell::Bash => r#"for v in $(compgen -v __); do printf '%s=%s\n' "$v" "${!v}"; done"#,
            Shell::Zsh => r#"for v in ${(k)parameters[(I)__*]}; do print -r -- "$v=${(P)v}"; done"#,
        }
    }

    /// Source `files` in order in a clean shell and return the sorted dump.
    fn sourced_vars(shell: Shell, files: &[&std::path::Path]) -> Vec<String> {
        let mut script = String::new();
        for index in 1..=files.len() {
            let line = match shell {
                Shell::Fish => format!("source $argv[{index}]\n"),
                Shell::Bash | Shell::Zsh => format!("source \"${index}\"\n"),
            };
            script.push_str(&line);
        }
        script.push_str(dump_script(shell));
        let mut cmd = Command::new(shell_bin(shell));
        cmd.env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default());
        match shell {
            Shell::Fish => cmd.args(["--no-config", "-c", &script, "--"]),
            Shell::Bash => cmd.args(["--noprofile", "--norc", "-c", &script, "bash"]),
            Shell::Zsh => cmd.args(["-f", "-c", &script, "zsh"]),
        };
        let output = cmd.args(files).output().expect("spawning shell");
        assert!(
            output.status.success(),
            "{shell:?}: sourcing failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_owned)
            .collect();
        lines.sort();
        lines
    }

    /// Hot-reload invariant (#791).
    ///
    /// Re-sourcing export B over export A leaves the shell's whole `__*`
    /// variable set equal to a fresh shell that sourced only B, so no
    /// optional variable can leak across a theme switch.
    #[test]
    fn optional_shell_vars_reset_on_resource() {
        use crate::config::types::Icon;

        let mut theme_a = ThemeConfig::default();
        theme_a.segments.clock.time_format = Some("24".to_owned());
        theme_a.segments.status.ok_icon = Some(Icon::new("OK").expect("valid icon"));
        theme_a.segments.status.fail_icon = Some(Icon::new("NO").expect("valid icon"));
        let theme_b = ThemeConfig::default();
        let config = Config::default();
        let dir = tempfile::tempdir().expect("tempdir");

        for shell in [Shell::Fish, Shell::Bash, Shell::Zsh] {
            if !shell_available(shell) {
                skip_shell_or_fail_under_ci(shell);
                continue;
            }
            let export =
                |theme: &ThemeConfig| theme_to_shell(theme, "t", &config, shell, &BTreeMap::new());
            let path_a = dir
                .path()
                .join(format!("a.{shell_bin}", shell_bin = shell_bin(shell)));
            let path_b = dir
                .path()
                .join(format!("b.{shell_bin}", shell_bin = shell_bin(shell)));
            std::fs::write(&path_a, export(&theme_a)).expect("write a");
            std::fs::write(&path_b, export(&theme_b)).expect("write b");

            let only_a = sourced_vars(shell, &[&path_a]);
            assert!(
                only_a.contains(&"__time_format=24".to_owned()),
                "{shell:?}: export A must set the 24h clock, got {only_a:?}"
            );
            let fresh = sourced_vars(shell, &[&path_b]);
            let reloaded = sourced_vars(shell, &[&path_a, &path_b]);
            assert_eq!(reloaded, fresh, "{shell:?}: A-then-B must equal fresh B");
            assert!(
                reloaded.contains(&"__time_format=12".to_owned()),
                "{shell:?}: B must reset the clock to 12h, got {reloaded:?}"
            );
        }
    }
}
