//! `ratatui` widget composition for the wizard frame.
//!
//! Pure with respect to
//! terminal I/O (takes a `Frame` to draw into, never touches stdout/stdin
//! directly) but not unit-tested at the pixel level — `ratatui::backend::
//! TestBackend` is available if a snapshot-style smoke test earns its keep,
//! but none is currently justified; the interactive behavior worth testing
//! lives in `keys.rs`/`state.rs`/`detail.rs` and is already covered there.
//! `body_layout`'s width-based split *is* unit-tested below since it's pure
//! geometry (issue #400's narrow-terminal degradation).

use super::keys::InputMode;
use super::preview::to_ratatui_color;
use super::state::{Section, WizardState};
use crate::template::Palette;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color as RatatuiColor, Modifier, Style};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};

/// Chrome colors derived from the wizard's currently *selected* palette.
///
/// From `WizardState::selected_palette`, resolved once per frame in `mod.rs`'s
/// `refresh_theme_palette_cache` — the same palette the live preview renders with.
///
/// Every field falls back to `None` (the terminal's own default
/// rendering) when the active palette has no entry for that name — the
/// 8 builtin palettes (`config/palettes/*.toml`) all define `blue`/`red`/
/// `yellow`, but a user-authored custom palette is not required to.
#[derive(Clone, Copy, Default)]
pub struct WizardColors {
    /// Focused section/panel borders and the cursor highlight row —
    /// the palette's `blue`.
    accent: Option<RatatuiColor>,
    /// The discard-changes confirmation prompt — the palette's `red`.
    danger: Option<RatatuiColor>,
    /// The active-filter status line — the palette's `yellow`.
    warning: Option<RatatuiColor>,
}

impl WizardColors {
    /// Resolve chrome colors from `palette`.
    #[must_use]
    pub fn from_palette(palette: &Palette) -> Self {
        Self {
            accent: palette_named(palette, "blue"),
            danger: palette_named(palette, "red"),
            warning: palette_named(palette, "yellow"),
        }
    }
}

/// Look up `name` in `palette` and convert it to a `ratatui` color, or
/// `None` if the palette has no such entry.
fn palette_named(palette: &Palette, name: &str) -> Option<RatatuiColor> {
    palette.get(name).as_ref().and_then(to_ratatui_color)
}

/// A border style: bold when `focused`, additionally tinted with `accent`
/// when it resolved to a color.
fn border_style(focused: bool, accent: Option<RatatuiColor>) -> Style {
    let mut style = Style::default();
    if focused {
        style = style.add_modifier(Modifier::BOLD);
    }
    if let Some(color) = accent {
        style = style.fg(color);
    }
    style
}

/// Body width below which the detail panel moves below the option lists
/// instead of beside them — side-by-side below this width would crush both
/// panes to unreadable widths (issue #400).
const NARROW_WIDTH_THRESHOLD: u16 = 80;

/// Bundles `draw`'s non-frame parameters.
///
/// `clippy.toml` caps functions at 5
/// arguments (`too-many-arguments-threshold`), and `draw` needs more inputs
/// than that once the detail panel (issue #400) joined `preview`.
pub struct DrawArgs<'a> {
    pub state: &'a WizardState,
    pub mode: InputMode,
    pub preview: &'a [Line<'static>],
    pub detail: &'a [Line<'static>],
    pub colors: WizardColors,
}

/// Draw one wizard frame.
///
/// A master/detail body (Theme / Palette / Segments
/// option lists plus a detail panel describing the highlighted item), the
/// live preview line, and a compact footer (or the discard-changes prompt
/// when `args.mode == InputMode::ConfirmQuit`). When `args.mode ==
/// InputMode::Help`, a contextual help overlay is drawn on top, scoped to
/// `args.state.focus()`.
pub fn draw(frame: &mut Frame, args: &DrawArgs) {
    let area = frame.area();
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(9),    // master/detail body
            Constraint::Length(4), // preview (chain line + status line)
            Constraint::Length(1), // footer
        ])
        .split(area);
    let [body_area, preview_area, footer_area] = *outer else {
        // `Layout::split` always returns one `Rect` per constraint above (3),
        // so this is unreachable in practice; fall back to the full frame
        // area rather than panicking if that guarantee ever changes.
        render_sections(frame, area, args.state, args.colors);
        return;
    };

    let (master_area, detail_area) = body_layout(body_area);
    render_sections(frame, master_area, args.state, args.colors);
    render_detail_panel(frame, detail_area, args.detail);
    render_preview_panel(frame, preview_area, args.state, args.preview, args.colors);
    render_footer(
        frame,
        footer_area,
        &FooterArgs {
            mode: args.mode,
            filter_query: args.state.filter_query(),
            colors: args.colors,
        },
    );

    if args.mode == InputMode::Help {
        render_help_overlay(frame, area, args.state.focus(), args.colors);
    }
}

