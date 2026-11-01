//! Detail-panel content for whichever item has the cursor in the focused section.

use super::bindings::{Command, key_hint};
use super::state::{Section, SegmentEdit, WizardState};
use crate::theme::ThemeConfig;
use ratatui::text::Line;

struct SegmentDetail {
    /// Config path(s) shown to the user; `git`/`language` need both their
    /// boolean and `ui.enabled_segments` membership.
    config_field: &'static str,
    preview_contribution: &'static str,
    /// Where the preview differs from the real prompt; `None` if faithful.
    caveat: Option<&'static str>,
}

/// Detail metadata for `segment`; unknown (plugin) segments get a generic fallback.
fn segment_detail(segment: &str) -> SegmentDetail {
    use crate::plugin::BuiltinSegment;

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
            config_field: "ui.enabled_segments",
            preview_contribution: "Shows the detected language and version for the current directory.",
            caveat: Some("Only shown when a supported language is detected in the directory."),
        },
        Ok(BuiltinSegment::Directory) => SegmentDetail {
            config_field: "ui.enabled_segments",
            preview_contribution: "Shows the current working directory.",
            caveat: None,
        },
        Ok(BuiltinSegment::Git) => SegmentDetail {
            config_field: "ui.enabled_segments",
            preview_contribution: "Shows git branch, ahead/behind counts, and working-tree status.",
            caveat: Some("Only shown inside a git repository."),
        },
        Ok(BuiltinSegment::Status) => SegmentDetail {
            config_field: "ui.enabled_segments",
            preview_contribution: "Shows the previous command's exit status as a pill (✔/✖) in the segment chain.",
            caveat: Some("Preview uses a representative sample status, not a real command result."),
        },
        Err(_) => SegmentDetail {
            config_field: "ui.enabled_segments",
            preview_contribution: "Provided by a plugin.",
            caveat: Some(
                "Preview rendering for plugin segments may not exactly match the real prompt.",
            ),
        },
    }
}

/// Detail lines for the cursor item in the focused section.
///
/// `theme` is the on-disk theme, the baseline for theme-scoped fields like clock.
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

/// Shared Theme/Palette body; always states the current selection and whether
/// it differs from startup, regardless of cursor position.
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
        format!(
            "Not selected — press {} to select.",
            key_hint(&[Command::Activate])
        )
    }));
    lines.push(Line::from(format!("Selected value: {selected}")));
    lines.push(Line::from(if selected == initial {
        "Matches the startup config.".to_owned()
    } else {
        format!("Changed from startup value '{initial}' (not yet saved).")
    }));
    lines
}

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
    match crate::plugin::BuiltinSegment::try_from(segment.as_str()) {
        Ok(crate::plugin::BuiltinSegment::Directory) => {
            lines.extend(directory_display_lines(state));
        }
        Ok(crate::plugin::BuiltinSegment::Language) => {
            lines.extend(language_display_lines(state));
        }
        Ok(crate::plugin::BuiltinSegment::Clock) => {
            lines.extend(clock_display_lines(state, theme));
        }
        Ok(crate::plugin::BuiltinSegment::Duration) => {
            lines.extend(duration_display_lines(state, theme));
        }
        Ok(crate::plugin::BuiltinSegment::Git) => {
            lines.extend(git_display_lines(state, theme));
        }
        Ok(crate::plugin::BuiltinSegment::Status) | Err(_) => {}
    }
    lines
}

/// `(press <keys> to <verb>)` for the keys bound to `edits`.
fn press_hint(edits: &[SegmentEdit], verb: &str) -> String {
    let commands: Vec<Command> = edits.iter().map(|&edit| Command::Edit(edit)).collect();
    format!("(press {} to {verb})", key_hint(&commands))
}

fn directory_display_lines(state: &WizardState) -> Vec<Line<'static>> {
    let current = state.directory_display();
    let initial = state.initial_directory_display();
    let hint = press_hint(
        &[
            SegmentEdit::PrevDirectoryDisplay,
            SegmentEdit::NextDirectoryDisplay,
        ],
        "cycle",
    );
    vec![
        Line::from(""),
        Line::from(format!("Display style: {current} {hint}")),
        Line::from("Display config field: ui.directory.display"),
        Line::from(if current == initial {
            "Matches the startup config.".to_owned()
        } else {
            format!("Changed from startup value '{initial}' (not yet saved).")
        }),
    ]
}

fn language_display_lines(state: &WizardState) -> Vec<Line<'static>> {
    let current_display = state.language_display();
    let initial_display = state.initial_language_display();
    let current_versions = state.language_show_versions();
    let initial_versions = state.initial_language_show_versions();
    vec![
        Line::from(""),
        Line::from(format!(
            "Display style: {current_display} {}",
            press_hint(&[SegmentEdit::ToggleLanguageDisplay], "cycle")
        )),
        Line::from("Display config field: language.display"),
        Line::from(if current_display == initial_display {
            "Matches the startup config.".to_owned()
        } else {
            format!("Changed from startup value '{initial_display}' (not yet saved).")
        }),
        Line::from(""),
        Line::from(format!(
            "Show versions: {} {}",
            if current_versions { "on" } else { "off" },
            press_hint(&[SegmentEdit::ToggleLanguageShowVersions], "toggle")
        )),
        Line::from("Show-versions config field: language.show_versions"),
        Line::from(if current_versions == initial_versions {
            "Matches the startup config.".to_owned()
        } else {
            format!("Changed from startup value '{initial_versions}' (not yet saved).")
        }),
    ]
}

