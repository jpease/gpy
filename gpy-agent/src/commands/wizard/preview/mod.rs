//! Renders the wizard's live preview line through the real template engine,
//! converting its pre-ANSI spans directly to `ratatui` spans (no ANSI parsing).

use crate::commands::wizard::facts::PreviewFacts;
use crate::commands::wizard::state::WizardState;
use crate::config::Config;
use crate::config::types::Icon;
use crate::formatter::IsFirst;
use crate::formatter::IsLast;
use crate::formatter::RenderContext as FormatterRenderContext;
use crate::formatter::SegmentPosition;
use crate::formatter::character_resolver::CharacterResolver;
use crate::formatter::directory_resolver::DirectoryResolver;
use crate::formatter::duration_resolver::DurationResolver;
use crate::formatter::git_resolver::GitResolver;
use crate::formatter::hostname_resolver::HostnameResolver;
use crate::formatter::language_resolver::LanguageResolver;
use crate::formatter::separator::Glyphs;
use crate::formatter::username_resolver::UsernameResolver;
use crate::plugin::BuiltinSegment;
use crate::template::{
    Attr, Color as TemplateColor, RenderContext as TemplateRenderContext, Span as TemplateSpan,
    SpanKind, Style as TemplateStyle, VariableResolver, canonical_ansi_name,
};
use crate::theme::{GitState, ThemeConfig};
use ratatui::style::{Color as RatatuiColor, Modifier, Style as RatatuiStyle};
use ratatui::text::{Line, Span as RatatuiSpan};

/// Fixed sample cwd for the directory segment.
const SAMPLE_CWD: &str = "~/dev/gpy";

/// Template color → `ratatui` color. `Palette`/`PrevFg`/`PrevBg` are already
/// resolved by `template::render`, so they map to `None`.
pub(super) fn to_ratatui_color(color: &TemplateColor) -> Option<RatatuiColor> {
    match color {
        TemplateColor::Named(name) => named_to_ratatui(name),
        TemplateColor::Rgb { r, g, b } => Some(RatatuiColor::Rgb(*r, *g, *b)),
        TemplateColor::Ansi256(index) => Some(RatatuiColor::Indexed(*index)),
        TemplateColor::Palette(_) | TemplateColor::PrevFg | TemplateColor::PrevBg => None,
    }
}

/// Named color → `ratatui`, matching gpy's SGR table: gpy `"white"` (ANSI 37)
/// is ratatui `Gray`, gpy `"bright-white"` (ANSI 97) is ratatui `White`.
fn named_to_ratatui(name: &str) -> Option<RatatuiColor> {
    let canonical = if name == "default" {
        name
    } else {
        canonical_ansi_name(name)?
    };
    match canonical {
        "black" => Some(RatatuiColor::Black),
        "red" => Some(RatatuiColor::Red),
        "green" => Some(RatatuiColor::Green),
        "yellow" => Some(RatatuiColor::Yellow),
        "blue" => Some(RatatuiColor::Blue),
        "purple" => Some(RatatuiColor::Magenta),
        "cyan" => Some(RatatuiColor::Cyan),
        "white" => Some(RatatuiColor::Gray),
        "bright-black" => Some(RatatuiColor::DarkGray),
        "bright-red" => Some(RatatuiColor::LightRed),
        "bright-green" => Some(RatatuiColor::LightGreen),
        "bright-yellow" => Some(RatatuiColor::LightYellow),
        "bright-blue" => Some(RatatuiColor::LightBlue),
        "bright-purple" => Some(RatatuiColor::LightMagenta),
        "bright-cyan" => Some(RatatuiColor::LightCyan),
        "bright-white" => Some(RatatuiColor::White),
        "default" => Some(RatatuiColor::Reset),
        _ => None,
    }
}

