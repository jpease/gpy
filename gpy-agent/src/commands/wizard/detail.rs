//! Pure detail-panel content for the master/detail wizard layout: describes
//! whichever item currently has the cursor in the focused section, without
//! requiring it to be selected first (issue #400).
//!
//! No terminal I/O — like `preview.rs`, this builds `ratatui` text types
//! directly rather than an intermediate string representation, since
//! `ui::draw` renders them as-is.

use super::state::{Section, WizardState};
use crate::theme::ThemeConfig;
use ratatui::text::Line;

/// Static metadata about a builtin segment, shown in its detail panel entry.
struct SegmentDetail {
    /// Dotted config path(s) that control this segment, as shown to the user
    /// (e.g. `"git.enabled + ui.enabled_segments"`). `git`/`language` render
    /// only with both their dedicated boolean and list membership (see
    /// `commands::segments::is_effectively_enabled`); everything else is list
    /// membership in `ui.enabled_segments`.
    config_field: &'static str,
    /// One-line description of what the segment adds to the preview.
    preview_contribution: &'static str,
    /// A known caveat about the segment's *preview* behavior specifically
    /// (not general segment behavior) — e.g. that the preview substitutes a
    /// fixed sample instead of a real value. `None` when the preview is a
    /// faithful representation of the real prompt output.
    caveat: Option<&'static str>,
}

/// Look up detail metadata for `segment`. Unknown names (plugin segments,
/// which have no curated copy) get a generic fallback rather than an
/// `Option::None` — the detail panel always has something to show.
fn segment_detail(segment: &str) -> SegmentDetail {
    use crate::commands::segments::BuiltinSegment;

    match BuiltinSegment::try_from(segment) {
        Ok(BuiltinSegment::Clock) => SegmentDetail {
            config_field: "ui.enabled_segments",
            preview_contribution: "Adds the current time to the prompt chain.",
            caveat: Some("Preview shows a fixed demo time, not the live wall-clock time."),
        },
        Ok(BuiltinSegment::Duration) => SegmentDetail {
            config_field: "ui.enabled_segments",
            preview_contribution: "Shows how long the previous command took to run.",
            caveat: Some("Preview uses a representative sample duration, not a real command."),
        },
        Ok(BuiltinSegment::Language) => SegmentDetail {
            config_field: "language.enabled + ui.enabled_segments",
            preview_contribution: "Shows the detected language and version for the current directory.",
            caveat: Some("Only shown when a supported language is detected in the directory."),
        },
        Ok(BuiltinSegment::Directory) => SegmentDetail {
            config_field: "ui.enabled_segments",
            preview_contribution: "Shows the current working directory.",
            caveat: None,
        },
        Ok(BuiltinSegment::Git) => SegmentDetail {
            config_field: "git.enabled + ui.enabled_segments",
            preview_contribution: "Shows git branch, ahead/behind counts, and working-tree status.",
            caveat: Some("Only shown inside a git repository."),
        },
        Ok(BuiltinSegment::Status) => SegmentDetail {
            config_field: "ui.enabled_segments",
            preview_contribution: "Shows the previous command's exit status on its own line.",
            caveat: Some("Preview uses a representative sample status, not a real command result."),
        },
        Err(()) => SegmentDetail {
            config_field: "ui.enabled_segments",
            preview_contribution: "Provided by a plugin.",
            caveat: Some(
                "Preview rendering for plugin segments may not exactly match the real prompt.",
            ),
        },
    }
}

/// Build the detail panel's content for the item currently under the cursor
/// in `state`'s focused section. Empty (but non-panicking) when the focused
/// section's list has no items.
///
/// `theme` is the currently-selected theme as loaded from disk (the render
/// loop's cached `ThemeConfig`) — needed for `"clock"`'s detail lines, since
/// `segments.clock.*` (unlike `directory`/`language`'s config-level fields)
/// lives in the per-theme TOML, not anything `WizardState` snapshots itself
/// (#407).
pub fn detail_lines(state: &WizardState, theme: &ThemeConfig) -> Vec<Line<'static>> {
    match state.focus() {
        Section::Theme => theme_or_palette_lines(
            state.available_themes().get(state.theme_cursor()),
            state.selected_theme(),
            state.initial_theme(),
        ),
        Section::Palette => theme_or_palette_lines(
            state.available_palettes().get(state.palette_cursor()),
            state.selected_palette(),
            state.initial_palette(),
        ),
        Section::Segments => segment_lines(state, theme),
    }
}

