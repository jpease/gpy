//! Template-rendered ANSI prompt formatter
//!
//! Renders agent responses as pre-formatted ANSI escape sequences via the
//! template engine. Each segment is rendered from its `format` template; when a
//! segment has no `format` (or its template errors) the agent emits nothing.
//! The formatter's [`PromptDialect`] selects how span text is escaped: verbatim
//! for Fish (`ansi`), or for bash/zsh prompt expansion (`bash-prompt`,
//! `zsh-prompt`, #677).

use crate::Result;
use crate::config::Config;
use crate::formatter::{Formatter, PromptDialect, RenderContext};
use crate::git::RepositoryStatus;
use crate::ipc::{LanguageInfo, Response, protocol};
use crate::theme::ThemeConfig;

/// Format responses as ANSI-rendered prompt segments in one [`PromptDialect`].
#[derive(Debug, Clone, Copy, Default)]
pub struct FishAnsiFormatter {
    dialect: PromptDialect,
}

impl FishAnsiFormatter {
    /// A formatter that encodes segment text for `dialect`.
    #[must_use]
    pub const fn new(dialect: PromptDialect) -> Self {
        Self { dialect }
    }
}

impl Formatter for FishAnsiFormatter {
    fn render(&self, response: &Response, ctx: &RenderContext<'_>) -> Result<String> {
        let dialect = self.dialect;
        match response {
            Response::RepositoryStatus(repo) => Ok(render_git_segment(repo, ctx, dialect)),
            Response::Language { languages } => {
                Ok(render_language_segment(languages, ctx, dialect))
            }
            Response::Directory { cwd, read_only } => {
                Ok(render_directory_segment(cwd, *read_only, ctx, dialect))
            }
            Response::Clock { shell } => Ok(render_clock_segment(*shell, ctx, dialect)),
            Response::Duration { duration_ms } => {
                Ok(render_duration_segment(*duration_ms, ctx, dialect))
            }
            Response::Character { success } => Ok(render_character_segment(*success, ctx, dialect)),
            Response::Hostname { hostname } => Ok(render_hostname_segment(hostname, ctx, dialect)),
            Response::Username { username } => Ok(render_username_segment(username, ctx, dialect)),
            // A failed or disabled request omits the segment: the reply is
            // printed verbatim into the prompt, so protocol JSON must never
            // reach it (#680). JSON clients get the error from `JsonFormatter`.
            Response::Error { message } => {
                crate::debug_log!("formatter", "omitting segment after error: {message}");
                Ok(String::new())
            }
            // For other response types, fall back to the protocol serializer
            _ => protocol::serialize_response(response)
                .map_err(|e| crate::Error::ipc(format!("Serialization error: {e}")))
                .and_then(|bytes| {
                    String::from_utf8(bytes).map_err(|e| {
                        crate::Error::ipc(format!("UTF-8 error serializing response: {e}"))
                    })
                }),
        }
    }
}

/// Render `resolver` through the template engine and encode it for `dialect`.
///
/// This is the single shared tail for every `render_X_via_template` function:
/// build the template context (inheriting `ctx`'s previous colors and
/// palette), render `format` against `resolver`, and encode the result.
///
/// # Errors
///
/// Returns a [`crate::template::TemplateError`] when `format` fails to parse or evaluate.
fn render_via_template(
    resolver: &dyn crate::template::VariableResolver,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
    format: &str,
) -> crate::template::Result<String> {
    use crate::template::{RenderContext as TemplateContext, render};

    let template_ctx = TemplateContext::new(resolver)
        .with_prev_colors(ctx.prev_fg.clone(), ctx.prev_bg.clone())
        .with_palette(ctx.palette.clone());
    let spans = render(format, &template_ctx)?;
    Ok(dialect.encode(&spans))
}

/// Resolve a segment's template result to a string, warning on error.
///
/// A broken template must never break or spam the live prompt, so any error
/// is downgraded to an empty segment — but the failure is still surfaced via
/// [`crate::debug::warn_fallback`] so `gpy doctor` / `gpy theme check` can
/// pick it up. `component` should be a short human-readable segment name
/// (e.g. `"directory segment"`), matching the style already used by the git
/// and language segments.
fn render_or_warn(component: &str, result: crate::template::Result<String>) -> String {
    match result {
        Ok(rendered) => rendered,
        Err(error) => {
            crate::debug::warn_fallback(component, "emitting empty segment", &error);
            String::new()
        }
    }
}

/// Render the git segment via the template engine. No format (or a template
/// error) yields an empty segment — the legacy color path no longer exists.
fn render_git_segment(
    status: &RepositoryStatus,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
) -> String {
    let Some(format) = ctx.theme.segments.git.format.as_deref() else {
        // No format configured: nothing to render (legacy color path removed).
        return String::new();
    };
    render_or_warn(
        "git segment",
        render_git_via_template(status, ctx, dialect, format),
    )
}

