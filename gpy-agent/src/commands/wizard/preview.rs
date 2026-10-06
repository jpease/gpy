//! Renders the wizard's live preview line.
//!
//! Runs each enabled segment through the *real* template engine — the same
//! one [`crate::formatter::FishAnsiFormatter`] uses — and converts its
//! structured, pre-ANSI spans directly into `ratatui` spans.
//!
//! This hooks in one layer below `Formatter::render` (which produces an ANSI
//! *string*) and one layer below [`crate::formatter::encode_ansi`] (which
//! consumes structured spans): directly at `crate::template::render(format,
//! &ctx) -> Vec<template::Span>`, the exact call
//! `formatter::fish_ansi::render_via_template` makes right before it hands
//! spans to `encode_ansi` (see `gpy-agent/src/formatter/fish_ansi.rs`). This
//! is the whole point of the design: no ANSI-string parsing anywhere here.
//!
//! `wizard/mod.rs`'s render loop constructs a [`WizardState`], config, and
//! theme and calls [`render_preview_line`] from its draw closure each frame.

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

/// Representative preview-only working directory for the directory segment.
///
/// The wizard has no live "current directory" concept threaded through
/// [`PreviewFacts`] (unlike git/language, which are gathered for the wizard's
/// real `cwd` — see `facts.rs`) — the directory segment's own text is purely
/// cosmetic for the preview, so a fixed, plausible path is used here, the
/// same "representative sample" rationale `PreviewFacts::sample_duration_ms`
/// and `PreviewFacts::sample_character_success` document.
const SAMPLE_CWD: &str = "~/dev/gpy";

// ---------------------------------------------------------------------
// Part A: `template::Span` -> `ratatui::text::Span` adapter
// ---------------------------------------------------------------------

/// Convert one resolved template color to a `ratatui` color.
///
/// `Palette`/`PrevFg`/`PrevBg` are unreachable here in practice —
/// `template::render` fully resolves them before a span is returned (see
/// `crate::template::eval::RenderContext::resolve_color`) — but are handled
/// defensively as "no color" rather than panicking, matching how
/// `crate::formatter::style_encoder::encode_ansi` itself treats them as
/// no-ops (`Color::Palette(_) | Color::PrevFg | Color::PrevBg => {}`).
pub(super) fn to_ratatui_color(color: &TemplateColor) -> Option<RatatuiColor> {
    match color {
        TemplateColor::Named(name) => named_to_ratatui(name),
        TemplateColor::Rgb { r, g, b } => Some(RatatuiColor::Rgb(*r, *g, *b)),
        TemplateColor::Ansi256(index) => Some(RatatuiColor::Indexed(*index)),
        TemplateColor::Palette(_) | TemplateColor::PrevFg | TemplateColor::PrevBg => None,
    }
}

/// Map a named color to its `ratatui` equivalent.
///
/// Accepts the vocabulary `crate::template::style::parse_color` accepts (every
/// spelling `canonical_ansi_name` maps, plus `"default"`, which
/// `parse_color` normalizes to `Named("default")`) to `ratatui` colors.
///
/// Named per ANSI code, not per ratatui's user-friendly `FromStr` for
/// `ratatui::style::Color` (which treats the English word "white" as bright
/// white). `crate::formatter::style_encoder`'s own `Named` → SGR-code table
/// is the authoritative mapping this must match: `"white"` is ANSI 37 (the
/// *non*-bright white) and `"bright-white"` is ANSI 97 (bright white) — see
/// `style_encoder.rs`'s `"white" => 37` / `"bright-white" => 97`. `ratatui`
/// spells these the other way around from what the names suggest:
/// `ratatui::style::Color::White` is documented as ANSI 97 (bright white)
/// and `ratatui::style::Color::Gray` is documented as ANSI 37 (the "silver"/
/// standard white) — so gpy's `"white"` maps to ratatui's `Gray`, and gpy's
/// `"bright-white"` maps to ratatui's `White`. Getting this backwards is an
/// easy trap — this mapping is verified directly against both source files
/// rather than assumed.
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
        // "default" (terminal default fg/bg) has a direct ratatui equivalent,
        // `Color::Reset` — "explicitly reset to the terminal's own color" is
        // exactly what gpy's ANSI encoder does for `Named("default")` (SGR
        // 39/49), so this is a real mapping, not a defensive fallback.
        "default" => Some(RatatuiColor::Reset),
        _ => None,
    }
}

/// Convert a template style's attributes into a `ratatui` modifier.
///
/// Variant names verified against `ratatui` 0.30.2 (`ratatui-core` 0.1.2,
/// see `Cargo.toml`'s `ratatui = "0.30.2"` pin) — its `Modifier` bitflags are
/// `BOLD`/`DIM`/`ITALIC`/`UNDERLINED`/`SLOW_BLINK`/`RAPID_BLINK`/`REVERSED`/
/// `HIDDEN`/`CROSSED_OUT`, unchanged from pre-0.30 versions for this basic
/// set.
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

/// Convert a resolved template style into a `ratatui` style.
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

/// Convert one template span into an owned `ratatui` span.
fn to_ratatui_span(span: &TemplateSpan) -> RatatuiSpan<'static> {
    RatatuiSpan::styled(span.text.clone(), to_ratatui_style(&span.style))
}

// ---------------------------------------------------------------------
// Part B: per-segment composition into one preview line
// ---------------------------------------------------------------------

/// One chain segment's rendered output plus the prev-colors it was rendered with.
///
/// Kept around so [`render_preview_line`] can re-render just the last entry
/// once the true last index is known — see that function's doc comment.
struct ChainEntry {
    segment: String,
    spans: Vec<TemplateSpan>,
    incoming_fg: Option<TemplateColor>,
    incoming_bg: Option<TemplateColor>,
    /// Whether this was the first entry pushed onto the chain — known at push
    /// time (unlike `is_last`, which needs the whole chain built first), so
    /// it's set correctly on the very first render and just carried along for
    /// the later `is_last` re-render to reuse.
    is_first: bool,
}

/// Segments in the order the preview draws them: the configured list order
/// (what a save will write), then any offered segment the list does not
/// mention, in `available_segments()` order.
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