/// Shared detail body for Theme and Palette.
///
/// Both are single-select lists diffed against the startup config the same
/// way, so this one function backs both `Section::Theme` and
/// `Section::Palette` in `detail_lines`.
///
/// Always states the current selection and its startup-diff status — not
/// just when the cursor happens to sit on the selected item — so the panel
/// answers "what's selected, and has it changed from startup?" regardless
/// of what's highlighted (issue #400's Theme/Palette acceptance criterion).
fn theme_or_palette_lines(
    highlighted: Option<&String>,
    selected: &str,
    initial: &str,
) -> Vec<Line<'static>> {
    let Some(highlighted_item) = highlighted else {
        return vec![Line::from("No options available.")];
    };

    let mut lines = vec![Line::from(highlighted_item.clone()), Line::from("")];
    lines.push(Line::from(if highlighted_item == selected {
        "Currently selected.".to_owned()
    } else {
        "Not selected — press space/enter to select.".to_owned()
    }));
    lines.push(Line::from(format!("Selected value: {selected}")));
    lines.push(Line::from(if selected == initial {
        "Matches the startup config.".to_owned()
    } else {
        format!("Changed from startup value '{initial}' (not yet saved).")
    }));
    lines
}

/// Detail body for `Section::Segments`: the highlighted segment's enabled
/// state plus the curated `segment_detail` metadata.
fn segment_lines(state: &WizardState, theme: &ThemeConfig) -> Vec<Line<'static>> {
    let Some(segment) = state.available_segments().get(state.segment_cursor()) else {
        return vec![Line::from("No segments available.")];
    };

    let detail = segment_detail(segment);
    let enabled = if state.is_segment_enabled(segment) {
        "Enabled"
    } else {
        "Disabled"
    };

    let mut lines = vec![
        Line::from(segment.clone()),
        Line::from(""),
        Line::from(enabled),
        Line::from(format!("Config field: {}", detail.config_field)),
        Line::from(format!("Preview: {}", detail.preview_contribution)),
    ];
    if let Some(caveat) = detail.caveat {
        lines.push(Line::from(format!("Caveat: {caveat}")));
    }
    match crate::commands::segments::BuiltinSegment::try_from(segment.as_str()) {
        Ok(crate::commands::segments::BuiltinSegment::Directory) => {
            lines.extend(directory_display_lines(state));
        }
        Ok(crate::commands::segments::BuiltinSegment::Language) => {
            lines.extend(language_display_lines(state));
        }
        Ok(crate::commands::segments::BuiltinSegment::Clock) => {
            lines.extend(clock_display_lines(state, theme));
        }
        Ok(crate::commands::segments::BuiltinSegment::Duration) => {
            lines.extend(duration_display_lines(state, theme));
        }
        Ok(crate::commands::segments::BuiltinSegment::Git) => {
            lines.extend(git_display_lines(state, theme));
        }
        Ok(crate::commands::segments::BuiltinSegment::Status) | Err(()) => {}
    }
    lines
}

/// Extra detail lines for the `"directory"` segment.
///
/// Its display style (`ui.directory.display`), the key to change it, and a startup-diff note
/// — mirrors `theme_or_palette_lines`'s diff note, since this is the same "what's selected,
/// has it changed" question for a value that isn't a `Section` of its own.
///
fn directory_display_lines(state: &WizardState) -> Vec<Line<'static>> {
    let current = state.directory_display();
    let initial = state.initial_directory_display();
    vec![
        Line::from(""),
        Line::from(format!("Display style: {current} (press [ / ] to cycle)")),
        Line::from("Display config field: ui.directory.display"),
        Line::from(if current == initial {
            "Matches the startup config.".to_owned()
        } else {
            format!("Changed from startup value '{initial}' (not yet saved).")
        }),
    ]
}