const fn to_ratatui_modifier(attr: Attr) -> Modifier {
    match attr {
        Attr::Bold => Modifier::BOLD,
        Attr::Italic => Modifier::ITALIC,
        Attr::Underline => Modifier::UNDERLINED,
        Attr::Dimmed => Modifier::DIM,
        Attr::Inverted => Modifier::REVERSED,
        Attr::Blink => Modifier::SLOW_BLINK,
        Attr::Hidden => Modifier::HIDDEN,
        Attr::Strikethrough => Modifier::CROSSED_OUT,
    }
}

fn to_ratatui_style(style: &TemplateStyle) -> RatatuiStyle {
    let mut out = RatatuiStyle::default();
    if let Some(fg) = style.fg.as_ref().and_then(to_ratatui_color) {
        out = out.fg(fg);
    }
    if let Some(bg) = style.bg.as_ref().and_then(to_ratatui_color) {
        out = out.bg(bg);
    }
    for attr in &style.attrs {
        out = out.add_modifier(to_ratatui_modifier(*attr));
    }
    out
}

fn to_ratatui_span(span: &TemplateSpan) -> RatatuiSpan<'static> {
    RatatuiSpan::styled(span.text.clone(), to_ratatui_style(&span.style))
}

/// One chain segment's spans plus the prev-colors used, so the last entry can
/// be re-rendered with `is_last`.
struct ChainEntry {
    segment: String,
    spans: Vec<TemplateSpan>,
    incoming_fg: Option<TemplateColor>,
    incoming_bg: Option<TemplateColor>,
    /// Known at push time, unlike `is_last`.
    is_first: bool,
}

/// Configured list order, then unmentioned offered segments.
fn preview_order<'a>(
    state: &'a WizardState,
    config: &'a Config,
) -> impl Iterator<Item = &'a String> {
    let unlisted = state
        .available_segments()
        .iter()
        .filter(|segment| !config.ui.enabled_segments.contains(segment));
    config.ui.enabled_segments.iter().chain(unlisted)
}

/// Preview lines for the selected theme/palette/segments: the segment chain,
/// then the prompt character on its own line, as in `fish_prompt.fish`.
///
/// `is_first`/`is_last` are decided by position among *rendered* segments,
/// never by segment identity.
pub fn render_preview_line(
    state: &WizardState,
    config: &Config,
    theme: &ThemeConfig,
    palette: &crate::template::Palette,
    facts: &PreviewFacts,
) -> Vec<Line<'static>> {
    // With icons off the live prompt shows no private-use delimiters; match it.
    if Glyphs::from(&config.ui) == Glyphs::Nerd {
        return render_chain(state, config, theme, palette, facts);
    }
    render_chain(
        state,
        config,
        &without_private_use_delimiters(theme),
        palette,
        facts,
    )
}

/// `theme` with private-use (Nerd Font) delimiter glyphs removed.
fn without_private_use_delimiters(theme: &ThemeConfig) -> ThemeConfig {
    let mut plain = theme.clone();
    let delimiters = [
        &mut plain.ui.prompt_open,
        &mut plain.ui.prompt_close,
        &mut plain.ui.segment_open,
        &mut plain.ui.segment_close,
    ];
    for delimiter in delimiters.into_iter().flatten() {
        let kept = Glyphs::Plain.delimiter(delimiter.icon.as_str()).to_owned();
        // Cannot fail: `kept` is the validated original or empty.
        if let Ok(icon) = Icon::new(kept) {
            delimiter.icon = icon;
        }
    }
    plain
}