/// Clock options, diffed against the on-disk `theme` (not `initial_config`),
/// since these fields are theme-scoped.
fn clock_display_lines(state: &WizardState, theme: &ThemeConfig) -> Vec<Line<'static>> {
    let current = &state.active_theme(theme).segments.clock;
    let initial = &theme.segments.clock;
    let (current_format, initial_format) = (current.time_format(), initial.time_format());
    let (current_seconds, initial_seconds) = (current.show_seconds(), initial.show_seconds());
    vec![
        Line::from(""),
        Line::from(format!(
            "Time format: {current_format}h {}",
            press_hint(&[SegmentEdit::ToggleClockTimeFormat], "cycle")
        )),
        Line::from("Time format config field: segments.clock.time_format"),
        Line::from(if current_format == initial_format {
            "Matches the startup config.".to_owned()
        } else {
            format!("Changed from startup value '{initial_format}' (not yet saved).")
        }),
        Line::from(""),
        Line::from(format!(
            "Show seconds: {} {}",
            if current_seconds { "on" } else { "off" },
            press_hint(&[SegmentEdit::ToggleClockShowSeconds], "toggle")
        )),
        Line::from("Show-seconds config field: segments.clock.show_seconds"),
        Line::from(if current_seconds == initial_seconds {
            "Matches the startup config.".to_owned()
        } else {
            format!("Changed from startup value '{initial_seconds}' (not yet saved).")
        }),
    ]
}

/// Duration options, diffed against `theme` like `clock_display_lines`.
fn duration_display_lines(state: &WizardState, theme: &ThemeConfig) -> Vec<Line<'static>> {
    let current = state
        .active_theme(theme)
        .segments
        .duration
        .show_milliseconds;
    let initial = theme.segments.duration.show_milliseconds;
    vec![
        Line::from(""),
        Line::from(format!(
            "Show milliseconds: {} {}",
            if current { "on" } else { "off" },
            press_hint(&[SegmentEdit::ToggleDurationShowMilliseconds], "toggle")
        )),
        Line::from("Show-milliseconds config field: segments.duration.show_milliseconds"),
        Line::from(if current == initial {
            "Matches the startup config.".to_owned()
        } else {
            format!("Changed from startup value '{initial}' (not yet saved).")
        }),
    ]
}

/// Git visibility flags, diffed against `theme`.
fn git_display_lines(state: &WizardState, theme: &ThemeConfig) -> Vec<Line<'static>> {
    let edited = &state.active_theme(theme).segments.git;
    let on_disk = &theme.segments.git;
    let rows = [
        (
            "Show branch",
            SegmentEdit::ToggleGitShowBranch,
            "segments.git.show_branch",
            edited.show_branch(),
            on_disk.show_branch(),
        ),
        (
            "Show ahead/behind",
            SegmentEdit::ToggleGitShowAheadBehind,
            "segments.git.show_ahead_behind",
            edited.show_ahead_behind(),
            on_disk.show_ahead_behind(),
        ),
        (
            "Show stash",
            SegmentEdit::ToggleGitShowStash,
            "segments.git.show_stash",
            edited.show_stash(),
            on_disk.show_stash(),
        ),
    ];
    let mut lines = Vec::new();
    for (label, edit, field, current, initial) in rows {
        lines.push(Line::from(""));
        lines.push(Line::from(format!(
            "{label}: {} {}",
            if current { "on" } else { "off" },
            press_hint(&[edit], "toggle")
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
        crate::commands::wizard::state::builtin_state(config)
    }

    fn lines_to_strings(lines: &[Line<'static>]) -> Vec<String> {
        lines.iter().map(std::string::ToString::to_string).collect()
    }

    #[test]
    fn theme_detail_shows_highlighted_theme_as_selected() {
        let state = make_state("default", "default");
        let theme = ThemeConfig::default();
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
            // Only one theme available.
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
            // Only one theme available.
            return;
        }

        // Highlight something other than the unchanged selection.
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
        assert!(lines.contains(&"Config field: ui.enabled_segments".to_owned()));
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

        state.apply_edit(SegmentEdit::NextDirectoryDisplay, &theme);
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

        state.apply_edit(SegmentEdit::ToggleLanguageDisplay, &theme);
        state.apply_edit(SegmentEdit::ToggleLanguageShowVersions, &theme);
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

        state.apply_edit(SegmentEdit::ToggleDurationShowMilliseconds, &theme);
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

        state.apply_edit(SegmentEdit::ToggleGitShowStash, &theme);
        let after = lines_to_strings(&detail_lines(&state, &theme));

        assert!(after.contains(&"Show stash: off (press t to toggle)".to_owned()));
        assert!(
            after
                .iter()
                .any(|line| line.contains("Changed from startup value 'true'"))
        );
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