/// Render live preview lines for the wizard's currently-selected
/// theme/palette/segments: one line for the main segment chain, plus a
/// second line for the prompt character (`❯`), which is always drawn.
///
/// `theme` and `palette` are expected to be the wizard's *selected* (not
/// necessarily on-disk-active) theme/palette — loading those from
/// `state.selected_theme()`/`state.selected_palette()` is the caller's job,
/// via `ThemeManager::new` / `crate::palette::resolve::palette_by_name`.
///
/// Three things mirror the real prompt (`fish_prompt.fish`) rather than
/// treating every segment as independent:
///
/// - Only the true last chain segment renders with `is_last = true` (closing
///   powerline cap); every other enabled segment renders `is_last = false`
///   (continuation glyph into the next segment) — matching
///   `fish_prompt.fish`'s `current_idx -eq $segment_count` check.
/// - Only the true first *rendered* chain segment (the first entry actually
///   pushed onto `chain` — a segment that's enabled but renders nothing, e.g.
///   no `format` configured, never occupies this slot) renders with
///   `is_first = true` (suppressed opening cap); this is determined purely by
///   chain position, matching `fish_prompt.fish`'s `current_idx -eq 1` check
///   — never by segment identity (e.g. hardcoding "clock is always first").
/// - The prompt character is never part of that chain and is never
///   toggled by a segment: the real prompt always renders it on its own line,
///   after the chain, from `theme.segments.character` with `is_last`
///   hardcoded true (`fish_prompt.fish`'s `__gpy_request_character
///   $char_success true $__gpy_last_segment_bg` call, preceded by a bare
///   `echo`/newline). It's rendered here as a second `Line` (omitted only
///   when the theme has no character format), picking up the chain's final
///   prev-colors the same way `$__gpy_last_segment_bg` does. The `"status"`
///   segment is unrelated to it: an exit-status pill drawn inside the chain
///   at its list position.
pub fn render_preview_line(
    state: &WizardState,
    config: &Config,
    theme: &ThemeConfig,
    palette: &crate::template::Palette,
    facts: &PreviewFacts,
) -> Vec<Line<'static>> {
    // The preview must show what the live prompt draws. With icons off the
    // agent resolves no `$sep_*` caps and the theme export erases private-use
    // delimiters, so the clock and status pills drawn from the theme's own
    // delimiters must not show them here either (#695).
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

/// `theme` with every private-use (Nerd Font) delimiter glyph removed; plain
/// text delimiters such as `[`/`]` are kept ([`Glyphs::delimiter`]).
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
        // Cannot fail: `kept` is either the original, already-validated icon
        // text or empty.
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

    // `is_last` only changes a segment's own trailing tokens (`sep_close`/
    // `sep_gap`), never anything upstream — so re-rendering just the last
    // entry in place with `is_last = true` is equivalent to (and cheaper
    // than) rendering the whole chain a second time. Its `is_first` was
    // already correct from the first pass (chain position is known
    // immediately, unlike `is_last`), so it's carried through unchanged —
    // this matters when the chain has exactly one entry that is both first
    // and last.
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

/// Render the git segment (split out of [`render_segment`] because its setup is
/// its own distinct step — deriving a `GitState` and building a
/// `GitResolver`, which alone spans several lines).
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

/// Render the directory segment (split out of [`render_segment`] for the
/// same reason as [`render_git_span`]).
fn render_directory_span(
    config: &Config,
    theme: &ThemeConfig,
    ctx: &FormatterRenderContext<'_>,
) -> Option<Vec<TemplateSpan>> {
    let format = theme.segments.directory.format.as_deref()?;
    let resolver = DirectoryResolver::new(SAMPLE_CWD, false, config, theme, ctx.position);
    crate::template::render(format, &template_ctx(&resolver, ctx)).ok()
}