/// Render the git segment through the template engine + ANSI encoder.
///
/// # Errors
///
/// Returns a [`crate::template::TemplateError`] when `format` fails to parse or evaluate.
fn render_git_via_template(
    status: &RepositoryStatus,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
    format: &str,
) -> crate::template::Result<String> {
    use crate::formatter::git_resolver::GitResolver;

    let state = crate::theme::GitState::from_status(status);
    let resolver = GitResolver::new(status, ctx.config, ctx.theme, state, ctx.position);
    render_via_template(&resolver, ctx, dialect, format)
}

/// Render the language segment via the template engine. No format (or a
/// template error) yields an empty segment — the legacy color path no longer
/// exists.
fn render_language_segment(
    languages: &[LanguageInfo],
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
) -> String {
    let Some(format) = ctx.theme.segments.language.format.as_deref() else {
        // No format configured: nothing to render (legacy color path removed).
        return String::new();
    };
    render_or_warn(
        "language segment",
        render_languages_via_template(languages, ctx, dialect, format),
    )
}

/// Select the languages the prompt shows, capped at three (the "up to 3"
/// behavior the template path renders).
///
/// When `show_versions` is on, versionless languages are normally dropped — but
/// the active theme can opt into Starship's behavior of always showing the
/// symbol via `[segments.language].show_symbol_without_version = true`, in which
/// case versionless languages are kept.
///
/// `pub(crate)` (not private) so the wizard's live preview
/// (`commands::wizard::preview::render_segment`'s `"language"` arm) can apply
/// the exact same selection logic the real prompt uses — see that call site
/// for why: without it, toggling `language.show_versions` in the wizard
/// wouldn't visibly change the preview.
#[expect(
    clippy::redundant_pub_crate,
    reason = "fish_ansi is a private submodule of formatter, but formatter::mod.rs re-exports this function crate-wide (`pub(crate) use fish_ansi::select_languages`) for the wizard preview to call; pub(crate) here is what that re-export needs, plain-private fails to compile (E0603) since the re-export sits in the parent module"
)]
pub(crate) fn select_languages<'a>(
    config: &Config,
    theme: &ThemeConfig,
    languages: &'a [LanguageInfo],
) -> Vec<&'a LanguageInfo> {
    let show_symbol_without_version = theme
        .segments
        .language
        .show_symbol_without_version
        .unwrap_or(false);
    if config.language.show_versions && !show_symbol_without_version {
        languages
            .iter()
            .filter(|lang| lang.version.is_some())
            .take(3)
            .collect()
    } else {
        languages.iter().take(3).collect()
    }
}

/// Render the language segment through the template engine + ANSI encoder.
///
/// Each selected language is rendered independently (the `format` owns its
/// presentation, Starship-style) and the results are concatenated, preserving
/// the "up to 3" selection.
///
/// # Errors
///
/// Returns a [`crate::template::TemplateError`] when `format` fails to parse or evaluate.
fn render_languages_via_template(
    languages: &[LanguageInfo],
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
    format: &str,
) -> crate::template::Result<String> {
    use crate::formatter::language_resolver::LanguageResolver;
    use crate::formatter::{IsFirst, IsLast, SegmentPosition};

    let selected = select_languages(ctx.config, ctx.theme, languages);
    let last_index = selected.len().saturating_sub(1);
    let mut output = String::with_capacity(160);
    for (index, lang) in selected.into_iter().enumerate() {
        // Only the first pill inherits the segment's `is_first`, only the last
        // its `is_last`; the rest chain as middle pills (#751).
        let pill_position = SegmentPosition::new(
            if index == last_index {
                ctx.position.is_last
            } else {
                IsLast::No
            },
            if index == 0 {
                ctx.position.is_first
            } else {
                IsFirst::No
            },
        );
        let resolver = LanguageResolver::new(lang, ctx.config, ctx.theme, pill_position);
        output.push_str(&render_via_template(&resolver, ctx, dialect, format)?);
    }
    Ok(output)
}

/// Entry point: template path when a `format` is set and renders cleanly,
/// otherwise the agent emits nothing (shell renders directory locally).
fn render_directory_segment(
    cwd: &str,
    read_only: bool,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
) -> String {
    let Some(format) = ctx.theme.segments.directory.format.as_deref() else {
        // No format set: shell renders directory locally; agent emits nothing.
        return String::new();
    };
    render_or_warn(
        "directory segment",
        render_directory_via_template(cwd, read_only, ctx, dialect, format),
    )
}

/// Render the directory segment through the template engine + ANSI encoder.
///
/// # Errors
///
/// Returns a [`crate::template::TemplateError`] when `format` fails to parse or evaluate.
fn render_directory_via_template(
    cwd: &str,
    read_only: bool,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
    format: &str,
) -> crate::template::Result<String> {
    use crate::formatter::directory_resolver::DirectoryResolver;

    let resolver = DirectoryResolver::new(cwd, read_only, ctx.config, ctx.theme, ctx.position);
    render_via_template(&resolver, ctx, dialect, format)
}