/// Draw the Theme / Palette / Segments master lists, stacked vertically
/// within `area`.
///
/// All three sections are narrowed by the same shared
/// filter query (see `WizardState::filtered_indices`) — typing while any
/// one section has focus narrows all of them at once.
fn render_sections(frame: &mut Frame, area: Rect, state: &WizardState, colors: WizardColors) {
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Min(3), Constraint::Min(3)])
        .split(area);
    let [theme_area, palette_area, segments_area] = *sections else {
        // `Layout::split` always returns one `Rect` per constraint above (3),
        // so this is unreachable in practice; skip rendering rather than
        // panicking if that guarantee ever changes.
        return;
    };

    render_one_section(
        frame,
        theme_area,
        state,
        &SectionSource {
            section: Section::Theme,
            title: "Theme",
            items: state.available_themes(),
            raw_cursor: state.theme_cursor(),
            is_selected: &|name| name == state.selected_theme(),
        },
        colors,
    );

    render_one_section(
        frame,
        palette_area,
        state,
        &SectionSource {
            section: Section::Palette,
            title: "Palette",
            items: state.available_palettes(),
            raw_cursor: state.palette_cursor(),
            is_selected: &|name| name == state.selected_palette(),
        },
        colors,
    );

    render_one_section(
        frame,
        segments_area,
        state,
        &SectionSource {
            section: Section::Segments,
            title: "Segments (space to toggle)",
            items: state.available_segments(),
            raw_cursor: state.segment_cursor(),
            is_selected: &|name| state.is_segment_enabled(name),
        },
        colors,
    );
}

/// One master-list section's data source, bundled so `render_one_section`
/// stays under `clippy.toml`'s `too-many-arguments-threshold`.
struct SectionSource<'a> {
    section: Section,
    title: &'a str,
    items: &'a [String],
    /// Index into `items` (the *unfiltered* list) the cursor currently sits
    /// on — see `narrow_to_filtered`.
    raw_cursor: usize,
    is_selected: &'a dyn Fn(&str) -> bool,
}

/// Narrow `source` to its section's current filter (see
/// `WizardState::filtered_indices`) and render it into `area`.
fn render_one_section(
    frame: &mut Frame,
    area: Rect,
    state: &WizardState,
    source: &SectionSource,
    colors: WizardColors,
) {
    let indices = state.filtered_indices(source.section);
    let (items, selected, cursor_index) = narrow_to_filtered(
        source.items,
        source.is_selected,
        &indices,
        source.raw_cursor,
    );
    render_section(
        frame,
        area,
        &SectionSpec {
            title: source.title,
            items: &items,
            selected: &selected,
            cursor_index,
            focused: state.focus() == source.section,
            filter_query: state.filter_query(),
            accent: colors.accent,
        },
    );
}

/// `narrow_to_filtered`'s return value: the narrowed items, their aligned
/// `[x]`/`( )` selection flags, and the cursor's position within the
/// narrowed list.
///
/// Named to keep the signature under `clippy::type_complexity`
/// (`(Vec<String>, Vec<bool>, Option<usize>)` on its own trips the lint).
type FilteredSection = (Vec<String>, Vec<bool>, Option<usize>);

