//! Template-rendered ANSI prompt formatter. Segments without a `format` (or
//! whose template errors) emit nothing; [`PromptDialect`] selects text escaping.

use crate::Result;
use crate::formatter::character_resolver::CharacterResolver;
use crate::formatter::clock_resolver::ClockResolver;
use crate::formatter::directory_resolver::DirectoryResolver;
use crate::formatter::duration_resolver::DurationResolver;
use crate::formatter::git_resolver::GitResolver;
use crate::formatter::language_resolver::{LanguageResolver, select_languages};
use crate::formatter::status_resolver::StatusResolver;
use crate::formatter::{Formatter, IsFirst, IsLast, PromptDialect, RenderContext, SegmentPosition};
use crate::ipc::{LanguageInfo, Response};
use crate::template::VariableResolver;

/// Format responses as ANSI-rendered prompt segments in one [`PromptDialect`].
#[derive(Debug, Clone, Copy, Default)]
pub struct PromptFormatter {
    dialect: PromptDialect,
}

impl PromptFormatter {
    /// A formatter that encodes segment text for `dialect`.
    #[must_use]
    pub const fn new(dialect: PromptDialect) -> Self {
        Self { dialect }
    }
}

impl Formatter for PromptFormatter {
    fn render(&self, response: &Response, ctx: &RenderContext<'_>) -> Result<String> {
        let dialect = self.dialect;
        let segments = &ctx.theme.segments;
        let (config, theme, pos) = (ctx.config, ctx.theme, ctx.position);
        let rendered = match response {
            Response::RepositoryStatus(repo) => render_resolved(
                "git segment",
                segments.git.format.as_deref(),
                ctx,
                dialect,
                || {
                    let state = crate::theme::GitState::from_status(repo);
                    GitResolver::new(repo, state, config, theme, pos)
                },
            ),
            Response::Language { languages } => render_segment(
                "language segment",
                segments.language.format.as_deref(),
                |f| render_languages_via_template(languages, ctx, dialect, f),
            ),
            Response::Directory { cwd, read_only } => render_resolved(
                "directory segment",
                segments.directory.format.as_deref(),
                ctx,
                dialect,
                || DirectoryResolver::new(cwd, *read_only, ctx.directory, config, theme, pos),
            ),
            Response::Clock { shell } => render_resolved(
                "clock segment",
                segments.clock.format.as_deref(),
                ctx,
                dialect,
                || ClockResolver::new(*shell, config, theme, pos),
            ),
            Response::Duration { duration_ms } => render_resolved(
                "duration segment",
                segments.duration.format.as_deref(),
                ctx,
                dialect,
                || DurationResolver::new(*duration_ms, config, theme, pos),
            ),
            Response::Character { success } => render_resolved(
                "character segment",
                segments.character.format.as_deref(),
                ctx,
                dialect,
                || CharacterResolver::new(*success, config, theme, pos),
            ),
            Response::ExitStatus { success } => render_resolved(
                "status segment",
                segments.status.format.as_deref(),
                ctx,
                dialect,
                || StatusResolver::new(*success, config, theme, pos),
            ),
            // The reply is printed verbatim into the prompt, so protocol JSON must never reach it.
            Response::Error { message } => {
                crate::debug_log!("formatter", "omitting segment after error: {message}");
                String::new()
            }
            Response::Ack | Response::AgentStatus(_) => String::new(),
        };
        Ok(rendered)
    }
}

/// Render the segment whose variables `resolver` supplies; the resolver is only
/// built when `format` is set.
fn render_resolved<R: VariableResolver>(
    component: &str,
    format: Option<&str>,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
    resolver: impl FnOnce() -> R,
) -> String {
    render_segment(component, format, |f| {
        render_via_template(&resolver(), ctx, dialect, f)
    })
}

/// Render `resolver` with `format`, inheriting `ctx`'s prev colors and palette, and encode.
fn render_via_template(
    resolver: &dyn VariableResolver,
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
    format: &str,
) -> crate::template::Result<String> {
    use crate::template::{EvalContext, render};

    let template_ctx = EvalContext::new(resolver)
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

/// No format → empty; template error → empty + `warn_fallback`.
fn render_segment(
    component: &str,
    format: Option<&str>,
    build: impl FnOnce(&str) -> crate::template::Result<String>,
) -> String {
    let Some(template) = format else {
        return String::new();
    };
    render_or_warn(component, build(template))
}

/// Render each selected language independently and concatenate.
fn render_languages_via_template(
    languages: &[LanguageInfo],
    ctx: &RenderContext<'_>,
    dialect: PromptDialect,
    format: &str,
) -> crate::template::Result<String> {
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

#[cfg(test)]
mod tests;