/// Extra detail lines for the `"language"` segment.
///
/// Its display style (`language.display`, icon vs text) and its show-versions flag
/// (`language.show_versions`), each with the key to change it and a startup-diff note —
/// mirrors `directory_display_lines`, since both are the same "what's selected, has it
/// changed" question for values that aren't a `Section` of their own.
///
fn language_display_lines(state: &WizardState) -> Vec<Line<'static>> {
    let current_display = state.language_display();
    let initial_display = state.initial_language_display();
    let current_versions = state.language_show_versions();
    let initial_versions = state.initial_language_show_versions();
    vec![
        Line::from(""),
        Line::from(format!(
            "Display style: {current_display} (press [ / ] to cycle)"
        )),
        Line::from("Display config field: language.display"),
        Line::from(if current_display == initial_display {
            "Matches the startup config.".to_owned()
        } else {
            format!("Changed from startup value '{initial_display}' (not yet saved).")
        }),
        Line::from(""),
        Line::from(format!(
            "Show versions: {} (press v to toggle)",
            if current_versions { "on" } else { "off" }
        )),
        Line::from("Show-versions config field: language.show_versions"),
        Line::from(if current_versions == initial_versions {
            "Matches the startup config.".to_owned()
        } else {
            format!("Changed from startup value '{initial_versions}' (not yet saved).")
        }),
    ]
}

/// Extra detail lines for the `"clock"` segment.
///
/// Its time format (`segments.clock.time_format`) and show-seconds flag
/// (`segments.clock.show_seconds`), each with the key to change it and a startup-diff note —
/// mirrors `directory_display_lines`/`language_display_lines`, except the "startup" baseline
/// compared against is `theme` (the loaded active theme) rather than `WizardState`'s own
/// `initial_config`: these are theme-scoped fields `WizardState` doesn't snapshot itself
/// (#407), so "unedited" means "matches `theme` as loaded from disk for the currently selected
/// theme", read through `WizardState::clock_time_format`/`clock_show_seconds`'s
/// pending-edit-or-`theme` accessors.
///
fn clock_display_lines(state: &WizardState, theme: &ThemeConfig) -> Vec<Line<'static>> {
    let current_format = state.clock_time_format(theme);
    let initial_format = theme.segments.clock.time_format.as_deref().unwrap_or("12");
    let current_seconds = state.clock_show_seconds(theme);
    let initial_seconds = theme.segments.clock.show_seconds.unwrap_or(false);
    vec![
        Line::from(""),
        Line::from(format!(
            "Time format: {current_format}h (press [ / ] to cycle)"
        )),
        Line::from("Time format config field: segments.clock.time_format"),
        Line::from(if current_format == initial_format {
            "Matches the startup config.".to_owned()
        } else {
            format!("Changed from startup value '{initial_format}' (not yet saved).")
        }),
        Line::from(""),
        Line::from(format!(
            "Show seconds: {} (press : to toggle)",
            if current_seconds { "on" } else { "off" }
        )),
        Line::from("Show-seconds config field: segments.clock.show_seconds"),
        Line::from(if current_seconds == initial_seconds {
            "Matches the startup config.".to_owned()
        } else {
            format!("Changed from startup value '{initial_seconds}' (not yet saved).")
        }),
    ]
}

/// Extra detail lines for the `"duration"` segment.
///
/// Its show-milliseconds flag (`segments.duration.show_milliseconds`), with the key to change
/// it and a startup-diff note — mirrors `clock_display_lines`'s theme-scoped-field diff shape
/// (#407), except there's no `Option::unwrap_or` fallback needed:
/// `DurationTheme::show_milliseconds` is a plain `bool` (default `true`), not an
/// `Option<bool>`.
///
fn duration_display_lines(state: &WizardState, theme: &ThemeConfig) -> Vec<Line<'static>> {
    let current = state.duration_show_milliseconds(theme);
    let initial = theme.segments.duration.show_milliseconds;
    vec![
        Line::from(""),
        Line::from(format!(
            "Show milliseconds: {} (press m to toggle)",
            if current { "on" } else { "off" }
        )),
        Line::from("Show-milliseconds config field: segments.duration.show_milliseconds"),
        Line::from(if current == initial {
            "Matches the startup config.".to_owned()
        } else {
            format!("Changed from startup value '{initial}' (not yet saved).")
        }),
    ]
}