/// Narrow `items`/`is_selected` down to `indices`.
///
/// See `WizardState::filtered_indices`, and translate `raw_cursor` (an
/// index into the
/// *unfiltered* `items`) into its position within the narrowed list —
/// `None` when `indices` is empty (the no-matches case `render_section`
/// renders a placeholder for). Indices with no matching item (which should
/// not happen in practice, since `indices` is derived from `items`) are
/// skipped rather than panicking.
fn narrow_to_filtered(
    items: &[String],
    is_selected: impl Fn(&str) -> bool,
    indices: &[usize],
    raw_cursor: usize,
) -> FilteredSection {
    let filtered_items: Vec<String> = indices
        .iter()
        .filter_map(|&i| items.get(i).cloned())
        .collect();
    let filtered_selected: Vec<bool> = indices
        .iter()
        .filter_map(|&i| items.get(i).map(|item| is_selected(item)))
        .collect();
    let cursor_index = indices.iter().position(|&i| i == raw_cursor);
    (filtered_items, filtered_selected, cursor_index)
}

/// The detail panel: a bordered paragraph describing whichever item
/// currently has the cursor in the focused master list (issue #400).
fn render_detail_panel(frame: &mut Frame, area: Rect, detail: &[Line<'static>]) {
    frame.render_widget(
        Paragraph::new(Text::from(detail.to_vec()))
            .block(Block::default().title("Detail").borders(Borders::ALL))
            .wrap(Wrap { trim: true }),
        area,
    );
}

/// The live prompt preview block.
///
/// Its border is tinted with `colors.accent` unconditionally (not just when
/// focused) — this is the wizard's "hero" panel demonstrating the selected
/// palette, so it stays visibly tied to that palette regardless of which
/// section currently has keyboard focus.
fn render_preview_panel(
    frame: &mut Frame,
    area: Rect,
    state: &WizardState,
    preview: &[Line<'static>],
    colors: WizardColors,
) {
    frame.render_widget(
        Paragraph::new(Text::from(preview.to_vec())).block(
            Block::default()
                .title(preview_title(state))
                .borders(Borders::ALL)
                .border_style(border_style(false, colors.accent)),
        ),
        area,
    );
}

/// Bundles `render_footer`'s parameters.
///
/// `clippy.toml` caps functions at 5 arguments
/// (`too-many-arguments-threshold`), and `render_footer` was already at 5
/// with `frame`/`area`/`mode`/`filter_query`/`colors`.
struct FooterArgs<'a> {
    mode: InputMode,
    /// The live filter query text — only shown while `mode ==
    /// InputMode::Filter`, but always passed so `render_footer` doesn't need
    /// its own way back to `WizardState::filter_query`.
    filter_query: &'a str,
    colors: WizardColors,
}

/// The single-line footer, in priority order.
///
/// The discard-changes prompt (tinted with `colors.danger`), the help-overlay
/// close hint, the filter-mode status (tinted with `colors.warning`; live
/// query text plus explicit clear/exit guidance — this is the primary "you
/// are filtering right now" signal, since the same query narrows all three
/// sections at once), or the normal key-binding summary.
fn render_footer(frame: &mut Frame, area: Rect, args: &FooterArgs) {
    let (footer, style) = match args.mode {
        InputMode::ConfirmQuit => (
            "Discard changes and quit? [y/N]".to_owned(),
            args.colors
                .danger
                .map_or_else(Style::default, |color| Style::default().fg(color)),
        ),
        InputMode::Help => ("[?/esc] close help".to_owned(), Style::default()),
        InputMode::Filter => (
            format!(
                "FILTERING: /{}_  [tab] switch section  [enter/space] activate  [esc] clear filter",
                args.filter_query
            ),
            args.colors
                .warning
                .map_or_else(Style::default, |color| Style::default().fg(color)),
        ),
        InputMode::Normal => ("[?] help  [s] save  [q] quit".to_owned(), Style::default()),
    };
    frame.render_widget(Paragraph::new(footer).style(style), area);
}

/// Split a body region into master (option lists) and detail panel areas.
///
/// Side-by-side when `area` is wide enough for both to stay readable;
/// stacked (detail panel below the lists, full width) in narrow terminals so
/// neither pane gets crushed to unreadable widths (issue #400).
fn body_layout(area: Rect) -> (Rect, Rect) {
    if area.width < NARROW_WIDTH_THRESHOLD {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(6), Constraint::Length(7)])
            .split(area);
        let [master, detail] = *rows else {
            // `Layout::split` always returns one `Rect` per constraint above
            // (2); fall back to stacking the full area onto itself rather
            // than panicking if that guarantee ever changes.
            return (area, area);
        };
        (master, detail)
    } else {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(area);
        let [master, detail] = *cols else {
            return (area, area);
        };
        (master, detail)
    }
}