/// Render one segment via the template engine.
///
/// Returns `None` when the theme has no `format` configured for it, or the
/// segment has no data to render — matching `fish_ansi.rs`'s "no format =
/// empty segment" contract for every segment type. Rendering errors are also
/// downgraded to `None` (the wizard preview should never crash on a
/// malformed theme; the real prompt path already treats template errors as
/// empty output too, see `fish_ansi.rs`'s `render_or_warn`).
///
/// Segment names are the ones `commands::segments::available_segments()`
/// (via `BUILTIN_ORDER`) produces: `clock`, `duration`, `language`,
/// `directory`, `git`, `status`, `username`, `hostname`. `"status"` renders
/// the exit-status pill from `theme.segments.status` (`StatusTheme`, which has
/// no `format` template), like `fish/segments/status.fish`; the prompt
/// character (`theme.segments.character`) is rendered separately by
/// [`render_preview_line`].
fn render_segment(
    segment: &str,
    config: &Config,
    theme: &ThemeConfig,
    facts: &PreviewFacts,
    ctx: &FormatterRenderContext<'_>,
) -> Option<Vec<TemplateSpan>> {
    // Any non-builtin (e.g. plugin) segment name yields no preview: plugin
    // segments render in Fish, not in this preview — out of scope.
    let Ok(builtin) = BuiltinSegment::try_from(segment) else {
        return None;
    };
    match builtin {
        BuiltinSegment::Git => render_git_span(config, theme, facts, ctx),
        BuiltinSegment::Language => {
            let format = theme.segments.language.format.as_deref()?;
            // Real rendering (`fish_ansi.rs::render_languages_via_template`)
            // concatenates up to three selected languages; the preview keeps
            // this simple and renders just the first *selected* one — running
            // `select_languages` first (rather than taking `facts.languages`
            // raw) so toggling `language.show_versions` in the wizard is
            // reflected here exactly as it would be in the real prompt: with
            // it on, a versionless first-detected language is dropped and the
            // segment vanishes (or the next qualifying language takes its
            // place), matching `select_languages`'s doc comment.
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
        )),
        // Clock has no `format`/template — it's rendered entirely Fish-side
        // against live wall-clock time (see `fish/segments/clock.fish`), which
        // this static preview pane can't demo. Show a fixed representative
        // time instead (see `clock_demo_spans`'s doc comment) rather than
        // leaving the segment invisible whenever it's enabled.
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

/// Fixed demo time shown for the clock segment in the wizard preview.
///
/// Capped with the same opening/closing powerline glyphs the real
/// Fish-rendered clock segment gets from `gpy_section_standalone`
/// (`fish/core/renderer.fish`) — issue: the wizard preview was previously
/// rendering clock with no cap glyphs at all, which only matched a theme
/// with entirely empty delimiter icons (e.g. the `starship` preset); themes
/// with real delimiter icons — including the `default` theme itself, whose
/// `ui.segment_open`/`ui.segment_close`/`ui.prompt_close` icons are the same
/// Nerd Font powerline glyphs (`""`/`""`/`""`) the other segments' `sep_open`/
/// `sep_close` template tokens hardcode — showed a visibly different (capless)
/// clock in the preview than the live prompt.
///
/// The time text itself is a fixed, representative sample (9:41, chosen to
/// need no AM/PM-boundary special-casing at hour 12/midnight) — the real
/// prompt always reflects live wall-clock time, which a static preview pane
/// can't demo — the same "representative sample" rationale as [`SAMPLE_CWD`]
/// and `PreviewFacts::sample_duration_ms`.
///
/// Deliberately does not reproduce `segments.clock.open`/`.close` (a
/// *different*, per-clock delimiter override): those are unused dead config
/// today (`theme/export.rs` never reads them). The real prompt's clock caps
/// come from the session-global `ui.prompt_open`/`ui.prompt_close`/
/// `ui.segment_open`/`ui.segment_close` delimiters instead, which this
/// function does reproduce.
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

/// Build the exit-status pill.
///
/// Uses `theme.segments.status`'s ok variant when the sample command
/// succeeded, the fail variant otherwise. Mirrors `fish/segments/status.fish` (icon-only content, `✔`/`✖` when the theme
/// leaves the icon unset, pill drawn by `gpy_section_standalone`).
fn status_pill_spans(
    theme: &ThemeConfig,
    success: bool,
    position: SegmentPosition,
) -> Vec<TemplateSpan> {
    let status = &theme.segments.status;
    let (icon, fallback, bg, fg) = if success {
        (
            status.ok_icon.as_ref(),
            "✔",
            status.ok_bg_color.as_str(),
            status.ok_text_color.as_str(),
        )
    } else {
        (
            status.fail_icon.as_ref(),
            "✖",
            status.fail_bg_color.as_str(),
            status.fail_text_color.as_str(),
        )
    };
    let text = icon
        .map_or(fallback, |configured| configured.as_str())
        .to_owned();
    standalone_pill_spans(text, bg, fg, theme, position)
}

/// Draw `text` as a theme-colored pill with open/close caps, the way
/// `gpy_section_standalone` (`fish/core/renderer.fish`) does for the
/// shell-rendered clock and status segments.
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

    // Same-background gap space before a non-last closing cap: mirrors
    // `gpy_section_standalone`'s own comment — the segment's backdrop
    // continues right up to the cap instead of falling through early.
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

/// Resolve one delimiter color keyword against the clock segment's own bg/fg.
///
/// Accepts `match_bg`, `match_text`, `transparent`, or a literal color name —
/// mirrors `gpy_section_standalone`'s per-keyword `switch` blocks in
/// `fish/core/renderer.fish`, which resolve a cap's `icon_color`/`bg_color`
/// the same way relative to the `$bg`/`$fg` arguments Fish passes it.
#[expect(
    clippy::similar_names,
    reason = "clock_bg/clock_fg are a deliberately paired bg/fg pair"
)]
fn resolve_clock_cap_color(keyword: &str, clock_bg: &str, clock_fg: &str) -> Option<TemplateColor> {
    match keyword {
        "transparent" => None,
        "match_bg" => Some(TemplateColor::Named(clock_bg.to_owned())),
        "match_text" => Some(TemplateColor::Named(clock_fg.to_owned())),
        other => Some(TemplateColor::Named(other.to_owned())),
    }
}

/// Build one clock powerline-cap span (opening or closing).
///
/// Built from its resolved icon text and `DelimiterConfig`, or `None` when
/// the icon is empty — matching `gpy_section_standalone`'s `if test -n
/// "$start_delim"`/`"$end_delim"` guard, which prints nothing for an empty
/// delimiter rather than a blank glyph. `config` is `None` only when a theme
/// omits the delimiter section entirely, in which case the same
/// `icon_color`/`bg_color` defaults `DelimiterConfig`'s serde defaults use
/// (`"white"`/`"transparent"`) apply.
#[expect(
    clippy::similar_names,
    reason = "clock_bg/clock_fg are a deliberately paired bg/fg pair"
)]
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

