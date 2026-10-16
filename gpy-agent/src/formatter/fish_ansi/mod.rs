//! Template-rendered ANSI prompt formatter. Segments without a `format` (or
//! whose template errors) emit nothing; [`PromptDialect`] selects text escaping.

use crate::Result;
use crate::config::Config;
use crate::formatter::separator::Glyphs;
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
            Response::Hostname { hostname, is_ssh } => {
                Ok(render_hostname_segment(hostname, *is_ssh, ctx, dialect))
            }
            Response::Username { username } => Ok(render_username_segment(username, ctx, dialect)),
            // The reply is printed verbatim into the prompt, so protocol JSON must never reach it.
            Response::Error { message } => {
                crate::debug_log!("formatter", "omitting segment after error: {message}");
                Ok(String::new())
            }
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

/// Render `resolver` with `format`, inheriting `ctx`'s prev colors and palette, and encode.
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

/// Downgrade a template error to an empty segment (never break the live prompt),
/// surfacing it via [`crate::debug::warn_fallback`] for `gpy doctor`.
fn render_or_warn(component: &str, result: crate::template::Result<String>) -> String {
    match result {
        Ok(rendered) => rendered,
        Err(error) => {
            crate::debug::warn_fallback(component, "emitting empty segment", &error);
            String::new()
        }
    }
}

/// Render the git segment; no format or a template error yields nothing.
fn render_git_segment(
    status: &RepositoryStatus,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
) -> String {
    let Some(format) = ctx.theme.segments.git.format.as_deref() else {
        return String::new();
    };
    render_or_warn(
        "git segment",
        render_git_via_template(status, ctx, dialect, format),
    )
}

/// Render the git segment through the template engine.
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

/// Render the language segment; no format or a template error yields nothing.
fn render_language_segment(
    languages: &[LanguageInfo],
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
) -> String {
    let Some(format) = ctx.theme.segments.language.format.as_deref() else {
        return String::new();
    };
    render_or_warn(
        "language segment",
        render_languages_via_template(languages, ctx, dialect, format),
    )
}

/// Select up to three languages to show. With `show_versions`, versionless ones
/// are dropped unless `show_symbol_without_version`. Shared with the wizard preview.
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

/// Render each selected language independently and concatenate.
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
        // Only the first pill inherits `is_first`, only the last `is_last`.
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

/// Render the directory; without a format the shell renders it locally.
fn render_directory_segment(
    cwd: &str,
    read_only: bool,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
) -> String {
    let Some(format) = ctx.theme.segments.directory.format.as_deref() else {
        return String::new();
    };
    render_or_warn(
        "directory segment",
        render_directory_via_template(cwd, read_only, ctx, dialect, format),
    )
}

/// Render the directory segment through the template engine.
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

/// Render the clock; without a format the shell renders it locally.
fn render_clock_segment(
    shell: crate::shell::Shell,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
) -> String {
    let Some(format) = ctx.theme.segments.clock.format.as_deref() else {
        return String::new();
    };
    render_or_warn(
        "clock segment",
        render_clock_via_template(shell, ctx, dialect, format),
    )
}

/// Render the clock segment through the template engine.
fn render_clock_via_template(
    shell: crate::shell::Shell,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
    format: &str,
) -> crate::template::Result<String> {
    use crate::formatter::clock_resolver::ClockResolver;

    let resolver = ClockResolver::new(shell, ctx.theme, ctx.position)
        .with_glyphs(Glyphs::from(&ctx.config.ui));
    render_via_template(&resolver, ctx, dialect, format)
}

/// Render duration; without a format the shell renders it locally.
fn render_duration_segment(
    duration_ms: u64,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
) -> String {
    let Some(format) = ctx.theme.segments.duration.format.as_deref() else {
        return String::new();
    };
    render_or_warn(
        "duration segment",
        render_duration_via_template(duration_ms, ctx, dialect, format),
    )
}

/// Render the duration segment through the template engine.
fn render_duration_via_template(
    duration_ms: u64,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
    format: &str,
) -> crate::template::Result<String> {
    use crate::formatter::duration_resolver::DurationResolver;

    let resolver = DurationResolver::new(duration_ms, ctx.theme, ctx.position)
        .with_glyphs(Glyphs::from(&ctx.config.ui));
    render_via_template(&resolver, ctx, dialect, format)
}

/// Render the character; without a format the shell renders it locally.
fn render_character_segment(
    success: bool,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
) -> String {
    let Some(format) = ctx.theme.segments.character.format.as_deref() else {
        return String::new();
    };
    render_or_warn(
        "character segment",
        render_character_via_template(success, ctx, dialect, format),
    )
}

/// Render the character segment through the template engine.
fn render_character_via_template(
    success: bool,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
    format: &str,
) -> crate::template::Result<String> {
    use crate::formatter::character_resolver::CharacterResolver;

    let resolver = CharacterResolver::new(success, ctx.theme, ctx.position)
        .with_glyphs(Glyphs::from(&ctx.config.ui));
    render_via_template(&resolver, ctx, dialect, format)
}

/// Render the hostname; without a format the shell renders it locally.
fn render_hostname_segment(
    hostname: &str,
    is_ssh: bool,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
) -> String {
    let Some(format) = ctx.theme.segments.hostname.format.as_deref() else {
        return String::new();
    };
    render_or_warn(
        "hostname segment",
        render_hostname_via_template(hostname, is_ssh, ctx, dialect, format),
    )
}

/// Render the hostname segment through the template engine.
fn render_hostname_via_template(
    hostname: &str,
    is_ssh: bool,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
    format: &str,
) -> crate::template::Result<String> {
    use crate::formatter::hostname_resolver::HostnameResolver;

    let resolver = HostnameResolver::new(hostname.to_owned(), is_ssh, ctx.theme, ctx.position)
        .with_glyphs(Glyphs::from(&ctx.config.ui));
    render_via_template(&resolver, ctx, dialect, format)
}

/// Render the username; without a format the shell renders it locally.
fn render_username_segment(
    username: &str,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
) -> String {
    let Some(format) = ctx.theme.segments.username.format.as_deref() else {
        return String::new();
    };
    render_or_warn(
        "username segment",
        render_username_via_template(username, ctx, dialect, format),
    )
}

/// Render the username segment through the template engine.
fn render_username_via_template(
    username: &str,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
    format: &str,
) -> crate::template::Result<String> {
    use crate::formatter::username_resolver::UsernameResolver;

    let resolver = UsernameResolver::new(username.to_owned(), ctx.theme, ctx.position)
        .with_glyphs(Glyphs::from(&ctx.config.ui));
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
mod tests;