/// Entry point: render the clock via the template engine when a `format` is set.
///
/// When no format template is configured the agent emits nothing and the shell
/// renders the clock locally — which is what Fish always does, and what Zsh and
/// Bash did for every theme before this segment gained a template.
fn render_clock_segment(
    shell: crate::shell::Shell,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
) -> String {
    let Some(format) = ctx.theme.segments.clock.format.as_deref() else {
        // No format set: shell renders the clock locally; agent emits nothing.
        return String::new();
    };
    render_or_warn(
        "clock segment",
        render_clock_via_template(shell, ctx, dialect, format),
    )
}

/// Render the clock segment through the template engine + ANSI encoder.
///
/// # Errors
///
/// Returns a [`crate::template::TemplateError`] when `format` fails to parse or evaluate.
fn render_clock_via_template(
    shell: crate::shell::Shell,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
    format: &str,
) -> crate::template::Result<String> {
    use crate::formatter::clock_resolver::ClockResolver;

    let resolver = ClockResolver::new(shell, ctx.theme, ctx.position);
    render_via_template(&resolver, ctx, dialect, format)
}

/// Entry point: render via template engine when a `format` is set.
///
/// When no format template is configured the agent emits nothing and the
/// shell renders the duration segment locally (byte-identical to pre-#192
/// behavior). A malformed template also falls back to empty.
fn render_duration_segment(
    duration_ms: u64,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
) -> String {
    let Some(format) = ctx.theme.segments.duration.format.as_deref() else {
        // No format set: shell renders duration locally; agent emits nothing.
        return String::new();
    };
    render_or_warn(
        "duration segment",
        render_duration_via_template(duration_ms, ctx, dialect, format),
    )
}

/// Render the duration segment through the template engine + ANSI encoder.
///
/// # Errors
///
/// Returns a [`crate::template::TemplateError`] when `format` fails to parse or evaluate.
fn render_duration_via_template(
    duration_ms: u64,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
    format: &str,
) -> crate::template::Result<String> {
    use crate::formatter::duration_resolver::DurationResolver;

    let resolver = DurationResolver::new(duration_ms, ctx.theme, ctx.position);
    render_via_template(&resolver, ctx, dialect, format)
}

/// Entry point: render via template engine when a `format` is set.
///
/// When no format template is configured the agent emits nothing and the
/// shell renders the character segment locally (byte-identical to pre-#193
/// behavior). A malformed template also falls back to empty.
fn render_character_segment(
    success: bool,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
) -> String {
    let Some(format) = ctx.theme.segments.character.format.as_deref() else {
        // No format set: shell renders character locally; agent emits nothing.
        return String::new();
    };
    render_or_warn(
        "character segment",
        render_character_via_template(success, ctx, dialect, format),
    )
}

/// Render the character segment through the template engine + ANSI encoder.
///
/// # Errors
///
/// Returns a [`crate::template::TemplateError`] when `format` fails to parse or evaluate.
fn render_character_via_template(
    success: bool,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
    format: &str,
) -> crate::template::Result<String> {
    use crate::formatter::character_resolver::CharacterResolver;

    let resolver = CharacterResolver::new(success, ctx.theme, ctx.position);
    render_via_template(&resolver, ctx, dialect, format)
}

/// Entry point: render via template engine when a `format` is set.
///
/// When no format template is configured the agent emits nothing and the
/// shell renders the hostname segment locally. A malformed template also
/// falls back to empty.
fn render_hostname_segment(
    hostname: &str,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
) -> String {
    let Some(format) = ctx.theme.segments.hostname.format.as_deref() else {
        // No format set: shell renders hostname locally; agent emits nothing.
        return String::new();
    };
    render_or_warn(
        "hostname segment",
        render_hostname_via_template(hostname, ctx, dialect, format),
    )
}

/// Render the hostname segment through the template engine + ANSI encoder.
///
/// # Errors
///
/// Returns a [`crate::template::TemplateError`] when `format` fails to parse or evaluate.
fn render_hostname_via_template(
    hostname: &str,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
    format: &str,
) -> crate::template::Result<String> {
    use crate::formatter::hostname_resolver::HostnameResolver;

    let resolver = HostnameResolver::new(hostname.to_owned(), ctx.theme, ctx.position);
    render_via_template(&resolver, ctx, dialect, format)
}

/// Entry point: render the username segment via the template engine when a
/// `format` is set.
///
/// When no format template is configured the agent emits nothing and the shell
/// renders the username segment locally (pure-shell pill path). A malformed
/// template also falls back to empty.
fn render_username_segment(
    username: &str,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
) -> String {
    let Some(format) = ctx.theme.segments.username.format.as_deref() else {
        // No format set: shell renders username locally; agent emits nothing.
        return String::new();
    };
    render_or_warn(
        "username segment",
        render_username_via_template(username, ctx, dialect, format),
    )
}

