//! `ratatui` widget composition for the wizard frame.

use super::bindings;
use super::keys::InputMode;
use super::preview::to_ratatui_color;
use super::state::{Section, WizardState};
use crate::template::Palette;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color as RatatuiColor, Modifier, Style};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};

/// Chrome colors from the selected palette; `None` (terminal default) when the
/// palette lacks the entry, since custom palettes needn't define them.
#[derive(Clone, Copy, Default)]
pub struct WizardColors {
    /// Palette `blue`: focused borders and cursor row.
    accent: Option<RatatuiColor>,
    /// Palette `red`: discard prompt.
    danger: Option<RatatuiColor>,
    /// Palette `yellow`: filter status line.
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

fn palette_named(palette: &Palette, name: &str) -> Option<RatatuiColor> {
    palette.get(name).as_ref().and_then(to_ratatui_color)
}

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

/// Below this body width the detail panel stacks under the lists.
const NARROW_WIDTH_THRESHOLD: u16 = 80;

/// Bundles `draw`'s non-frame parameters (keeps `draw` under clippy's argument limit).
pub struct DrawArgs<'a> {
    pub state: &'a WizardState,
    pub mode: InputMode,
    pub preview: &'a [Line<'static>],
    pub detail: &'a [Line<'static>],
    pub colors: WizardColors,
}

/// Draw one wizard frame.
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
        // Unreachable: split returns one Rect per constraint.
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
        render_help_overlay(frame, area, args.state, args.colors);
    }
}

/// Theme / Palette / Segments lists, all narrowed by the shared filter query.
fn render_sections(frame: &mut Frame, area: Rect, state: &WizardState, colors: WizardColors) {
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Min(3), Constraint::Min(3)])
        .split(area);
    let [theme_area, palette_area, segments_area] = *sections else {
        // Unreachable: split returns one Rect per constraint.
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

    let segments_title = format!(
        "Segments ({} to toggle)",
        bindings::key_hint(&[bindings::Command::Activate])
    );
    render_one_section(
        frame,
        segments_area,
        state,
        &SectionSource {
            section: Section::Segments,
            title: &segments_title,
            items: state.available_segments(),
            raw_cursor: state.segment_cursor(),
            is_selected: &|name| state.is_segment_enabled(name),
        },
        colors,
    );
}

/// One master-list section's data source.
struct SectionSource<'a> {
    section: Section,
    title: &'a str,
    items: &'a [String],
    /// Cursor index into the unfiltered `items`.
    raw_cursor: usize,
    is_selected: &'a dyn Fn(&str) -> bool,
}

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

/// Narrowed items, their selection flags, and the cursor's narrowed position.
type FilteredSection = (Vec<String>, Vec<bool>, Option<usize>);

/// Narrow to `indices`, mapping `raw_cursor` into the narrowed list (`None`
/// if empty).
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

fn render_detail_panel(frame: &mut Frame, area: Rect, detail: &[Line<'static>]) {
    frame.render_widget(
        Paragraph::new(Text::from(detail.to_vec()))
            .block(Block::default().title("Detail").borders(Borders::ALL))
            .wrap(Wrap { trim: true }),
        area,
    );
}

/// Live preview block; always accent-tinted to showcase the selected palette.
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

struct FooterArgs<'a> {
    mode: InputMode,
    /// Shown only in `InputMode::Filter`.
    filter_query: &'a str,
    colors: WizardColors,
}

/// Single-line footer: discard prompt, or the mode's key hints.
fn render_footer(frame: &mut Frame, area: Rect, args: &FooterArgs) {
    let hints = bindings::footer_hints(args.mode);
    let (footer, style) = match args.mode {
        InputMode::ConfirmQuit => (
            "Discard changes and quit? [y/N]".to_owned(),
            args.colors
                .danger
                .map_or_else(Style::default, |color| Style::default().fg(color)),
        ),
        InputMode::Filter => (
            format!("FILTERING: /{}_  {hints}", args.filter_query),
            args.colors
                .warning
                .map_or_else(Style::default, |color| Style::default().fg(color)),
        ),
        InputMode::Normal | InputMode::Help => (hints, Style::default()),
    };
    frame.render_widget(Paragraph::new(footer).style(style), area);
}

/// Split the body into master and detail areas: side-by-side when wide,
/// stacked when narrow.
fn body_layout(area: Rect) -> (Rect, Rect) {
    if area.width < NARROW_WIDTH_THRESHOLD {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(6), Constraint::Length(7)])
            .split(area);
        let [master, detail] = *rows else {
            // Unreachable: split returns one Rect per constraint.
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

/// One section's render inputs, already narrowed to the filtered view.
struct SectionSpec<'a> {
    title: &'a str,
    items: &'a [String],
    /// Selection marker per item, aligned with `items`.
    selected: &'a [bool],
    /// `None` when `items` is empty.
    cursor_index: Option<usize>,
    focused: bool,
    filter_query: &'a str,
    accent: Option<RatatuiColor>,
}

/// `title` suffixed with the filter query, so filters stay visible unfocused.
fn section_title(title: &str, filter_query: &str) -> String {
    if filter_query.is_empty() {
        title.to_owned()
    } else {
        format!("{title} /{filter_query}")
    }
}

/// Render one section. The cursor highlight is separate from the `[x]` marker
/// because in Segments the cursor can sit on an unselected item.
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

/// `?` help overlay listing the keys bound right now. `Clear` first: ratatui
/// doesn't blend widgets, so old glyphs would show through.
fn render_help_overlay(frame: &mut Frame, area: Rect, state: &WizardState, colors: WizardColors) {
    let popup = centered_rect(70, 80, area);
    frame.render_widget(Clear, popup);

    let lines: Vec<Line> = bindings::help_lines(state.focused_builtin())
        .into_iter()
        .map(Line::from)
        .collect();

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .title("Help")
                    .borders(Borders::ALL)
                    .border_style(border_style(true, colors.accent)),
            )
            .wrap(Wrap { trim: false }),
        popup,
    );
}

/// Centered `percent_x` × `percent_y` rect within `area`.
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
        // Unreachable: split returns one Rect per constraint.
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

/// Preview title; flags that the clock shows a fixed demo time, since the
/// real clock is rendered shell-side.
fn preview_title(state: &WizardState) -> String {
    if state.is_segment_enabled(crate::plugin::BuiltinSegment::Clock.as_str()) {
        "Preview (clock shows a fixed demo time)".to_owned()
    } else {
        "Preview".to_owned()
    }
}

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

    use super::*;
    use crate::config::Config;
    use crate::config::types::{PaletteName, ThemeName};

    fn make_state() -> WizardState {
        let mut config = Config::default();
        config.ui.theme = ThemeName::new("default".to_owned()).expect("valid theme name");
        config.ui.palette = PaletteName::new("default".to_owned()).expect("valid palette name");
        crate::commands::wizard::state::builtin_state(config)
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
        let clock = crate::plugin::BuiltinSegment::Clock.as_str();
        if state.is_segment_enabled(clock) {
            state.toggle_segment(clock);
        }

        assert_eq!(preview_title(&state), "Preview");
    }

    #[test]
    fn preview_title_flags_clock_when_enabled() {
        let mut state = make_state();
        let clock = crate::plugin::BuiltinSegment::Clock.as_str();
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

    /// Full-frame smoke test at wide and narrow widths to catch wiring bugs.
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