#[expect(
    clippy::similar_names,
    reason = "incoming_fg/incoming_bg are a deliberately paired fg/bg pair"
)]
fn render_chain(
    state: &WizardState,
    config: &Config,
    theme: &ThemeConfig,
    palette: &crate::template::Palette,
    facts: &PreviewFacts,
) -> Vec<Line<'static>> {
    let mut chain: Vec<ChainEntry> = Vec::new();
    let mut prev_foreground: Option<TemplateColor> = None;
    let mut prev_background: Option<TemplateColor> = None;

    for segment in preview_order(state, config) {
        if !state.is_segment_enabled(segment) {
            continue;
        }

        let incoming_fg = prev_foreground.clone();
        let incoming_bg = prev_background.clone();
        let is_first = chain.is_empty();
        let position = SegmentPosition::new(IsLast::No, IsFirst::from(is_first));
        let ctx = FormatterRenderContext::new(config, theme, position)
            .with_palette(palette.clone())
            .with_prev_colors(incoming_fg.clone(), incoming_bg.clone());

        let Some(spans) = render_segment(segment, config, theme, facts, &ctx) else {
            continue;
        };

        if let Some(last) = spans.last() {
            prev_foreground.clone_from(&last.style.fg);
            prev_background.clone_from(&last.style.bg);
        }

        chain.push(ChainEntry {
            segment: segment.clone(),
            spans,
            incoming_fg,
            incoming_bg,
            is_first,
        });
    }

    // `is_last` only affects the segment's own trailing tokens, so re-rendering
    // just the last entry is equivalent to re-rendering the chain.
    if let Some(last_entry) = chain.last_mut() {
        let position = SegmentPosition::new(IsLast::Yes, IsFirst::from(last_entry.is_first));
        let ctx = FormatterRenderContext::new(config, theme, position)
            .with_palette(palette.clone())
            .with_prev_colors(
                last_entry.incoming_fg.clone(),
                last_entry.incoming_bg.clone(),
            );
        if let Some(spans) = render_segment(&last_entry.segment, config, theme, facts, &ctx) {
            last_entry.spans = spans;
        }
    }

    let main_spans: Vec<RatatuiSpan<'static>> = chain
        .iter()
        .flat_map(|entry| entry.spans.iter().map(to_ratatui_span))
        .collect();
    let mut lines = vec![Line::from(main_spans)];

    if let Some(format) = theme.segments.character.format.as_deref() {
        let resolver =
            CharacterResolver::new(facts.sample_character_success, theme, SegmentPosition::LAST)
                .with_glyphs(Glyphs::from(&config.ui));
        let ctx = FormatterRenderContext::new(config, theme, SegmentPosition::LAST)
            .with_palette(palette.clone())
            .with_prev_colors(prev_foreground, prev_background);
        if let Ok(spans) = crate::template::render(format, &template_ctx(&resolver, &ctx)) {
            lines.push(Line::from(
                spans.iter().map(to_ratatui_span).collect::<Vec<_>>(),
            ));
        }
    }

    lines
}

fn render_git_span(
    config: &Config,
    theme: &ThemeConfig,
    facts: &PreviewFacts,
    ctx: &FormatterRenderContext<'_>,
) -> Option<Vec<TemplateSpan>> {
    let status = facts.repository_status.as_ref()?;
    let format = theme.segments.git.format.as_deref()?;
    let state = GitState::from_status(status);
    let resolver = GitResolver::new(status, config, theme, state, ctx.position);
    crate::template::render(format, &template_ctx(&resolver, ctx)).ok()
}

fn render_directory_span(
    config: &Config,
    theme: &ThemeConfig,
    ctx: &FormatterRenderContext<'_>,
) -> Option<Vec<TemplateSpan>> {
    let format = theme.segments.directory.format.as_deref()?;
    let resolver = DirectoryResolver::new(SAMPLE_CWD, false, config, theme, ctx.position);
    crate::template::render(format, &template_ctx(&resolver, ctx)).ok()
}