/// One master-list section's render inputs, bundled behind a struct so
/// `render_section` stays under `clippy.toml`'s `too-many-arguments-threshold`.
///
/// `items`/`selected` are already narrowed to the section's filtered view
/// (see `narrow_to_filtered`), not the full `available_*` list.
struct SectionSpec<'a> {
    title: &'a str,
    items: &'a [String],
    /// `[x]`/`( )` marker per item, aligned by index with `items` (see
    /// `narrow_to_filtered`).
    selected: &'a [bool],
    /// Position of the highlighted item within `items` — `None` when
    /// `items` is empty (the no-matches case).
    cursor_index: Option<usize>,
    focused: bool,
    /// The section's live filter query (empty string when not filtering).
    filter_query: &'a str,
    /// Border/highlight tint from the selected palette — `None` falls back
    /// to the terminal's default color (see [`WizardColors`]).
    accent: Option<RatatuiColor>,
}

/// `title`, suffixed with the live filter query (e.g. `"Theme /cat"`) when
/// non-empty.
///
/// This keeps a section's active filter visible even when a
/// different section currently has keyboard focus (filters persist across
/// Tab — see `WizardState::filter`).
fn section_title(title: &str, filter_query: &str) -> String {
    if filter_query.is_empty() {
        title.to_owned()
    } else {
        format!("{title} /{filter_query}")
    }
}

/// Render one master-list section into `area`.
///
/// A bordered, vertical list of
/// `spec.items`, one per line, each prefixed with a `[x]`/`( )` selection
/// marker (per `spec.selected`), or a `"(no matches)"` placeholder when
/// `spec.items` is empty (the section's filter matched nothing). The
/// cursor row (`spec.cursor_index`) is reverse-video highlighted only when
/// `spec.focused` — a separate visual channel from the `[x]`/`( )` marker
/// since, for the multi-select Segments section, the cursor can sit on an
/// item that isn't (yet) selected. Tinted with `spec.accent` (reverse video
/// swaps it to the highlight row's background) when the selected palette
/// resolved one. Using `ratatui`'s `List`/`ListState`
/// (rather than a hand-rolled wrapped `Paragraph`, the pre-#400 approach)
/// gives each item a stable line and automatic keep-cursor-visible
/// scrolling for lists longer than the visible area.
fn render_section(frame: &mut Frame, area: Rect, spec: &SectionSpec) {
    let title = section_title(spec.title, spec.filter_query);
    let block = section_block(&title, spec.focused, spec.accent);

    if spec.items.is_empty() {
        frame.render_widget(Paragraph::new("(no matches)").block(block), area);
        return;
    }

    let list_items: Vec<ListItem<'static>> = spec
        .items
        .iter()
        .zip(spec.selected)
        .map(|(item, &is_selected)| {
            let marker = if is_selected { "[x]" } else { "( )" };
            ListItem::new(format!("{marker} {item}"))
        })
        .collect();

    let highlight_style = if spec.focused {
        let mut style = Style::default().add_modifier(Modifier::REVERSED);
        if let Some(color) = spec.accent {
            style = style.fg(color);
        }
        style
    } else {
        Style::default()
    };
    let list = List::new(list_items)
        .block(block)
        .highlight_style(highlight_style);

    let mut list_state = ListState::default();
    list_state.select(spec.cursor_index);

    frame.render_stateful_widget(list, area, &mut list_state);
}