/// Build a `template::RenderContext` from a `formatter::RenderContext`.
///
/// Mirrors `fish_ansi.rs`'s `render_via_template` field plumbing exactly
/// (minus the final `encode_ansi` step, which this module deliberately never
/// calls).
fn template_ctx<'a>(
    resolver: &'a dyn VariableResolver,
    ctx: &FormatterRenderContext<'a>,
) -> TemplateRenderContext<'a> {
    TemplateRenderContext::new(resolver)
        .with_prev_colors(ctx.prev_fg.clone(), ctx.prev_bg.clone())
        .with_palette(ctx.palette.clone())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(clippy::missing_errors_doc)]

    use super::*;
    use crate::config::types::{Icon, PaletteName, ThemeName};
    use crate::git::{RepositoryState, RepositoryStatus};
    use crate::template::{Palette, Style as TStyle};

    fn make_facts() -> PreviewFacts {
        PreviewFacts {
            repository_status: None,
            languages: Vec::new(),
            hostname: "host.example.com".to_owned(),
            username: "tester".to_owned(),
            sample_duration_ms: 128,
            sample_character_success: true,
            sample_is_ssh: true,
        }
    }

    /// Bundles the git/language toggles behind one param — `.clippy.toml`
    /// caps functions at 1 bool parameter (`max-fn-params-bools`).
    #[derive(Clone, Copy)]
    struct GitLanguageFlags {
        git: bool,
        language: bool,
    }

    fn make_config(enabled_segments: &[&str], flags: GitLanguageFlags) -> Config {
        let mut config = Config::default();
        config.ui.theme = ThemeName::new("default".to_owned()).expect("valid theme name");
        config.ui.palette = PaletteName::new("default".to_owned()).expect("valid palette name");
        config.git.enabled = flags.git;
        config.language.enabled = flags.language;
        config.ui.enabled_segments = enabled_segments.iter().map(|s| (*s).to_owned()).collect();
        config
    }

    /// Fixed, deterministic stand-in for `WizardState::new`'s real discovery.
    ///
    /// See `state.rs`'s own `FIXED_THEMES`/`FIXED_PALETTES`/`FIXED_SEGMENTS`
    /// for the rationale (#607). Kept in sync with those exact values.
    /// `FIXED_SEGMENTS`'s `BUILTIN_ORDER` ordering matters here: several
    /// tests below assert on chain position (e.g. which segment renders the
    /// opening/closing cap).
    fn test_state(config: Config) -> WizardState {
        WizardState::from_parts(
            config,
            vec![
                "default".to_owned(),
                "starship".to_owned(),
                "text".to_owned(),
            ],
            vec![
                "catppuccin-frappe".to_owned(),
                "catppuccin-latte".to_owned(),
                "catppuccin-macchiato".to_owned(),
                "catppuccin-mocha".to_owned(),
                "default".to_owned(),
                "gruvbox-dark-medium".to_owned(),
                "nord".to_owned(),
                "starship".to_owned(),
            ],
            vec![
                "clock".to_owned(),
                "duration".to_owned(),
                "language".to_owned(),
                "directory".to_owned(),
                "git".to_owned(),
                "status".to_owned(),
            ],
        )
    }

    fn precedence_status() -> RepositoryStatus {
        RepositoryStatus {
            branch: "main".to_owned(),
            ahead: 0,
            behind: 0,
            ahead_capped: false,
            behind_capped: false,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            conflicts: 0,
            state: RepositoryState::Clean,
            stash_count: 0,
            detached: false,
            rebase_progress: None,
        }
    }

    // -- Part A: color/attr adapter table tests --
    // One assertion per `TemplateColor` variant and per `Attr` variant, so a
    // future missing/incorrect-arm regression fails on the specific variant
    // rather than a loose loop hiding which one broke.

    #[test]
    fn to_ratatui_color_maps_named_variant() {
        assert_eq!(
            to_ratatui_color(&TemplateColor::Named("green".to_owned())),
            Some(RatatuiColor::Green)
        );
    }

    #[test]
    fn to_ratatui_color_maps_rgb_variant() {
        assert_eq!(
            to_ratatui_color(&TemplateColor::Rgb {
                r: 10,
                g: 20,
                b: 30
            }),
            Some(RatatuiColor::Rgb(10, 20, 30))
        );
    }

    #[test]
    fn to_ratatui_color_maps_ansi256_variant() {
        assert_eq!(
            to_ratatui_color(&TemplateColor::Ansi256(42)),
            Some(RatatuiColor::Indexed(42))
        );
    }

    #[test]
    fn to_ratatui_color_treats_palette_variant_as_unreachable_none() {
        assert_eq!(
            to_ratatui_color(&TemplateColor::Palette("accent".to_owned())),
            None
        );
    }

    #[test]
    fn to_ratatui_color_treats_prev_fg_variant_as_unreachable_none() {
        assert_eq!(to_ratatui_color(&TemplateColor::PrevFg), None);
    }

    #[test]
    fn to_ratatui_color_treats_prev_bg_variant_as_unreachable_none() {
        assert_eq!(to_ratatui_color(&TemplateColor::PrevBg), None);
    }

    #[test]
    fn named_to_ratatui_white_is_gray_not_bright() {
        // gpy's "white" is ANSI 37 (see style_encoder.rs's "white" => 37),
        // which ratatui spells `Gray`, not `White` (`White` is ratatui's
        // name for ANSI 97 / bright white). Regression test for the
        // easy-to-get-backwards mapping documented on `named_to_ratatui`.
        assert_eq!(named_to_ratatui("white"), Some(RatatuiColor::Gray));
    }

    #[test]
    fn named_to_ratatui_bright_white_is_white() {
        assert_eq!(named_to_ratatui("bright-white"), Some(RatatuiColor::White));
    }

    #[test]
    fn named_to_ratatui_default_resets() {
        assert_eq!(named_to_ratatui("default"), Some(RatatuiColor::Reset));
    }

    #[test]
    fn named_to_ratatui_unknown_name_is_none() {
        assert_eq!(named_to_ratatui("not-a-real-color"), None);
    }

    #[test]
    fn to_ratatui_modifier_maps_bold() {
        assert_eq!(to_ratatui_modifier(Attr::Bold), Modifier::BOLD);
    }

    #[test]
    fn to_ratatui_modifier_maps_italic() {
        assert_eq!(to_ratatui_modifier(Attr::Italic), Modifier::ITALIC);
    }

    #[test]
    fn to_ratatui_modifier_maps_underline() {
        assert_eq!(to_ratatui_modifier(Attr::Underline), Modifier::UNDERLINED);
    }

    #[test]
    fn to_ratatui_modifier_maps_dimmed() {
        assert_eq!(to_ratatui_modifier(Attr::Dimmed), Modifier::DIM);
    }

    #[test]
    fn to_ratatui_modifier_maps_inverted() {
        assert_eq!(to_ratatui_modifier(Attr::Inverted), Modifier::REVERSED);
    }

    #[test]
    fn to_ratatui_modifier_maps_blink() {
        assert_eq!(to_ratatui_modifier(Attr::Blink), Modifier::SLOW_BLINK);
    }

    #[test]
    fn to_ratatui_modifier_maps_hidden() {
        assert_eq!(to_ratatui_modifier(Attr::Hidden), Modifier::HIDDEN);
    }

    #[test]
    fn to_ratatui_modifier_maps_strikethrough() {
        assert_eq!(
            to_ratatui_modifier(Attr::Strikethrough),
            Modifier::CROSSED_OUT
        );
    }

    #[test]
    fn to_ratatui_span_preserves_text_and_style() {
        let span = TemplateSpan {
            text: "main".to_owned(),
            style: TStyle {
                fg: Some(TemplateColor::Named("green".to_owned())),
                bg: Some(TemplateColor::Rgb { r: 1, g: 2, b: 3 }),
                attrs: vec![Attr::Bold],
            },
            kind: SpanKind::Text,
        };

        let got = to_ratatui_span(&span);

        assert_eq!(got.content, "main");
        assert_eq!(got.style.fg, Some(RatatuiColor::Green));
        assert_eq!(got.style.bg, Some(RatatuiColor::Rgb(1, 2, 3)));
        assert!(got.style.add_modifier.contains(Modifier::BOLD));
    }

    // -- Part B: per-segment composition tests --

    #[test]
    fn render_segment_git_uses_state_precedence() {
        let config = make_config(
            &[],
            GitLanguageFlags {
                git: false,
                language: false,
            },
        );
        let mut theme = ThemeConfig::default();
        theme.segments.git.format = Some("[$branch]($style)".to_owned());

        let mut status = precedence_status();
        // Conflicts set alongside other non-zero fields to prove precedence,
        // not merely "conflicts != 0", drives the resolved state.
        status.conflicts = 1;
        status.state = RepositoryState::Rebasing;
        status.staged = 1;
        status.untracked = 1;

        let facts = PreviewFacts {
            repository_status: Some(status.clone()),
            ..make_facts()
        };

        assert_eq!(GitState::from_status(&status), GitState::Conflicts);

        let ctx = FormatterRenderContext::new(&config, &theme, SegmentPosition::LAST);
        let spans = render_segment("git", &config, &theme, &facts, &ctx)
            .expect("git segment should render when format + status are present");
        assert!(
            spans.iter().any(|s| s.text.contains("main")),
            "expected branch text in rendered spans, got {spans:?}"
        );
    }

    fn make_lang(name: &str, version: Option<&str>, color: &str) -> crate::ipc::LanguageInfo {
        crate::ipc::LanguageInfo {
            name: name.to_owned(),
            version: version.map(str::to_owned),
            color: crate::config::types::ColorSpec::new(color).expect("valid color spec"),
        }
    }

    #[test]
    fn render_segment_language_uses_selected_display_symbol() {
        let mut config = make_config(
            &[],
            GitLanguageFlags {
                git: false,
                language: true,
            },
        );
        config.language.display = crate::config::types::LanguageDisplay::Text;
        let mut theme = ThemeConfig::default();
        theme.segments.language.format = Some("[$symbol]($style)".to_owned());

        let facts = PreviewFacts {
            languages: vec![make_lang("Rust", Some("1.75.0"), "red")],
            ..make_facts()
        };

        let ctx = FormatterRenderContext::new(&config, &theme, SegmentPosition::LAST);
        let spans = render_segment("language", &config, &theme, &facts, &ctx)
            .expect("language segment should render when format + a language are present");

        assert!(
            spans.iter().any(|s| s.text.contains("Rust")),
            "text display mode should render the language name, got {spans:?}"
        );
    }

    #[test]
    fn render_segment_language_drops_versionless_language_when_show_versions_on() {
        let config = make_config(
            &[],
            GitLanguageFlags {
                git: false,
                language: true,
            },
        );
        assert!(
            config.language.show_versions,
            "test relies on show_versions defaulting to true"
        );
        let mut theme = ThemeConfig::default();
        theme.segments.language.format = Some("[$symbol]($style)".to_owned());

        let facts = PreviewFacts {
            languages: vec![make_lang("Nix", None, "blue")],
            ..make_facts()
        };

        let ctx = FormatterRenderContext::new(&config, &theme, SegmentPosition::LAST);
        let result = render_segment("language", &config, &theme, &facts, &ctx);

        assert!(
            result.is_none(),
            "show_versions=true must drop a versionless language from the preview, matching the real prompt's select_languages"
        );
    }

    #[test]
    fn render_segment_language_keeps_versionless_language_when_show_versions_off() {
        let mut config = make_config(
            &[],
            GitLanguageFlags {
                git: false,
                language: true,
            },
        );
        config.language.show_versions = false;
        let mut theme = ThemeConfig::default();
        theme.segments.language.format = Some("[$symbol]($style)".to_owned());

        let facts = PreviewFacts {
            languages: vec![make_lang("Nix", None, "blue")],
            ..make_facts()
        };

        let ctx = FormatterRenderContext::new(&config, &theme, SegmentPosition::LAST);
        let spans = render_segment("language", &config, &theme, &facts, &ctx);

        assert!(
            spans.is_some(),
            "show_versions=false must keep a versionless language in the preview"
        );
    }

    #[test]
    fn render_segment_language_hides_version_text_when_show_versions_off() {
        // Regression coverage for the reported wizard bug: toggling
        // language.show_versions had no visible effect in the preview
        // whenever the detected language already had a version (e.g. this
        // repo's own Rust toolchain) — `select_languages` only ever dropped
        // *versionless* languages, it never suppressed already-present
        // version text. See `LanguageResolver::resolve("version")`.
        let mut config = make_config(
            &[],
            GitLanguageFlags {
                git: false,
                language: true,
            },
        );
        config.language.show_versions = false;
        let mut theme = ThemeConfig::default();
        theme.segments.language.format = Some("[$symbol $version]($style)".to_owned());

        let facts = PreviewFacts {
            languages: vec![make_lang("Rust", Some("1.75.0"), "red")],
            ..make_facts()
        };

        let ctx = FormatterRenderContext::new(&config, &theme, SegmentPosition::LAST);
        let spans = render_segment("language", &config, &theme, &facts, &ctx)
            .expect("a versioned language with show_versions=false should still render");

        assert!(
            spans.iter().all(|s| !s.text.contains("1.75.0")),
            "show_versions=false must hide version text even when a version was detected, got {spans:?}"
        );
    }

    #[test]
    fn render_segment_returns_none_when_no_format_configured() {
        let config = make_config(
            &[],
            GitLanguageFlags {
                git: false,
                language: false,
            },
        );
        let theme = ThemeConfig::default();
        assert!(theme.segments.directory.format.is_none());

        let facts = make_facts();
        let ctx = FormatterRenderContext::new(&config, &theme, SegmentPosition::LAST);

        let result = render_segment("directory", &config, &theme, &facts, &ctx);

        assert!(
            result.is_none(),
            "no configured format must yield None, not empty spans"
        );
    }

    #[test]
    fn render_preview_line_threads_prev_colors_between_segments() {
        let config = make_config(
            &["duration"],
            GitLanguageFlags {
                git: false,
                language: false,
            },
        );
        let state = test_state(config.clone());

        let mut theme = ThemeConfig::default();
        // Duration's background is a known, fixed color...
        theme.segments.duration.format = Some("[$duration](bg:#010101)".to_owned());
        // ...and the character line's foreground explicitly asks for the
        // previous segment's background, proving the engine's prev_bg
        // threading is actually wired through render_preview_line's
        // per-segment RenderContext construction, not just theoretically
        // supported.
        theme.segments.character.format = Some("[$symbol](fg:prev_bg)".to_owned());

        let palette = Palette::default();
        let facts = make_facts();

        let lines = render_preview_line(&state, &config, &theme, &palette, &facts);

        let fg_colors: Vec<Option<RatatuiColor>> = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|s| s.style.fg)
            .collect();
        assert!(
            fg_colors.contains(&Some(RatatuiColor::Rgb(1, 1, 1))),
            "expected the character line's fg to inherit the duration segment's \
             bg (rgb(1,1,1)) via prev_bg threading, got {fg_colors:?}"
        );
    }

    #[test]
    fn render_preview_line_skips_disabled_segments() {
        let config = make_config(
            &["duration"],
            GitLanguageFlags {
                git: false,
                language: false,
            },
        );
        let mut state = test_state(config.clone());
        state.toggle_segment("duration"); // starts enabled, toggle off

        let mut theme = ThemeConfig::default();
        theme.segments.duration.format = Some("[$duration]($style)".to_owned());

        let palette = Palette::default();
        let facts = make_facts();

        let lines = render_preview_line(&state, &config, &theme, &palette, &facts);

        let text: String = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(
            !text.contains("128ms") && text.is_empty(),
            "disabled duration segment must not appear in the composed line, got {text:?}"
        );
    }

    #[test]
    fn render_preview_line_follows_enabled_segments_order() {
        let config = make_config(
            &["directory", "clock"],
            GitLanguageFlags {
                git: false,
                language: false,
            },
        );
        let state = test_state(config.clone());

        let mut theme = ThemeConfig::default();
        theme.segments.directory.format = Some("[DIRMARK]".to_owned());

        let lines =
            render_preview_line(&state, &config, &theme, &Palette::default(), &make_facts());
        let text: String = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        let dir = text.find("DIRMARK");
        let clock = text.find("9:41");
        assert!(
            dir.is_some() && clock.is_some() && dir < clock,
            "directory must precede clock, got {text:?}"
        );
    }

    #[test]
    fn render_preview_line_only_last_chain_segment_gets_closing_cap() {
        // Real theme formats end with `([$sep_gap]($style))([$sep_close](fg:$bg
        // bg:default))` (see `config/themes/default.toml`'s duration/directory
        // segments) — copy that shape so this exercises the same `$sep_close`
        // resolution production formats do, just with a fixed, unconditional
        // trailing color spec instead of `$bg` (irrelevant to what's under
        // test: which segment gets `is_last = true`).
        let config = make_config(
            &["duration", "directory"],
            GitLanguageFlags {
                git: false,
                language: false,
            },
        );
        let state = test_state(config.clone());

        let mut theme = ThemeConfig::default();
        theme.segments.duration.format =
            Some("[$duration]($style)([$sep_close](fg:black bg:default))".to_owned());
        theme.segments.directory.format =
            Some("[$path]($style)([$sep_close](fg:black bg:default))".to_owned());

        let palette = Palette::default();
        let facts = make_facts();

        let lines = render_preview_line(&state, &config, &theme, &palette, &facts);
        assert_eq!(lines.len(), 1, "no status segment enabled, expect one line");

        let text: String = lines
            .first()
            .expect("asserted above: exactly one line")
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        let duration_idx = text.find('\u{e0bc}').expect(
            "duration (not last in BUILTIN_ORDER) should render the continuation \
             triangle, not the closing cap",
        );
        let directory_idx = text.find('\u{e0b4}').expect(
            "directory (last enabled chain segment) should render the closing \
             half-circle cap",
        );
        assert!(
            duration_idx < directory_idx,
            "continuation glyph must precede the closing-cap glyph, got {text:?}"
        );
        assert!(
            !text.contains("duration\u{e0b4}") && text.matches('\u{e0b4}').count() == 1,
            "only the last chain segment may emit the closing cap, got {text:?}"
        );
    }

    #[test]
    fn render_preview_line_only_first_chain_segment_suppresses_opening_cap() {
        // Mirrors render_preview_line_only_last_chain_segment_gets_closing_cap,
        // but for the opening side: whichever segment ends up first (by chain
        // position, since neither of these two is "clock") must suppress
        // $sep_open, and the second segment must still render it.
        let config = make_config(
            &["duration", "directory"],
            GitLanguageFlags {
                git: false,
                language: false,
            },
        );
        let state = test_state(config.clone());

        let mut theme = ThemeConfig::default();
        theme.segments.duration.format =
            Some("([$sep_open](fg:black bg:default))[$duration]($style)".to_owned());
        theme.segments.directory.format =
            Some("([$sep_open](fg:black bg:default))[$path]($style)".to_owned());

        let palette = Palette::default();
        let facts = make_facts();

        let lines = render_preview_line(&state, &config, &theme, &palette, &facts);
        assert_eq!(lines.len(), 1, "no status segment enabled, expect one line");

        let text: String = lines
            .first()
            .expect("asserted above: exactly one line")
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(
            text.matches('\u{e0ba}').count(),
            1,
            "only the non-first chain segment may emit the opening cap, got {text:?}"
        );
        // Default display mode is Basename, so SAMPLE_CWD ("~/dev/gpy") renders as
        // just "gpy".
        let path_idx = text.find("gpy").expect("directory path text present");
        let cap_idx = text.find('\u{e0ba}').expect("opening cap glyph present");
        assert!(
            cap_idx < path_idx,
            "directory (not first) should render its opening cap before its own \
             content, got {text:?}"
        );
    }

    #[test]
    fn render_preview_line_first_segment_position_not_identity_determines_cap() {
        // Same duration/directory formats as
        // render_preview_line_only_first_chain_segment_suppresses_opening_cap,
        // but with clock also enabled ahead of them in BUILTIN_ORDER. Clock
        // renders its own demo spans (see clock_demo_spans) and so occupies
        // chain index 0 itself, meaning duration is no longer the first
        // *rendered* entry — it must therefore render its opening cap here
        // (unlike the other test, where duration WAS first and suppressed
        // it). The same segment (duration) behaving differently purely
        // because of what renders before it — never because of a hardcoded
        // "duration"/"clock" identity check — is exactly what proves the
        // suppression is chain-position-based.
        let config = make_config(
            &["clock", "duration", "directory"],
            GitLanguageFlags {
                git: false,
                language: false,
            },
        );
        let state = test_state(config.clone());

        let mut theme = ThemeConfig::default();
        theme.segments.duration.format =
            Some("([$sep_open](fg:black bg:default))[$duration]($style)".to_owned());
        theme.segments.directory.format =
            Some("([$sep_open](fg:black bg:default))[$path]($style)".to_owned());

        let palette = Palette::default();
        let facts = make_facts();

        let lines = render_preview_line(&state, &config, &theme, &palette, &facts);
        let text: String = lines
            .first()
            .expect("render_preview_line always produces at least the chain line")
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(
            text.matches('\u{e0ba}').count(),
            2,
            "with clock occupying the first chain slot, both duration and \
             directory are non-first and must render their opening caps, got {text:?}"
        );
    }

    #[test]
    fn render_segment_clock_renders_fixed_demo_time() {
        let config = make_config(
            &[],
            GitLanguageFlags {
                git: false,
                language: false,
            },
        );
        let theme = ThemeConfig::default();
        let facts = make_facts();
        let ctx = FormatterRenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

        let spans = render_segment("clock", &config, &theme, &facts, &ctx)
            .expect("clock should always render a demo, unlike hostname/username with no format");
        let text: String = spans.iter().map(|s| s.text.as_str()).collect();
        assert!(
            text.contains("9:41"),
            "expected the fixed demo time in clock output, got {text:?}"
        );
    }

    /// #826: the hostname icon is an SSH-only symbol; the preview draws the
    /// segment as it looks over SSH, so the icon shows, and a local sample
    /// drops it exactly as the real prompt does.
    #[test]
    fn render_segment_hostname_icon_follows_sample_ssh_state() {
        let config = make_config(
            &[],
            GitLanguageFlags {
                git: false,
                language: false,
            },
        );
        let mut theme = ThemeConfig::default();
        theme.segments.hostname.format = Some("[$symbol$hostname](green)".to_owned());
        theme.segments.hostname.icon = Some(Icon::new("🌐 ").expect("valid icon"));
        let ctx = FormatterRenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
        let text_for = |facts: &PreviewFacts| -> String {
            render_segment("hostname", &config, &theme, facts, &ctx)
                .expect("hostname with a format renders")
                .iter()
                .map(|s| s.text.as_str())
                .collect()
        };

        assert_eq!(text_for(&make_facts()), "🌐 host");
        let local = PreviewFacts {
            sample_is_ssh: false,
            ..make_facts()
        };
        assert_eq!(text_for(&local), "host");
    }

    /// Build a `DelimiterConfig` for the cap-glyph tests below without going
    /// through a full theme TOML file.
    fn delimiter(icon: &str, icon_color: &str, bg_color: &str) -> crate::config::DelimiterConfig {
        crate::config::DelimiterConfig {
            icon: crate::config::types::Icon::new(icon).expect("valid icon"),
            icon_color: crate::config::types::ColorSpec::new(icon_color).expect("valid icon_color"),
            bg_color: crate::config::types::ColorSpec::new(bg_color).expect("valid bg_color"),
        }
    }

    #[test]
    fn clock_demo_spans_omits_cap_when_delimiter_icon_empty() {
        // Regression guard for the *original* (correct) behavior on a flat
        // theme like the `starship` preset, whose delimiter icons are all
        // empty — clock should still render with no cap glyphs there.
        let mut theme = ThemeConfig::default();
        theme.ui.prompt_open = Some(delimiter("", "white", "transparent"));
        theme.ui.prompt_close = Some(delimiter("", "white", "transparent"));
        theme.ui.segment_open = Some(delimiter("", "white", "transparent"));
        theme.ui.segment_close = Some(delimiter("", "white", "transparent"));

        let first_and_last = clock_demo_spans(&theme, IsFirst::Yes, IsLast::Yes);
        let mid_chain = clock_demo_spans(&theme, IsFirst::No, IsLast::No);

        assert_eq!(
            first_and_last.len(),
            1,
            "no caps expected: {first_and_last:?}"
        );
        assert_eq!(
            mid_chain.len(),
            2,
            "no caps expected, only the time span and the mid-chain gap: {mid_chain:?}"
        );
    }

    #[test]
    fn clock_demo_spans_renders_closing_cap_when_last() {
        // Regression test for the bug this fixes: the wizard preview
        // previously never rendered clock's closing cap, even for themes
        // (like the built-in `default` theme) whose `ui.prompt_close` icon
        // is a real glyph, not empty.
        let mut theme = ThemeConfig::default();
        theme.ui.prompt_close = Some(delimiter(")", "white", "transparent"));

        let spans = clock_demo_spans(&theme, IsFirst::No, IsLast::Yes);
        let text: String = spans.iter().map(|s| s.text.as_str()).collect();

        assert!(
            text.ends_with(')'),
            "expected the prompt_close icon as the final glyph, got {text:?}"
        );
    }

    #[test]
    fn clock_demo_spans_renders_segment_close_cap_when_not_last() {
        let mut theme = ThemeConfig::default();
        theme.ui.segment_close = Some(delimiter("]", "white", "transparent"));
        theme.ui.prompt_close = Some(delimiter(")", "white", "transparent"));

        let spans = clock_demo_spans(&theme, IsFirst::No, IsLast::No);
        let text: String = spans.iter().map(|s| s.text.as_str()).collect();

        assert!(
            text.ends_with(']'),
            "not-last clock should use segment_close, not prompt_close, got {text:?}"
        );
    }

    #[test]
    fn clock_demo_spans_renders_opening_cap_when_not_first() {
        let mut theme = ThemeConfig::default();
        theme.ui.segment_open = Some(delimiter("[", "white", "transparent"));
        theme.ui.prompt_open = Some(delimiter("|", "white", "transparent"));

        let spans = clock_demo_spans(&theme, IsFirst::No, IsLast::Yes);
        let text: String = spans.iter().map(|s| s.text.as_str()).collect();

        assert!(
            text.starts_with('['),
            "not-first clock should use segment_open, not prompt_open, got {text:?}"
        );
    }

    #[test]
    fn clock_demo_spans_uses_prompt_open_when_first() {
        let mut theme = ThemeConfig::default();
        theme.ui.segment_open = Some(delimiter("[", "white", "transparent"));
        theme.ui.prompt_open = Some(delimiter("|", "white", "transparent"));

        let spans = clock_demo_spans(&theme, IsFirst::Yes, IsLast::Yes);
        let text: String = spans.iter().map(|s| s.text.as_str()).collect();

        assert!(
            text.starts_with('|'),
            "first clock should use prompt_open, not segment_open, got {text:?}"
        );
    }

    #[test]
    fn clock_demo_spans_inserts_gap_space_before_non_last_close_cap() {
        let mut theme = ThemeConfig::default();
        theme.ui.segment_close = Some(delimiter("]", "white", "transparent"));

        let spans = clock_demo_spans(&theme, IsFirst::Yes, IsLast::No);

        assert!(
            spans.iter().any(|s| s.text == " " && s.style.fg.is_none()),
            "expected a same-background gap span before the closing cap: {spans:?}"
        );
        let last = spans.last().expect("at least the time span");
        assert_eq!(last.text, "]");
    }

    #[test]
    fn clock_demo_spans_renders_closing_cap_on_the_real_default_theme() {
        // End-to-end guard using the actual embedded `default` theme (not a
        // hand-built `ThemeConfig::default()` stub): its `ui.prompt_close`
        // icon is the Nerd Font powerline glyph U+E0B4 (confirmed against
        // `config/themes/default.toml`), so a clock segment that is both
        // first and last must render it as its closing cap.
        let theme = crate::theme::ThemeManager::new("default")
            .expect("default theme should always load")
            .get();

        let spans = clock_demo_spans(&theme, IsFirst::Yes, IsLast::Yes);
        let text: String = spans.iter().map(|s| s.text.as_str()).collect();

        assert!(
            text.ends_with('\u{e0b4}'),
            "expected the default theme's powerline closing cap, got {text:?}"
        );
    }

    #[test]
    fn resolve_clock_cap_color_maps_match_bg_and_match_text_and_transparent() {
        assert_eq!(
            resolve_clock_cap_color("match_bg", "black", "white"),
            Some(TemplateColor::Named("black".to_owned()))
        );
        assert_eq!(
            resolve_clock_cap_color("match_text", "black", "white"),
            Some(TemplateColor::Named("white".to_owned()))
        );
        assert_eq!(
            resolve_clock_cap_color("transparent", "black", "white"),
            None
        );
        assert_eq!(
            resolve_clock_cap_color("red", "black", "white"),
            Some(TemplateColor::Named("red".to_owned()))
        );
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn preview_always_renders_character_line() {
        let config = make_config(
            &["duration"],
            GitLanguageFlags {
                git: false,
                language: false,
            },
        );
        let state = test_state(config.clone());

        let mut theme = ThemeConfig::default();
        theme.segments.duration.format = Some("[$duration]($style)".to_owned());
        theme.segments.character.format = Some("[$symbol]($style)".to_owned());

        let lines =
            render_preview_line(&state, &config, &theme, &Palette::default(), &make_facts());

        assert_eq!(lines.len(), 2, "chain line plus character line: {lines:?}");
        let last = lines.last().expect("asserted above: two lines");
        assert!(
            line_text(last).contains('❯'),
            "last line must be the character line, got {:?}",
            line_text(last)
        );
    }

    #[test]
    fn preview_renders_status_as_chain_pill() {
        let config = make_config(
            &["duration", "status"],
            GitLanguageFlags {
                git: false,
                language: false,
            },
        );
        let state = test_state(config.clone());

        let mut theme = ThemeConfig::default();
        theme.segments.duration.format = Some("[$duration]($style)".to_owned());
        theme.segments.character.format = Some("[$symbol]($style)".to_owned());
        theme.segments.status.ok_icon = Some(Icon::new("✔").expect("valid icon"));

        let lines =
            render_preview_line(&state, &config, &theme, &Palette::default(), &make_facts());

        assert_eq!(lines.len(), 2, "chain line plus character line: {lines:?}");
        let chain_text = line_text(lines.first().expect("asserted above: two lines"));
        let char_text = line_text(lines.get(1_usize).expect("asserted above: two lines"));
        assert!(
            chain_text.contains("0.128s"),
            "chain must still contain duration, got {chain_text:?}"
        );
        assert!(
            chain_text.contains('✔') && !chain_text.contains('❯'),
            "status must be a pill in the chain, not the character: {chain_text:?}"
        );
        assert!(
            char_text.contains('❯'),
            "character line must follow the chain, got {char_text:?}"
        );
    }

    /// #695: the preview matches the live prompt with `ui.show_icons` off.
    ///
    /// The live prompt draws no powerline caps then. This covers both the
    /// template-resolver caps (duration/directory) and the theme-delimiter
    /// caps of the shell-drawn clock pill.
    #[test]
    fn preview_draws_no_private_use_caps_when_icons_are_off() {
        let has_private_use =
            |text: &str| text.chars().any(|c| ('\u{e000}'..='\u{f8ff}').contains(&c));
        let theme = crate::theme::ThemeManager::new("default")
            .expect("default theme should always load")
            .get();
        let render = |show_icons: bool| {
            let mut config = make_config(
                &["clock", "duration", "directory"],
                GitLanguageFlags {
                    git: false,
                    language: false,
                },
            );
            config.ui.show_icons = show_icons;
            let state = test_state(config.clone());
            render_preview_line(&state, &config, &theme, &Palette::default(), &make_facts())
                .iter()
                .map(line_text)
                .collect::<String>()
        };

        let on = render(true);
        assert!(
            on.contains('\u{e0ba}') && on.contains('\u{e0bc}') && on.contains('\u{e0b4}'),
            "show_icons=true control must draw the caps, got {on:?}"
        );
        let off = render(false);
        assert!(
            !has_private_use(&off),
            "show_icons=false preview leaked a private-use glyph: {off:?}"
        );
        assert!(
            off.contains("0.128s") && off.contains("9:41"),
            "segments themselves must still render, got {off:?}"
        );
    }

    #[test]
    fn preview_status_pill_uses_fail_variant_and_theme_fallback_icon() {
        let config = make_config(
            &["status"],
            GitLanguageFlags {
                git: false,
                language: false,
            },
        );
        let state = test_state(config.clone());
        let theme = ThemeConfig::default();
        let mut facts = make_facts();

        facts.sample_character_success = false;
        let fail_lines = render_preview_line(&state, &config, &theme, &Palette::default(), &facts);
        let fail_text = line_text(fail_lines.first().expect("chain line"));
        assert!(fail_text.contains('✖'), "fail pill, got {fail_text:?}");

        facts.sample_character_success = true;
        let ok_lines = render_preview_line(&state, &config, &theme, &Palette::default(), &facts);
        let ok_text = line_text(ok_lines.first().expect("chain line"));
        assert!(ok_text.contains('✔'), "ok pill, got {ok_text:?}");
    }
}