/// Render the username segment through the template engine + ANSI encoder.
///
/// # Errors
///
/// Returns a [`crate::template::TemplateError`] when `format` fails to parse or evaluate.
fn render_username_via_template(
    username: &str,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
    format: &str,
) -> crate::template::Result<String> {
    use crate::formatter::username_resolver::UsernameResolver;

    let resolver = UsernameResolver::new(username.to_owned(), ctx.theme, ctx.position);
    render_via_template(&resolver, ctx, dialect, format)
}

/// Resolve the display text for a language (icon or name) used by resolvers.
pub fn get_language_display(config: &Config, language: &str) -> String {
    if !config.ui.show_icons {
        return language.to_owned();
    }
    if config.language.display == crate::config::types::LanguageDisplay::Icon {
        let icon_key = crate::language::metadata::get_icon_key(language).unwrap_or(language);

        config
            .language
            .icons
            .icons
            .get(icon_key)
            .map_or_else(|| language.to_owned(), std::string::ToString::to_string)
    } else {
        language.to_owned()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]

    use super::*;
    use crate::formatter::SegmentPosition;
    use crate::theme::ThemeConfig;

    fn ctx() -> (Config, ThemeConfig) {
        (Config::default(), ThemeConfig::default())
    }

    #[test]
    fn git_without_format_emits_empty() {
        // Post-migration contract: no format → agent emits nothing (no legacy path).
        let formatter = FishAnsiFormatter::default();
        let (config, theme) = ctx();
        assert!(theme.segments.git.format.is_none());
        let rc = RenderContext::new(&config, &theme, SegmentPosition::LAST);
        let status = crate::git::RepositoryStatus {
            branch: "main".to_owned(),
            ahead: 0,
            behind: 0,
            ahead_capped: false,
            behind_capped: false,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            conflicts: 0,
            state: crate::git::RepositoryState::Clean,
            stash_count: 0,
            detached: false,
            rebase_progress: None,
        };
        let response = Response::RepositoryStatus(status);
        let got = formatter.render(&response, &rc).expect("render");
        assert_eq!(got, "", "no format → empty (legacy path removed)");
    }

    #[test]
    fn git_malformed_format_emits_empty() {
        let formatter = FishAnsiFormatter::default();
        let (config, mut theme) = ctx();
        theme.segments.git.format = Some("on [$branch(green)".to_owned()); // unbalanced
        let rc = RenderContext::new(&config, &theme, SegmentPosition::LAST);
        let status = crate::git::RepositoryStatus {
            branch: "main".to_owned(),
            ahead: 0,
            behind: 0,
            ahead_capped: false,
            behind_capped: false,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            conflicts: 0,
            state: crate::git::RepositoryState::Clean,
            stash_count: 0,
            detached: false,
            rebase_progress: None,
        };
        let response = Response::RepositoryStatus(status);
        let got = formatter.render(&response, &rc).expect("render");
        assert_eq!(
            got, "",
            "broken format → silent empty (legacy path removed)"
        );
    }

    #[test]
    fn format_some_renders_via_engine() {
        let formatter = FishAnsiFormatter::default();
        let (config, mut theme) = ctx();
        theme.segments.git.format = Some("on [$branch](bold green)".to_owned());
        let rc = RenderContext::new(&config, &theme, SegmentPosition::LAST);
        let status = crate::git::RepositoryStatus {
            branch: "main".to_owned(),
            ahead: 0,
            behind: 0,
            ahead_capped: false,
            behind_capped: false,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            conflicts: 0,
            state: crate::git::RepositoryState::Clean,
            stash_count: 0,
            detached: false,
            rebase_progress: None,
        };
        let response = Response::RepositoryStatus(status);
        let got = formatter.render(&response, &rc).expect("render");
        // "on " is literal; "main" is bold green; one trailing reset.
        assert_eq!(got, "on \x1b[1;32mmain\x1b[0m");
    }

    #[test]
    fn style_indirection_renders_via_engine() {
        let formatter = FishAnsiFormatter::default();
        let (config, mut theme) = ctx();
        theme.segments.git.format = Some("[$branch]($style)".to_owned());
        let rc = RenderContext::new(&config, &theme, SegmentPosition::LAST);
        let status = crate::git::RepositoryStatus {
            branch: "main".to_owned(),
            ahead: 0,
            behind: 0,
            ahead_capped: false,
            behind_capped: false,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            conflicts: 0,
            state: crate::git::RepositoryState::Clean,
            stash_count: 0,
            detached: false,
            rebase_progress: None,
        };
        let response = Response::RepositoryStatus(status);
        let got = formatter.render(&response, &rc).expect("render");
        assert!(got.contains("main"), "expected branch name, got {got:?}");
        assert!(
            got.contains('\u{1b}'),
            "expected an ANSI escape, got {got:?}"
        );
    }

    fn rust_response() -> Response {
        Response::Language {
            languages: vec![LanguageInfo {
                name: "Rust".to_owned(),
                version: Some("1.75.0".to_owned()),
                color: crate::config::types::ColorSpec::new("#dea584").unwrap(),
            }],
        }
    }

    #[test]
    fn language_without_format_emits_empty() {
        // Post-migration contract: no format → agent emits nothing (no legacy path).
        let formatter = FishAnsiFormatter::default();
        let (config, theme) = ctx();
        assert!(theme.segments.language.format.is_none());
        let rc = RenderContext::new(&config, &theme, SegmentPosition::LAST);
        let got = formatter.render(&rust_response(), &rc).expect("render");
        assert_eq!(got, "", "no format → empty (legacy path removed)");
    }

    #[test]
    fn language_malformed_format_emits_empty() {
        let formatter = FishAnsiFormatter::default();
        let (config, mut theme) = ctx();
        theme.segments.language.format = Some("[$symbol(green)".to_owned()); // unbalanced
        let rc = RenderContext::new(&config, &theme, SegmentPosition::LAST);
        let got = formatter.render(&rust_response(), &rc).expect("render");
        assert_eq!(
            got, "",
            "broken format → silent empty (legacy path removed)"
        );
    }

    #[test]
    fn language_format_some_renders_via_engine() {
        let formatter = FishAnsiFormatter::default();
        let (config, mut theme) = ctx();
        theme.segments.language.format = Some("[$symbol $version](bold green)".to_owned());
        let rc = RenderContext::new(&config, &theme, SegmentPosition::LAST);
        let got = formatter.render(&rust_response(), &rc).expect("render");
        // The version is literal; bold green wraps the styled group; one trailing reset.
        assert!(got.contains("1.75.0"), "expected version, got {got:?}");
        assert!(
            got.contains("\x1b[1;32m"),
            "expected bold green, got {got:?}"
        );
        assert!(
            got.ends_with("\x1b[0m"),
            "expected trailing reset, got {got:?}"
        );
    }

    #[test]
    fn language_format_caps_at_three() {
        let formatter = FishAnsiFormatter::default();
        let (config, mut theme) = ctx();
        theme.segments.language.format = Some("[$version ](green)".to_owned());
        let rc = RenderContext::new(&config, &theme, SegmentPosition::LAST);
        let mk = |name: &str, ver: &str| LanguageInfo {
            name: name.to_owned(),
            version: Some(ver.to_owned()),
            color: crate::config::types::ColorSpec::new("#dea584").unwrap(),
        };
        let response = Response::Language {
            languages: vec![
                mk("Rust", "1.0.0"),
                mk("Go", "2.0.0"),
                mk("Python", "3.0.0"),
                mk("Node", "4.0.0"),
            ],
        };
        let got = formatter.render(&response, &rc).expect("render");
        assert!(got.contains("1.0.0") && got.contains("3.0.0"));
        assert!(
            !got.contains("4.0.0"),
            "fourth language must be dropped: {got:?}"
        );
    }

    /// #751: each language pill takes its own position — only the first pill
    /// inherits the segment's `is_first`, only the last its `is_last`.
    #[test]
    fn language_pills_chain_positions_within_segment() {
        let formatter = FishAnsiFormatter::default();
        let (config, mut theme) = ctx();
        theme.segments.language.format = Some(
            "([$sep_open](fg:$bg bg:default))[ $symbol]($style)([ $version]($style))([$sep_gap]($style))([$sep_close](fg:$bg bg:default))"
                .to_owned(),
        );
        let mk = |name: &str, ver: &str| LanguageInfo {
            name: name.to_owned(),
            version: Some(ver.to_owned()),
            color: crate::config::types::ColorSpec::new("#dea584").unwrap(),
        };
        let response = Response::Language {
            languages: vec![
                mk("Rust", "1.0.0"),
                mk("Go", "2.0.0"),
                mk("Python", "3.0.0"),
            ],
        };
        let counts = |pos: SegmentPosition| {
            let rc = RenderContext::new(&config, &theme, pos);
            let got = formatter.render(&response, &rc).expect("render");
            (
                got.matches('\u{e0ba}').count(),
                got.matches('\u{e0bc}').count(),
                got.matches('\u{e0b4}').count(),
            )
        };
        assert_eq!(counts(SegmentPosition::FIRST), (2, 3, 0), "first");
        assert_eq!(counts(SegmentPosition::MIDDLE), (3, 3, 0), "middle");
        assert_eq!(counts(SegmentPosition::LAST), (3, 2, 1), "last");
        assert_eq!(counts(SegmentPosition::ONLY), (2, 2, 1), "only");
    }

    #[test]
    fn directory_format_none_emits_empty() {
        let formatter = FishAnsiFormatter::default();
        let (config, theme) = ctx();
        assert!(theme.segments.directory.format.is_none());
        let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
        let response = Response::Directory {
            cwd: "/home/user/project".to_owned(),
            read_only: false,
        };
        let got = formatter.render(&response, &rc).expect("render");
        assert_eq!(got, "", "no format → agent emits nothing");
    }

    /// #680: an error reply renders as an empty segment in every prompt
    /// dialect instead of leaking protocol JSON into the prompt.
    #[test]
    fn error_response_renders_empty() {
        let (config, theme) = ctx();
        let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
        let response = Response::Error {
            message: "x".into(),
        };
        for dialect in PromptDialect::ALL {
            let got = FishAnsiFormatter::new(dialect)
                .render(&response, &rc)
                .expect("render");
            assert_eq!(got, "", "{dialect:?}: error must render empty");
        }
    }

    /// #677: the clock's live-time token reaches each shell unescaped, so the
    /// shell still expands it on every draw, while the theme literal around
    /// it is escaped like any other text.
    #[test]
    fn clock_prompt_token_is_left_for_the_shell_to_expand() {
        use crate::shell::Shell;
        let (config, mut theme) = ctx();
        theme.segments.clock.format = Some(r"[\$ $time](fg:white)".to_owned());
        let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

        let bash = FishAnsiFormatter::new(PromptDialect::BashPrompt)
            .render(&Response::Clock { shell: Shell::Bash }, &rc)
            .expect("render bash");
        // Each span re-emits its SGR (inside the zero-width markers, #679),
        // so check the literal and the token apart. The token is visible
        // text and sits outside the markers.
        assert!(bash.contains("\\\\$ \\["), "{bash:?}");
        assert!(bash.contains("\\]\\D{%-I:%M %p}\\["), "{bash:?}");

        let zsh = FishAnsiFormatter::new(PromptDialect::ZshPrompt)
            .render(&Response::Clock { shell: Shell::Zsh }, &rc)
            .expect("render zsh");
        assert!(zsh.contains("\\$ %{"), "{zsh:?}");
        assert!(zsh.contains("%}%D{%-I:%M %p}%{"), "{zsh:?}");
    }

    /// #677: directory data and theme literals are escaped for the
    /// requested shell; the `ansi` format stays verbatim for fish.
    #[test]
    fn directory_text_is_escaped_per_dialect() {
        let (config, mut theme) = ctx();
        theme.segments.directory.format = Some("[$path 100%](fg:green)".to_owned());
        let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
        let response = Response::Directory {
            cwd: "/w/$(echo X)".to_owned(),
            read_only: false,
        };
        let render = |dialect| {
            FishAnsiFormatter::new(dialect)
                .render(&response, &rc)
                .expect("render")
        };
        let ansi = render(PromptDialect::Ansi);
        assert!(
            ansi.contains("m$(echo X)") && ansi.contains(" 100%"),
            "{ansi:?}"
        );
        let bash = render(PromptDialect::BashPrompt);
        assert!(
            bash.contains("\\]\\\\$(echo X)") && bash.contains(" 100%"),
            "{bash:?}"
        );
        let zsh = render(PromptDialect::ZshPrompt);
        assert!(
            zsh.contains("%}\\$(echo X)") && zsh.contains(" 100%%"),
            "{zsh:?}"
        );
    }

    #[test]
    fn directory_format_some_renders_via_engine() {
        let formatter = FishAnsiFormatter::default();
        let (config, mut theme) = ctx();
        theme.segments.directory.format = Some("[$path](bold green)".to_owned());
        let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
        let response = Response::Directory {
            cwd: "/home/user/project".to_owned(),
            read_only: false,
        };
        let got = formatter.render(&response, &rc).expect("render");
        // Contains the path and at least one ANSI escape
        assert!(
            got.contains("project") || got.contains("/home/user/project"),
            "got {got:?}"
        );
        assert!(got.contains('\x1b'), "expected ANSI escape, got {got:?}");
        assert!(
            got.ends_with("\x1b[0m"),
            "expected trailing reset, got {got:?}"
        );
    }

    #[test]
    fn directory_malformed_format_emits_empty() {
        let formatter = FishAnsiFormatter::default();
        let (config, mut theme) = ctx();
        theme.segments.directory.format = Some("[$path(green)".to_owned()); // unbalanced '['
        let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
        let response = Response::Directory {
            cwd: "/tmp".to_owned(),
            read_only: false,
        };
        let got = formatter.render(&response, &rc).expect("render");
        assert_eq!(got, "", "broken format → silent fallback → empty");
    }

    #[test]
    fn duration_format_none_emits_empty() {
        let formatter = FishAnsiFormatter::default();
        let (config, theme) = ctx();
        assert!(theme.segments.duration.format.is_none());
        let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
        let response = Response::Duration {
            duration_ms: 2500_u64,
        };
        let got = formatter.render(&response, &rc).expect("render");
        assert_eq!(got, "", "no format → agent emits nothing");
    }

    #[test]
    fn duration_format_some_renders_via_engine() {
        let formatter = FishAnsiFormatter::default();
        let (config, mut theme) = ctx();
        theme.segments.duration.format = Some("[$duration]($style)".to_owned());
        let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
        let response = Response::Duration {
            duration_ms: 2500_u64,
        };
        let got = formatter.render(&response, &rc).expect("render");
        // Contains the human-formatted time and at least one ANSI escape
        assert!(got.contains("2.500s"), "expected '2.500s' in {got:?}");
        assert!(got.contains('\x1b'), "expected ANSI escape, got {got:?}");
        assert!(
            got.ends_with("\x1b[0m"),
            "expected trailing reset, got {got:?}"
        );
    }

    #[test]
    fn duration_malformed_format_emits_empty() {
        let formatter = FishAnsiFormatter::default();
        let (config, mut theme) = ctx();
        theme.segments.duration.format = Some("[$duration(green)".to_owned()); // unbalanced '['
        let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
        let response = Response::Duration {
            duration_ms: 1000_u64,
        };
        let got = formatter.render(&response, &rc).expect("render");
        assert_eq!(got, "", "broken format → silent fallback → empty");
    }

    #[test]
    fn character_format_none_emits_empty() {
        let formatter = FishAnsiFormatter::default();
        let (config, theme) = ctx();
        assert!(theme.segments.character.format.is_none());
        let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
        let response = Response::Character { success: true };
        let got = formatter.render(&response, &rc).expect("render");
        assert_eq!(got, "", "no format → agent emits nothing");
    }

    #[test]
    fn character_format_success_renders_symbol_with_ansi() {
        let formatter = FishAnsiFormatter::default();
        let (config, mut theme) = ctx();
        theme.segments.character.format = Some("[$symbol]($style)".to_owned());
        let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
        let response = Response::Character { success: true };
        let got = formatter.render(&response, &rc).expect("render");
        // Default success symbol is ❯ with green color
        assert!(got.contains('❯'), "expected ❯ in {got:?}");
        assert!(got.contains('\x1b'), "expected ANSI escape, got {got:?}");
        // Green foreground expected
        assert!(
            got.contains("\x1b[32m") || got.contains("32"),
            "expected green color, got {got:?}"
        );
        assert!(
            got.ends_with("\x1b[0m"),
            "expected trailing reset, got {got:?}"
        );
    }

    #[test]
    fn character_format_error_renders_red() {
        let formatter = FishAnsiFormatter::default();
        let (config, mut theme) = ctx();
        theme.segments.character.format = Some("[$symbol]($style)".to_owned());
        let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
        let response = Response::Character { success: false };
        let got = formatter.render(&response, &rc).expect("render");
        // Default error symbol is ❯ with red color
        assert!(got.contains('❯'), "expected ❯ in {got:?}");
        // Red foreground expected
        assert!(
            got.contains("\x1b[31m") || got.contains("31"),
            "expected red color, got {got:?}"
        );
    }

    #[test]
    fn character_malformed_format_emits_empty() {
        let formatter = FishAnsiFormatter::default();
        let (config, mut theme) = ctx();
        theme.segments.character.format = Some("[$symbol(green)".to_owned()); // unbalanced '['
        let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
        let response = Response::Character { success: true };
        let got = formatter.render(&response, &rc).expect("render");
        assert_eq!(got, "", "broken format → silent fallback → empty");
    }

    #[test]
    fn hostname_format_none_emits_empty() {
        let formatter = FishAnsiFormatter::default();
        let (config, theme) = ctx();
        assert!(theme.segments.hostname.format.is_none());
        let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
        let response = Response::Hostname {
            hostname: "host.example.com".to_owned(),
        };
        let got = formatter.render(&response, &rc).expect("render");
        assert_eq!(got, "", "no format → agent emits nothing");
    }

    #[test]
    fn hostname_format_some_renders_trimmed_name_with_ansi() {
        let formatter = FishAnsiFormatter::default();
        let (config, mut theme) = ctx();
        theme.segments.hostname.format = Some("on [$symbol$hostname](bold green) ".to_owned());
        let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
        let response = Response::Hostname {
            hostname: "host.example.com".to_owned(),
        };
        let got = formatter.render(&response, &rc).expect("render");
        // Default trim_at="." trims the FQDN down to the short name.
        assert!(
            got.contains("host"),
            "expected trimmed hostname, got {got:?}"
        );
        assert!(
            !got.contains("host.example.com"),
            "expected trimming to have applied, got {got:?}"
        );
        assert!(got.contains('\x1b'), "expected ANSI escape, got {got:?}");
        // The reset lands right after the styled group, before the trailing
        // literal " " — matching Starship's reset-at-group-boundary convention
        // (#265) — so the rendered string ends with a plain space, not the
        // reset sequence itself.
        assert_eq!(got, "on \u{1b}[1;32mhost\u{1b}[0m ");
    }

    #[test]
    fn hostname_malformed_format_emits_empty() {
        let formatter = FishAnsiFormatter::default();
        let (config, mut theme) = ctx();
        theme.segments.hostname.format = Some("[$hostname(green)".to_owned()); // unbalanced '['
        let rc = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
        let response = Response::Hostname {
            hostname: "host".to_owned(),
        };
        let got = formatter.render(&response, &rc).expect("render");
        assert_eq!(got, "", "broken format → silent fallback → empty");
    }

    #[test]
    fn git_format_resolves_through_attached_palette() {
        use crate::template::{Color, Palette};
        use std::collections::HashMap;

        let formatter = FishAnsiFormatter::default();
        let (config, mut theme) = ctx();
        // Reference a palette-only color name in the git format.
        theme.segments.git.format = Some("[$branch](fg:accent)".to_owned());
        let mut map = HashMap::new();
        map.insert(
            "accent".to_owned(),
            Color::Rgb {
                r: 10,
                g: 20,
                b: 30,
            },
        );
        let rc = RenderContext::new(&config, &theme, SegmentPosition::LAST)
            .with_palette(Palette::new(map));
        let status = crate::git::RepositoryStatus {
            branch: "main".to_owned(),
            ahead: 0,
            behind: 0,
            ahead_capped: false,
            behind_capped: false,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            conflicts: 0,
            state: crate::git::RepositoryState::Clean,
            stash_count: 0,
            detached: false,
            rebase_progress: None,
        };
        let response = Response::RepositoryStatus(status);
        let got = formatter.render(&response, &rc).expect("render");
        // Palette-resolved fg → truecolor SGR for rgb(10,20,30).
        assert_eq!(got, "\x1b[38;2;10;20;30mmain\x1b[0m");
    }

    fn make_lang(name: &str, version: Option<&str>, color: &str) -> crate::ipc::LanguageInfo {
        crate::ipc::LanguageInfo {
            name: name.to_owned(),
            version: version.map(str::to_owned),
            color: crate::config::types::ColorSpec::new(color).expect("valid color spec"),
        }
    }

    #[test]
    fn select_languages_show_versions_true_filters_versionless() {
        let mut config = Config::default();
        config.language.show_versions = true;
        let theme = ThemeConfig::default();

        let langs = vec![
            make_lang("Rust", Some("1.75.0"), "red"),
            make_lang("Nix", None, "blue"), // no version — filtered out when show_versions=true
            make_lang("Python", Some("3.12.0"), "yellow"),
        ];

        let selected = select_languages(&config, &theme, &langs);
        assert_eq!(selected.len(), 2_usize, "only versioned languages selected");
        assert_eq!(selected.first().map(|l| l.name.as_str()), Some("Rust"));
        assert_eq!(selected.get(1).map(|l| l.name.as_str()), Some("Python"));
    }

    #[test]
    fn select_languages_show_versions_false_includes_all() {
        let mut config = Config::default();
        config.language.show_versions = false;
        let theme = ThemeConfig::default();

        let langs = vec![
            make_lang("Rust", Some("1.75.0"), "red"),
            make_lang("Nix", None, "blue"),
        ];

        let selected = select_languages(&config, &theme, &langs);
        assert_eq!(
            selected.len(),
            2_usize,
            "all languages selected regardless of version"
        );
    }

    #[test]
    fn select_languages_theme_show_symbol_without_version_keeps_versionless() {
        // show_versions on would normally drop versionless languages, but a theme
        // opting into show_symbol_without_version (Starship parity) keeps them.
        let mut config = Config::default();
        config.language.show_versions = true;
        let mut theme = ThemeConfig::default();
        theme.segments.language.show_symbol_without_version = Some(true);

        let langs = vec![
            make_lang("Rust", Some("1.75.0"), "red"),
            make_lang("Fish", None, "green"),
        ];

        let selected = select_languages(&config, &theme, &langs);
        assert_eq!(
            selected.len(),
            2_usize,
            "versionless language kept when theme shows symbol without version"
        );
    }

    /// `render_or_warn` is the shared error-handling path all seven segment
    /// entry points now route through.
    ///
    /// Before this refactor, five of them
    /// (directory/duration/character/hostname/username) discarded template
    /// errors via `if let ... && let Ok(...)` with no `warn_fallback` call at
    /// all, so a malformed template failed invisibly. There is no existing
    /// stderr-capture test precedent in this codebase (`warn_fallback` is a
    /// bare `eprintln!`), so rather than inventing new capture machinery this
    /// test proves the control-flow contract directly: given a synthetic
    /// `Err`, `render_or_warn` takes the warn-and-empty-string branch (as
    /// opposed to, say, propagating the error or panicking). Combined with
    /// the `*_malformed_format_emits_empty` tests above — which now all
    /// route through this same helper — this closes the "silently discarded
    /// error" gap for all five previously-silent segments.
    #[test]
    fn render_or_warn_takes_warn_and_empty_branch_on_error() {
        let error = crate::template::TemplateError::Unbalanced {
            delimiter: '[',
            position: 3,
        };
        let got = render_or_warn("directory segment", Err(error));
        assert_eq!(
            got, "",
            "an error must still yield an empty segment, matching pre-refactor behavior"
        );
    }

    #[test]
    fn render_or_warn_passes_through_ok_value_unchanged() {
        let got = render_or_warn("directory segment", Ok("rendered".to_owned()));
        assert_eq!(got, "rendered", "Ok(value) must pass through unchanged");
    }
}