/// Draw the `?` contextual help overlay centered over `area`.
///
/// Lists only
/// the key bindings relevant to `focus` plus the section-agnostic ones
/// (section switching, save, quit). Uses `Clear` first since ratatui doesn't
/// blend widgets — without it, the popup's text would overlay whatever
/// glyphs the section widgets already painted into that rect.
fn render_help_overlay(frame: &mut Frame, area: Rect, focus: Section, colors: WizardColors) {
    let popup = centered_rect(70, 60, area);
    frame.render_widget(Clear, popup);

    let mut lines = vec![
        Line::from("[tab] next section   [shift+tab] previous section"),
        Line::from(""),
    ];
    lines.extend(section_help_lines(focus));
    lines.push(Line::from(""));
    lines.push(Line::from("[s] save & exit   [q] quit"));
    lines.push(Line::from("[?] or [esc] close this help"));

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .title("Help")
                    .borders(Borders::ALL)
                    .border_style(border_style(true, colors.accent)),
            )
            .wrap(Wrap { trim: true }),
        popup,
    );
}

/// Key bindings scoped to the currently focused section.
///
/// This is the
/// "contextual" part of the help overlay (issue #393): Segments is a
/// multi-select toggle while Theme/Palette are single-select, so the
/// activate-key description differs per section.
fn section_help_lines(focus: Section) -> Vec<Line<'static>> {
    let (title, activate) = match focus {
        Section::Theme => ("Theme", "[space/enter] select theme"),
        Section::Palette => ("Palette", "[space/enter] select palette"),
        Section::Segments => ("Segments", "[space/enter] toggle segment"),
    };
    let mut lines = vec![
        Line::from(title),
        Line::from("  [↑/↓ or ←/→] move cursor"),
        Line::from(format!("  {activate}")),
        Line::from("  [/] filter (narrows all sections)"),
    ];
    if focus == Section::Segments {
        lines.push(Line::from(
            "  [[ / ]] cycle directory display style (when 'directory' is highlighted)",
        ));
        lines.push(Line::from(
            "  [[ / ]] cycle language icon/text style (when 'language' is highlighted)",
        ));
        lines.push(Line::from(
            "  [v] toggle language show-versions (when 'language' is highlighted)",
        ));
        lines.push(Line::from(
            "  [[ / ]] cycle clock 12h/24h time format (when 'clock' is highlighted)",
        ));
        lines.push(Line::from(
            "  [:] toggle clock show-seconds (when 'clock' is highlighted)",
        ));
        lines.push(Line::from(
            "  [m] toggle duration show-milliseconds (when 'duration' is highlighted)",
        ));
        lines.push(Line::from(
            "  [b]/[a]/[t] toggle git branch / ahead-behind / stash (when 'git' is highlighted)",
        ));
    }
    lines
}

/// Standard ratatui centered-popup helper: carves a `percent_x` × `percent_y`
/// rect out of the middle of `area`.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let margin_y = 100_u16.saturating_sub(percent_y) / 2;
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(margin_y),
            Constraint::Percentage(percent_y),
            Constraint::Percentage(margin_y),
        ])
        .split(area);
    let [_, middle_row, _] = *vertical else {
        // `Layout::split` always returns one `Rect` per constraint above
        // (3); fall back to the full area rather than panicking if that
        // guarantee ever changes.
        return area;
    };

    let margin_x = 100_u16.saturating_sub(percent_x) / 2;
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(margin_x),
            Constraint::Percentage(percent_x),
            Constraint::Percentage(margin_x),
        ])
        .split(middle_row);
    let [_, middle_col, _] = *horizontal else {
        return area;
    };
    middle_col
}

/// Title for the preview block: flags when the clock segment is enabled.
///
/// What's shown below it is a fixed demo time rather than the real
/// prompt's live wall-clock time. The clock has no `format`/template — it's
/// rendered entirely Fish-side against live wall-clock time
/// (`fish/segments/clock.fish`'s `gpy_section_standalone` call), so
/// `preview::render_preview_line` can't reflect the actual current time; it
/// substitutes a fixed representative time instead (see
/// `preview::clock_demo_spans`). Naming that in the title is honest about it
/// without porting that Fish-only rendering path into Rust.
fn preview_title(state: &WizardState) -> String {
    if state.is_segment_enabled(crate::commands::segments::BuiltinSegment::Clock.as_str()) {
        "Preview (clock shows a fixed demo time)".to_owned()
    } else {
        "Preview".to_owned()
    }
}