/// Render one segment via the template engine; `None` when it has no format,
/// no data, or fails to render (as the real prompt treats errors).
fn render_segment(
    segment: &str,
    config: &Config,
    theme: &ThemeConfig,
    facts: &PreviewFacts,
    ctx: &FormatterRenderContext<'_>,
) -> Option<Vec<TemplateSpan>> {
    // Plugin segments render shell-side; not previewed.
    let Ok(builtin) = BuiltinSegment::try_from(segment) else {
        return None;
    };
    match builtin {
        BuiltinSegment::Git => render_git_span(config, theme, facts, ctx),
        BuiltinSegment::Language => {
            let format = theme.segments.language.format.as_deref()?;
            // Real prompt renders up to three; preview shows the first selected
            // one, via `select_languages` so `show_versions` behaves the same.
            let lang =
                *crate::formatter::select_languages(config, theme, &facts.languages).first()?;
            let resolver = LanguageResolver::new(lang, config, theme, ctx.position);
            crate::template::render(format, &template_ctx(&resolver, ctx)).ok()
        }
        BuiltinSegment::Directory => render_directory_span(config, theme, ctx),
        BuiltinSegment::Duration => {
            let format = theme.segments.duration.format.as_deref()?;
            let resolver = DurationResolver::new(facts.sample_duration_ms, theme, ctx.position)
                .with_glyphs(Glyphs::from(&config.ui));
            crate::template::render(format, &template_ctx(&resolver, ctx)).ok()
        }
        BuiltinSegment::Status => Some(status_pill_spans(
            theme,
            facts.sample_character_success,
            ctx.position,
            Glyphs::from(&config.ui),
        )),
        // Clock renders shell-side against wall-clock time; show a demo time.
        BuiltinSegment::Clock => Some(clock_demo_spans(
            theme,
            ctx.position.is_first,
            ctx.position.is_last,
        )),
        BuiltinSegment::Hostname => {
            let format = theme.segments.hostname.format.as_deref()?;
            let resolver = HostnameResolver::new(
                facts.hostname.clone(),
                facts.sample_is_ssh,
                theme,
                ctx.position,
            )
            .with_glyphs(Glyphs::from(&config.ui));
            crate::template::render(format, &template_ctx(&resolver, ctx)).ok()
        }
        BuiltinSegment::Username => {
            let format = theme.segments.username.format.as_deref()?;
            let resolver = UsernameResolver::new(facts.username.clone(), theme, ctx.position)
                .with_glyphs(Glyphs::from(&config.ui));
            crate::template::render(format, &template_ctx(&resolver, ctx)).ok()
        }
    }
}

/// Fixed demo clock (9:41) capped like the shell-rendered clock, using the
/// global `ui.*` delimiters (`segments.clock.open`/`.close` are unused config).
fn clock_demo_spans(theme: &ThemeConfig, is_first: IsFirst, is_last: IsLast) -> Vec<TemplateSpan> {
    let clock = &theme.segments.clock;
    let is_24h = clock.time_format.as_deref() == Some("24");
    let leading_zero = clock.show_leading_zero.unwrap_or(false);
    let show_seconds = clock.show_seconds.unwrap_or(false);

    let mut text = String::from(if leading_zero { " 09:41" } else { " 9:41" });
    if show_seconds {
        text.push_str(":05");
    }
    if !is_24h {
        text.push_str(" AM");
    }

    standalone_pill_spans(
        text,
        clock.bg_color.as_str(),
        clock.text_color.as_str(),
        theme,
        SegmentPosition::new(is_last, is_first),
    )
}

/// Exit-status pill, mirroring `fish/segments/status.fish`: a Nerd Font icon
/// under [`Glyphs::Plain`] gives way to the shells' ASCII fallback.
fn status_pill_spans(
    theme: &ThemeConfig,
    success: bool,
    position: SegmentPosition,
    glyphs: Glyphs,
) -> Vec<TemplateSpan> {
    let status = &theme.segments.status;
    let (icon, nerd_fallback, plain_fallback, bg, fg) = if success {
        (
            status.ok_icon.as_ref(),
            "✔",
            "ok",
            status.ok_bg_color.as_str(),
            status.ok_text_color.as_str(),
        )
    } else {
        (
            status.fail_icon.as_ref(),
            "✖",
            "err",
            status.fail_bg_color.as_str(),
            status.fail_text_color.as_str(),
        )
    };
    let fallback = match glyphs {
        Glyphs::Nerd => nerd_fallback,
        Glyphs::Plain => plain_fallback,
    };
    let text = icon
        .and_then(|configured| glyphs.icon(configured.as_str()))
        .unwrap_or(fallback)
        .to_owned();
    standalone_pill_spans(text, bg, fg, theme, position)
}