/// Extra detail lines for the `"git"` segment's content-visibility
/// multi-select (#410).
///
/// The three independent flags (`segments.git.show_branch` /
/// `show_ahead_behind` / `show_stash`) together form the "which git info to
/// show" multi-select, each with the key to toggle it and a startup-diff
/// note — mirrors `clock_display_lines`'s theme-scoped-field diff shape
/// (#407). Each flag is an `Option<bool>` on `GitTheme` defaulting to on when
/// unset, so the baseline compared against is `theme` (the loaded active
/// theme) read through `WizardState`'s pending-edit-or-`theme` accessors.
fn git_display_lines(state: &WizardState, theme: &ThemeConfig) -> Vec<Line<'static>> {
    let git = &theme.segments.git;
    let rows = [
        (
            "Show branch",
            'b',
            "segments.git.show_branch",
            state.git_show_branch(theme),
            git.show_branch.unwrap_or(true),
        ),
        (
            "Show ahead/behind",
            'a',
            "segments.git.show_ahead_behind",
            state.git_show_ahead_behind(theme),
            git.show_ahead_behind.unwrap_or(true),
        ),
        (
            "Show stash",
            't',
            "segments.git.show_stash",
            state.git_show_stash(theme),
            git.show_stash.unwrap_or(true),
        ),
    ];
    let mut lines = Vec::new();
    for (label, key, field, current, initial) in rows {
        lines.push(Line::from(""));
        lines.push(Line::from(format!(
            "{label}: {} (press {key} to toggle)",
            if current { "on" } else { "off" }
        )));
        lines.push(Line::from(format!("Config field: {field}")));
        lines.push(Line::from(if current == initial {
            "Matches the startup config.".to_owned()
        } else {
            format!("Changed from startup value '{initial}' (not yet saved).")
        }));
    }
    lines
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::missing_panics_doc
    )]

    use super::*;
    use crate::config::Config;
    use crate::config::types::{PaletteName, ThemeName};

    fn make_state(theme: &str, palette: &str) -> WizardState {
        let mut config = Config::default();
        config.ui.theme = ThemeName::new(theme.to_owned()).expect("valid theme name");
        config.ui.palette = PaletteName::new(palette.to_owned()).expect("valid palette name");
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

    fn lines_to_strings(lines: &[Line<'static>]) -> Vec<String> {
        lines.iter().map(std::string::ToString::to_string).collect()
    }

    #[test]
    fn theme_detail_shows_highlighted_theme_as_selected() {
        let state = make_state("default", "default");
        let theme = ThemeConfig::default();
        // Cursor starts on the selected theme (`WizardState::new` seeds
        // `theme_cursor` at the selected theme's position).
        let lines = lines_to_strings(&detail_lines(&state, &theme));

        assert_eq!(
            lines
                .first()
                .expect("detail_lines should return at least one line"),
            "default"
        );
        assert!(lines.contains(&"Currently selected.".to_owned()));
        assert!(lines.contains(&"Selected value: default".to_owned()));
        assert!(lines.contains(&"Matches the startup config.".to_owned()));
    }

    #[test]
    fn theme_detail_notes_change_from_startup_after_moving_cursor_and_selecting() {
        let mut state = make_state("default", "default");
        let theme = ThemeConfig::default();
        let Some(other) = state
            .available_themes()
            .iter()
            .find(|name| name.as_str() != "default")
            .cloned()
        else {
            // Only one built-in theme discovered in this environment;
            // nothing to switch to.
            return;
        };
        state.select_theme(&other);
        state.move_cursor_down(); // in case selecting didn't move the cursor

        let lines = lines_to_strings(&detail_lines(&state, &theme));

        assert!(
            lines.contains(&format!("Selected value: {other}")),
            "expected the new selection's value, got {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Changed from startup value 'default'")),
            "expected a startup-diff note, got {lines:?}"
        );
    }

    #[test]
    fn theme_detail_shows_selected_value_even_when_highlighting_a_different_item() {
        let mut state = make_state("default", "default");
        let theme = ThemeConfig::default();
        let theme_count = state.available_themes().len();
        if theme_count < 2 {
            // Only one built-in theme discovered in this environment;
            // nothing else to highlight.
            return;
        }

        // Move the cursor (without activating it) until it lands on
        // something other than "default", so the highlighted item differs
        // from the still-unchanged selection.
        for _ in 0..theme_count {
            state.move_cursor_down();
            if state
                .available_themes()
                .get(state.theme_cursor())
                .map(String::as_str)
                != Some("default")
            {
                break;
            }
        }
        assert_ne!(
            state
                .available_themes()
                .get(state.theme_cursor())
                .map(String::as_str),
            Some("default"),
            "cursor should have moved off the selected theme"
        );

        let lines = lines_to_strings(&detail_lines(&state, &theme));

        assert!(
            lines.contains(&"Selected value: default".to_owned()),
            "detail panel should state the current selection regardless of cursor position: {lines:?}"
        );
        assert!(lines.contains(&"Matches the startup config.".to_owned()));
    }

    #[test]
    fn palette_detail_shows_highlighted_palette_as_selected() {
        let mut state = make_state("default", "default");
        let theme = ThemeConfig::default();
        state.next_section(); // Theme -> Palette

        let lines = lines_to_strings(&detail_lines(&state, &theme));

        assert_eq!(
            lines
                .first()
                .expect("detail_lines should return at least one line"),
            "default"
        );
        assert!(lines.contains(&"Currently selected.".to_owned()));
        assert!(lines.contains(&"Selected value: default".to_owned()));
        assert!(lines.contains(&"Matches the startup config.".to_owned()));
    }

    #[test]
    fn segment_detail_reports_enabled_state_and_config_field() {
        let mut state = make_state("default", "default");
        let theme = ThemeConfig::default();
        state.next_section(); // Theme -> Palette
        state.next_section(); // Palette -> Segments
        while state
            .available_segments()
            .get(state.segment_cursor())
            .map(String::as_str)
            != Some("git")
        {
            state.move_cursor_down();
        }
        if !state.is_segment_enabled("git") {
            state.toggle_segment("git");
        }

        let lines = lines_to_strings(&detail_lines(&state, &theme));

        assert_eq!(
            lines
                .first()
                .expect("detail_lines should return at least one line"),
            "git"
        );
        assert!(lines.contains(&"Enabled".to_owned()));
        assert!(lines.contains(&"Config field: git.enabled + ui.enabled_segments".to_owned()));
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("Preview: Shows git branch"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("Caveat: Only shown inside a git repository."))
        );
    }

    #[test]
    fn directory_segment_detail_reports_display_style_and_diff() {
        let mut state = make_state("default", "default");
        let theme = ThemeConfig::default();
        state.next_section(); // Theme -> Palette
        state.next_section(); // Palette -> Segments
        while state
            .available_segments()
            .get(state.segment_cursor())
            .map(String::as_str)
            != Some("directory")
        {
            state.move_cursor_down();
        }

        let before = lines_to_strings(&detail_lines(&state, &theme));
        assert!(before.contains(&"Display style: basename (press [ / ] to cycle)".to_owned()));
        assert!(before.contains(&"Display config field: ui.directory.display".to_owned()));
        assert!(before.contains(&"Matches the startup config.".to_owned()));

        state.cycle_directory_display(true);
        let after = lines_to_strings(&detail_lines(&state, &theme));

        assert!(after.contains(&"Display style: abbreviated (press [ / ] to cycle)".to_owned()));
        assert!(
            after
                .iter()
                .any(|line| line.contains("Changed from startup value 'basename'"))
        );
    }

    #[test]
    fn language_segment_detail_reports_display_style_versions_and_diff() {
        let mut state = make_state("default", "default");
        let theme = ThemeConfig::default();
        state.next_section(); // Theme -> Palette
        state.next_section(); // Palette -> Segments
        while state
            .available_segments()
            .get(state.segment_cursor())
            .map(String::as_str)
            != Some("language")
        {
            state.move_cursor_down();
        }

        let before = lines_to_strings(&detail_lines(&state, &theme));
        assert!(before.contains(&"Display style: icon (press [ / ] to cycle)".to_owned()));
        assert!(before.contains(&"Display config field: language.display".to_owned()));
        assert!(before.contains(&"Show versions: on (press v to toggle)".to_owned()));
        assert!(before.contains(&"Show-versions config field: language.show_versions".to_owned()));
        assert_eq!(
            before
                .iter()
                .filter(|line| line.as_str() == "Matches the startup config.")
                .count(),
            2,
            "both display style and show-versions should report matching startup state"
        );

        state.cycle_language_display(true);
        state.toggle_language_show_versions();
        let after = lines_to_strings(&detail_lines(&state, &theme));

        assert!(after.contains(&"Display style: text (press [ / ] to cycle)".to_owned()));
        assert!(after.contains(&"Show versions: off (press v to toggle)".to_owned()));
        assert!(
            after
                .iter()
                .any(|line| line.contains("Changed from startup value 'icon'"))
        );
        assert!(
            after
                .iter()
                .any(|line| line.contains("Changed from startup value 'true'"))
        );
    }

    #[test]
    fn duration_segment_detail_reports_show_milliseconds_and_diff() {
        let mut state = make_state("default", "default");
        let theme = ThemeConfig::default();
        state.next_section(); // Theme -> Palette
        state.next_section(); // Palette -> Segments
        while state
            .available_segments()
            .get(state.segment_cursor())
            .map(String::as_str)
            != Some("duration")
        {
            state.move_cursor_down();
        }

        let before = lines_to_strings(&detail_lines(&state, &theme));
        assert!(before.contains(&"Show milliseconds: on (press m to toggle)".to_owned()));
        assert!(before.contains(
            &"Show-milliseconds config field: segments.duration.show_milliseconds".to_owned()
        ));
        assert!(before.contains(&"Matches the startup config.".to_owned()));

        state.toggle_duration_show_milliseconds(&theme);
        let after = lines_to_strings(&detail_lines(&state, &theme));

        assert!(after.contains(&"Show milliseconds: off (press m to toggle)".to_owned()));
        assert!(
            after
                .iter()
                .any(|line| line.contains("Changed from startup value 'true'"))
        );
    }

    #[test]
    fn git_segment_detail_reports_content_flags_and_diff() {
        let mut state = make_state("default", "default");
        let theme = ThemeConfig::default();
        state.next_section(); // Theme -> Palette
        state.next_section(); // Palette -> Segments
        while state
            .available_segments()
            .get(state.segment_cursor())
            .map(String::as_str)
            != Some("git")
        {
            state.move_cursor_down();
        }

        let before = lines_to_strings(&detail_lines(&state, &theme));
        assert!(before.contains(&"Show branch: on (press b to toggle)".to_owned()));
        assert!(before.contains(&"Show ahead/behind: on (press a to toggle)".to_owned()));
        assert!(before.contains(&"Show stash: on (press t to toggle)".to_owned()));
        assert!(before.contains(&"Config field: segments.git.show_stash".to_owned()));
        assert!(before.contains(&"Matches the startup config.".to_owned()));

        state.toggle_git_show_stash(&theme);
        let after = lines_to_strings(&detail_lines(&state, &theme));

        assert!(after.contains(&"Show stash: off (press t to toggle)".to_owned()));
        assert!(
            after
                .iter()
                .any(|line| line.contains("Changed from startup value 'true'"))
        );
        // Branch/ahead-behind remain unchanged.
        assert!(after.contains(&"Show branch: on (press b to toggle)".to_owned()));
    }

    #[test]
    fn segment_detail_falls_back_for_unknown_segment_names() {
        let detail = segment_detail("some-plugin-segment");

        assert_eq!(detail.config_field, "ui.enabled_segments");
        assert_eq!(detail.preview_contribution, "Provided by a plugin.");
        assert!(detail.caveat.is_some());
    }
}