/// A bordered block titled `title`, with a bold, `accent`-tinted border when
/// `focused` is true so keyboard focus is visually obvious.
fn section_block(title: &str, focused: bool, accent: Option<RatatuiColor>) -> Block<'static> {
    let mut block = Block::default()
        .title(title.to_owned())
        .borders(Borders::ALL);
    if focused {
        block = block.border_style(border_style(true, accent));
    }
    block
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    #![allow(clippy::missing_panics_doc)]

    use super::*;
    use crate::config::Config;
    use crate::config::types::{PaletteName, ThemeName};

    fn make_state() -> WizardState {
        let mut config = Config::default();
        config.ui.theme = ThemeName::new("default".to_owned()).expect("valid theme name");
        config.ui.palette = PaletteName::new("default".to_owned()).expect("valid palette name");
        // Fixed, deterministic stand-ins for the real discovery lists
        // `WizardState::new` gathers from disk — see `state.rs`'s own
        // `FIXED_THEMES`/`FIXED_PALETTES`/`FIXED_SEGMENTS` for the rationale
        // (#607). Kept in sync with those exact values.
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

    #[test]
    fn wizard_colors_from_palette_resolves_accent_danger_warning() {
        let mut map = std::collections::HashMap::new();
        map.insert(
            "blue".to_owned(),
            crate::template::Color::Rgb { r: 1, g: 2, b: 3 },
        );
        map.insert(
            "red".to_owned(),
            crate::template::Color::Named("red".to_owned()),
        );
        map.insert("yellow".to_owned(), crate::template::Color::Ansi256(11));
        let palette = Palette::new(map);

        let colors = WizardColors::from_palette(&palette);

        assert_eq!(colors.accent, Some(RatatuiColor::Rgb(1, 2, 3)));
        assert_eq!(colors.danger, Some(RatatuiColor::Red));
        assert_eq!(colors.warning, Some(RatatuiColor::Indexed(11)));
    }

    #[test]
    fn wizard_colors_from_palette_falls_back_to_none_for_missing_names() {
        let colors = WizardColors::from_palette(&Palette::default());

        assert_eq!(colors.accent, None);
        assert_eq!(colors.danger, None);
        assert_eq!(colors.warning, None);
    }

    #[test]
    fn preview_title_is_plain_when_clock_disabled() {
        let mut state = make_state();
        let clock = crate::commands::segments::BuiltinSegment::Clock.as_str();
        if state.is_segment_enabled(clock) {
            state.toggle_segment(clock);
        }

        assert_eq!(preview_title(&state), "Preview");
    }

    #[test]
    fn preview_title_flags_clock_when_enabled() {
        let mut state = make_state();
        let clock = crate::commands::segments::BuiltinSegment::Clock.as_str();
        if !state.is_segment_enabled(clock) {
            state.toggle_segment(clock);
        }

        assert_eq!(
            preview_title(&state),
            "Preview (clock shows a fixed demo time)"
        );
    }

    #[test]
    fn section_title_is_plain_when_query_empty() {
        assert_eq!(section_title("Theme", ""), "Theme");
    }

    #[test]
    fn section_title_appends_query_when_present() {
        assert_eq!(section_title("Theme", "cat"), "Theme /cat");
    }

    fn wide_area() -> Rect {
        Rect::new(0, 0, 120, 40)
    }

    fn narrow_area() -> Rect {
        Rect::new(0, 0, 60, 40)
    }

    #[test]
    fn body_layout_places_detail_beside_master_when_wide() {
        let (master, detail) = body_layout(wide_area());

        assert_eq!(master.y, detail.y, "wide layout should be side-by-side");
        assert!(
            detail.x >= master.x + master.width,
            "detail pane should start where the master pane ends"
        );
        assert_eq!(master.height, wide_area().height);
    }

    #[test]
    fn body_layout_stacks_detail_below_master_when_narrow() {
        let (master, detail) = body_layout(narrow_area());

        assert_eq!(
            master.x, detail.x,
            "narrow layout should be stacked, not side-by-side"
        );
        assert_eq!(master.width, narrow_area().width);
        assert_eq!(detail.width, narrow_area().width);
        assert!(
            detail.y >= master.y + master.height,
            "detail pane should start below the master pane"
        );
    }

    #[test]
    fn body_layout_threshold_boundary() {
        let just_below = Rect::new(0, 0, NARROW_WIDTH_THRESHOLD - 1, 40);
        let just_at = Rect::new(0, 0, NARROW_WIDTH_THRESHOLD, 40);

        let (master_below, detail_below) = body_layout(just_below);
        let (master_at, _detail_at) = body_layout(just_at);

        assert_eq!(
            master_below.x, detail_below.x,
            "just under the threshold should still be stacked"
        );
        assert_eq!(
            master_at.height, just_at.height,
            "at the threshold should be side-by-side (full-height master)"
        );
    }

    /// End-to-end smoke test for the whole `draw` frame (not just
    /// `body_layout`'s geometry).
    ///
    /// Renders into a real `ratatui::backend::
    /// TestBackend` buffer at both a wide and a narrow width and checks the
    /// master list titles and the detail panel both made it onto the
    /// screen. `preview_title_*` and `body_layout_*` above already cover
    /// their pieces in isolation; this exists to catch the wiring mistakes
    /// unit tests on individual functions can't (e.g. a section rendered
    /// into the wrong `Rect`, or a panic from an empty list).
    #[test]
    fn draw_renders_master_and_detail_panels_at_wide_and_narrow_widths() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        for width in [120, 60] {
            let backend = TestBackend::new(width, 40);
            let mut terminal = Terminal::new(backend).expect("terminal should construct");
            let state = make_state();
            let preview = vec![Line::from("preview line")];
            let detail = vec![Line::from("detail line")];
            let args = DrawArgs {
                state: &state,
                mode: InputMode::Normal,
                preview: &preview,
                detail: &detail,
                colors: WizardColors::default(),
            };

            terminal
                .draw(|frame| draw(frame, &args))
                .expect("draw should not panic or error");

            let rendered = format!("{:?}", terminal.backend().buffer());
            assert!(rendered.contains("Theme"), "width {width}: {rendered}");
            assert!(rendered.contains("Palette"), "width {width}: {rendered}");
            assert!(rendered.contains("Segments"), "width {width}: {rendered}");
            assert!(rendered.contains("Detail"), "width {width}: {rendered}");
            assert!(rendered.contains("default"), "width {width}: {rendered}");
        }
    }

    #[test]
    fn draw_renders_no_matches_placeholder_for_empty_filtered_section() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut state = make_state();
        for c in "zzznotarealtheme".chars() {
            state.push_filter_char(c);
        }
        assert!(
            state.filtered_indices(Section::Theme).is_empty(),
            "test setup expects the query to match nothing"
        );

        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal should construct");
        let preview = vec![Line::from("preview line")];
        let detail = vec![Line::from("detail line")];
        let args = DrawArgs {
            state: &state,
            mode: InputMode::Filter,
            preview: &preview,
            detail: &detail,
            colors: WizardColors::default(),
        };

        terminal
            .draw(|frame| draw(frame, &args))
            .expect("draw should not panic or error");

        let rendered = format!("{:?}", terminal.backend().buffer());
        assert!(rendered.contains("no matches"), "{rendered}");
    }

    #[test]
    fn draw_narrows_rendered_items_when_section_is_filtered() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut state = make_state();
        // "default" is guaranteed to exist as a theme; filtering down to it
        // specifically narrows the rendered list to a subset.
        for c in "default".chars() {
            state.push_filter_char(c);
        }

        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal should construct");
        let preview = vec![Line::from("preview line")];
        let detail = vec![Line::from("detail line")];
        let args = DrawArgs {
            state: &state,
            mode: InputMode::Filter,
            preview: &preview,
            detail: &detail,
            colors: WizardColors::default(),
        };

        terminal
            .draw(|frame| draw(frame, &args))
            .expect("draw should not panic or error");

        let rendered = format!("{:?}", terminal.backend().buffer());
        assert!(rendered.contains("Theme /default"), "{rendered}");
    }
}