/// Theme-colored pill with caps, as `gpy_section_standalone` draws.
fn standalone_pill_spans(
    text: String,
    bg: &str,
    fg: &str,
    theme: &ThemeConfig,
    position: SegmentPosition,
) -> Vec<TemplateSpan> {
    let (is_first, is_last) = (position.is_first, position.is_last);
    let ui = &theme.ui;

    let mut spans = Vec::new();
    let open_icon = if is_first.as_bool() {
        ui.get_prompt_open_delimiter()
    } else {
        ui.get_segment_open_delimiter()
    };
    let open_config = if is_first.as_bool() {
        ui.prompt_open.as_ref()
    } else {
        ui.segment_open.as_ref()
    };
    spans.extend(clock_cap_span(open_icon, open_config, bg, fg));

    spans.push(TemplateSpan {
        text,
        style: TemplateStyle {
            fg: Some(TemplateColor::Named(fg.to_owned())),
            bg: Some(TemplateColor::Named(bg.to_owned())),
            attrs: Vec::new(),
        },
        kind: SpanKind::Text,
    });

    // Backdrop continues to a non-last closing cap, as in `gpy_section_standalone`.
    if !is_last.as_bool() {
        spans.push(TemplateSpan {
            text: " ".to_owned(),
            style: TemplateStyle {
                fg: None,
                bg: Some(TemplateColor::Named(bg.to_owned())),
                attrs: Vec::new(),
            },
            kind: SpanKind::Text,
        });
    }

    let close_icon = if is_last.as_bool() {
        ui.get_prompt_close_delimiter()
    } else {
        ui.get_segment_close_delimiter()
    };
    let close_config = if is_last.as_bool() {
        ui.prompt_close.as_ref()
    } else {
        ui.segment_close.as_ref()
    };
    spans.extend(clock_cap_span(close_icon, close_config, bg, fg));

    spans
}

/// Resolve `match_bg`/`match_text`/`transparent`/literal like `gpy_section_standalone`.
#[expect(clippy::similar_names, reason = "paired bg/fg")]
fn resolve_clock_cap_color(keyword: &str, clock_bg: &str, clock_fg: &str) -> Option<TemplateColor> {
    match keyword {
        "transparent" => None,
        "match_bg" => Some(TemplateColor::Named(clock_bg.to_owned())),
        "match_text" => Some(TemplateColor::Named(clock_fg.to_owned())),
        other => Some(TemplateColor::Named(other.to_owned())),
    }
}

/// Clock cap span; `None` for an empty icon. Missing `config` uses
/// `DelimiterConfig`'s serde defaults.
#[expect(clippy::similar_names, reason = "paired bg/fg")]
fn clock_cap_span(
    icon: &str,
    config: Option<&crate::config::DelimiterConfig>,
    clock_bg: &str,
    clock_fg: &str,
) -> Option<TemplateSpan> {
    if icon.is_empty() {
        return None;
    }
    let icon_color = config.map_or("white", |c| c.icon_color.as_str());
    let bg_color = config.map_or("transparent", |c| c.bg_color.as_str());
    Some(TemplateSpan {
        text: icon.to_owned(),
        style: TemplateStyle {
            fg: resolve_clock_cap_color(icon_color, clock_bg, clock_fg),
            bg: resolve_clock_cap_color(bg_color, clock_bg, clock_fg),
            attrs: Vec::new(),
        },
        kind: SpanKind::Text,
    })
}

/// Mirrors `fish_ansi`'s `render_via_template` context plumbing.
fn template_ctx<'a>(
    resolver: &'a dyn VariableResolver,
    ctx: &FormatterRenderContext<'a>,
) -> TemplateRenderContext<'a> {
    TemplateRenderContext::new(resolver)
        .with_prev_colors(ctx.prev_fg.clone(), ctx.prev_bg.clone())
        .with_palette(ctx.palette.clone())
}

#[cfg(test)]
mod tests;
