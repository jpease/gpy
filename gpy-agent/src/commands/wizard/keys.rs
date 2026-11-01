//! Pure key-event → wizard-action mapping, testable without a terminal.

use super::bindings::{self, Command};
use super::state::WizardState;
use crate::theme::ThemeConfig;
use crossterm::event::{KeyCode, KeyEvent};

/// What the render loop should do after a key event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardAction {
    Continue,
    Save,
    ExitWithoutSaving,
}

/// Which overlay/prompt (if any) captures key input; not part of `dirty()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputMode {
    #[default]
    Normal,
    /// "Discard changes?" prompt; every key answers it.
    ConfirmQuit,
    /// Help overlay; keys other than its bindings are ignored.
    Help,
    /// Keys edit the filter query, which is shared across all sections.
    Filter,
}

/// Reduce `(mode, key)` to the next mode and the loop action.
///
/// `theme` is the on-disk theme, the base for theme-scoped segment edits.
pub fn handle_key(
    state: &mut WizardState,
    mode: InputMode,
    theme: &ThemeConfig,
    key: KeyEvent,
) -> (InputMode, WizardAction) {
    match bindings::lookup(mode, state.focused_builtin(), key.code) {
        Some(command) => run(state, mode, theme, command),
        None => handle_unbound_key(state, mode, key),
    }
}

fn run(
    state: &mut WizardState,
    mode: InputMode,
    theme: &ThemeConfig,
    command: Command,
) -> (InputMode, WizardAction) {
    let next_mode = match command {
        Command::NextSection => {
            state.next_section();
            mode
        }
        Command::PrevSection => {
            state.prev_section();
            mode
        }
        Command::CursorUp => {
            state.move_cursor_up();
            mode
        }
        Command::CursorDown => {
            state.move_cursor_down();
            mode
        }
        Command::Activate => {
            state.activate_cursor();
            mode
        }
        Command::OpenFilter => InputMode::Filter,
        Command::ClearFilter => {
            state.clear_filter();
            InputMode::Normal
        }
        Command::DeleteFilterChar => {
            state.pop_filter_char();
            mode
        }
        Command::OpenHelp => InputMode::Help,
        Command::CloseHelp => InputMode::Normal,
        Command::Edit(edit) => {
            state.apply_edit(edit, theme);
            mode
        }
        Command::Save => return (InputMode::Normal, WizardAction::Save),
        Command::Quit if state.dirty() => InputMode::ConfirmQuit,
        Command::Quit | Command::Discard => {
            return (InputMode::Normal, WizardAction::ExitWithoutSaving);
        }
    };
    (next_mode, WizardAction::Continue)
}

/// Filter mode types other characters into the query; any other key at the
/// discard prompt keeps editing; elsewhere unbound keys do nothing.
fn handle_unbound_key(
    state: &mut WizardState,
    mode: InputMode,
    key: KeyEvent,
) -> (InputMode, WizardAction) {
    match (mode, key.code) {
        (InputMode::Filter, KeyCode::Char(c)) => state.push_filter_char(c),
        (InputMode::ConfirmQuit, _) => return (InputMode::Normal, WizardAction::Continue),
        _ => {}
    }
    (mode, WizardAction::Continue)
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
    use crate::commands::wizard::state::Section;
    use crate::config::Config;
    use crate::config::types::{PaletteName, ThemeName};

    fn make_config(theme: &str, palette: &str) -> Config {
        let mut config = Config::default();
        config.ui.theme = ThemeName::new(theme.to_owned()).expect("valid theme name");
        config.ui.palette = PaletteName::new(palette.to_owned()).expect("valid palette name");
        config
    }

    fn test_state(config: Config) -> WizardState {
        crate::commands::wizard::state::builtin_state(config)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }

    #[test]
    fn tab_advances_section() {
        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::Normal;
        let theme = ThemeConfig::default();
        assert_eq!(state.focus(), Section::Theme);

        let (new_mode, action) = handle_key(&mut state, mode, &theme, key(KeyCode::Tab));

        assert_eq!(action, WizardAction::Continue);
        assert_eq!(new_mode, InputMode::Normal);
        assert_eq!(state.focus(), Section::Palette);
    }

    #[test]
    fn backtab_retreats_section() {
        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::Normal;
        let theme = ThemeConfig::default();
        assert_eq!(state.focus(), Section::Theme);

        let (new_mode, action) = handle_key(&mut state, mode, &theme, key(KeyCode::BackTab));

        assert_eq!(action, WizardAction::Continue);
        assert_eq!(new_mode, InputMode::Normal);
        assert_eq!(state.focus(), Section::Segments);
    }

    #[test]
    fn s_key_returns_save_action() {
        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::Normal;
        let theme = ThemeConfig::default();

        let (new_mode, action) = handle_key(&mut state, mode, &theme, key(KeyCode::Char('s')));

        assert_eq!(action, WizardAction::Save);
        assert_eq!(new_mode, InputMode::Normal);
    }

    #[test]
    fn q_key_with_clean_state_returns_exit_action() {
        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::Normal;
        let theme = ThemeConfig::default();
        assert!(!state.dirty());

        let (new_mode, action) = handle_key(&mut state, mode, &theme, key(KeyCode::Char('q')));

        assert_eq!(action, WizardAction::ExitWithoutSaving);
        assert_eq!(new_mode, InputMode::Normal);
    }

    #[test]
    fn q_key_with_dirty_state_activates_confirmation_not_exit() {
        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::Normal;
        let theme = ThemeConfig::default();
        state.toggle_segment("git");
        assert!(state.dirty());

        let (new_mode, action) = handle_key(&mut state, mode, &theme, key(KeyCode::Char('q')));

        assert_eq!(action, WizardAction::Continue);
        assert_eq!(new_mode, InputMode::ConfirmQuit);
    }

    #[test]
    fn confirm_yes_returns_exit_action() {
        let mut state = test_state(make_config("default", "default"));
        let theme = ThemeConfig::default();

        for code in [KeyCode::Char('y'), KeyCode::Char('Y')] {
            let mode = InputMode::ConfirmQuit;
            let (new_mode, action) = handle_key(&mut state, mode, &theme, key(code));
            assert_eq!(action, WizardAction::ExitWithoutSaving);
            assert_eq!(new_mode, InputMode::Normal);
        }
    }

    #[test]
    fn confirm_enter_cancels() {
        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::ConfirmQuit;
        let theme = ThemeConfig::default();

        let (new_mode, action) = handle_key(&mut state, mode, &theme, key(KeyCode::Enter));

        assert_eq!(action, WizardAction::Continue);
        assert_eq!(new_mode, InputMode::Normal);
    }

    #[test]
    fn confirm_other_key_cancels_and_returns_to_wizard() {
        let mut state = test_state(make_config("default", "default"));
        state.toggle_segment("git");
        let focus_before = state.focus();
        let git_before = state.is_segment_enabled("git");
        let mode = InputMode::ConfirmQuit;
        let theme = ThemeConfig::default();

        let (new_mode, action) = handle_key(&mut state, mode, &theme, key(KeyCode::Char('n')));

        assert_eq!(action, WizardAction::Continue);
        assert_eq!(new_mode, InputMode::Normal);
        assert_eq!(state.focus(), focus_before);
        assert_eq!(state.is_segment_enabled("git"), git_before);
    }

    #[test]
    fn up_down_move_cursor_within_focused_section() {
        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::Normal;
        let theme = ThemeConfig::default();
        state.next_section();
        state.next_section();
        assert_eq!(state.focus(), Section::Segments);
        assert!(
            state.available_segments().len() >= 2,
            "test relies on at least two builtin segments existing"
        );

        let start = state.segment_cursor();
        let (mode_after_down, _) = handle_key(&mut state, mode, &theme, key(KeyCode::Down));
        assert_eq!(state.segment_cursor(), start + 1);

        handle_key(&mut state, mode_after_down, &theme, key(KeyCode::Up));
        assert_eq!(state.segment_cursor(), start);
    }

    #[test]
    fn left_and_right_alias_up_and_down_within_focused_section() {
        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::Normal;
        let theme = ThemeConfig::default();
        state.next_section();
        state.next_section();
        assert_eq!(state.focus(), Section::Segments);
        assert!(
            state.available_segments().len() >= 2,
            "test relies on at least two builtin segments existing"
        );

        let start = state.segment_cursor();
        let (mode_after_right, _) = handle_key(&mut state, mode, &theme, key(KeyCode::Right));
        assert_eq!(state.segment_cursor(), start + 1);

        handle_key(&mut state, mode_after_right, &theme, key(KeyCode::Left));
        assert_eq!(state.segment_cursor(), start);
    }

    #[test]
    fn space_and_enter_activate_cursor() {
        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::Normal;
        let theme = ThemeConfig::default();
        state.next_section();
        state.next_section();
        assert_eq!(state.focus(), Section::Segments);

        let target = state
            .available_segments()
            .first()
            .cloned()
            .expect("at least one builtin segment should exist");
        let before = state.is_segment_enabled(&target);

        let (mode_after_space, _) = handle_key(&mut state, mode, &theme, key(KeyCode::Char(' ')));
        assert_ne!(state.is_segment_enabled(&target), before);

        handle_key(&mut state, mode_after_space, &theme, key(KeyCode::Enter));
        assert_eq!(state.is_segment_enabled(&target), before);
    }

    /// Focus Segments with the cursor on `segment` (below the cursor).
    fn highlight(state: &mut WizardState, segment: &str) {
        while state.focus() != Section::Segments {
            state.next_section();
        }
        while state
            .available_segments()
            .get(state.segment_cursor())
            .map(String::as_str)
            != Some(segment)
        {
            state.move_cursor_down();
        }
    }

    fn press(
        state: &mut WizardState,
        theme: &ThemeConfig,
        code: char,
    ) -> (InputMode, WizardAction) {
        handle_key(state, InputMode::Normal, theme, key(KeyCode::Char(code)))
    }

    #[test]
    fn bracket_keys_cycle_the_directory_display_both_ways() {
        use crate::config::types::DirectoryDisplay;

        let mut state = test_state(make_config("default", "default"));
        let theme = ThemeConfig::default();
        highlight(&mut state, "directory");
        assert_eq!(state.directory_display(), DirectoryDisplay::Basename);

        let outcome = press(&mut state, &theme, ']');
        assert_eq!(outcome, (InputMode::Normal, WizardAction::Continue));
        assert_eq!(state.directory_display(), DirectoryDisplay::Abbreviated);

        press(&mut state, &theme, '[');
        press(&mut state, &theme, '[');
        assert_eq!(state.directory_display(), DirectoryDisplay::Full);
    }

    #[test]
    fn bracket_keys_switch_the_language_display() {
        use crate::config::types::LanguageDisplay;

        let mut state = test_state(make_config("default", "default"));
        let theme = ThemeConfig::default();
        highlight(&mut state, "language");

        for (code, expected) in [
            (']', LanguageDisplay::Text),
            ('[', LanguageDisplay::Icon),
            ('[', LanguageDisplay::Text),
        ] {
            press(&mut state, &theme, code);
            assert_eq!(state.language_display(), expected, "after '{code}'");
        }
    }

    #[test]
    fn bracket_keys_switch_the_clock_time_format() {
        let mut state = test_state(make_config("default", "default"));
        let theme = ThemeConfig::default();
        highlight(&mut state, "clock");

        for (code, expected) in [(']', "24"), ('[', "12"), ('[', "24")] {
            press(&mut state, &theme, code);
            assert_eq!(
                state.active_theme(&theme).segments.clock.time_format(),
                expected,
                "after '{code}'"
            );
        }
    }

    #[test]
    fn bracket_keys_edit_only_the_highlighted_segment() {
        use crate::config::types::LanguageDisplay;

        let mut state = test_state(make_config("default", "default"));
        let theme = ThemeConfig::default();
        highlight(&mut state, "directory");

        press(&mut state, &theme, ']');

        assert_eq!(state.language_display(), LanguageDisplay::Icon);
        assert!(
            state.pending_theme().is_none(),
            "the clock format must stay unedited"
        );
    }

    #[test]
    fn toggle_keys_flip_their_option_on_the_highlighted_segment() {
        type Read = fn(&WizardState, &ThemeConfig) -> bool;
        let cases: [(&str, char, Read); 6] = [
            ("language", 'v', |state, _| state.language_show_versions()),
            ("clock", ':', |state, theme| {
                state.active_theme(theme).segments.clock.show_seconds()
            }),
            ("duration", 'm', |state, theme| {
                state
                    .active_theme(theme)
                    .segments
                    .duration
                    .show_milliseconds
            }),
            ("git", 'b', |state, theme| {
                state.active_theme(theme).segments.git.show_branch()
            }),
            ("git", 'a', |state, theme| {
                state.active_theme(theme).segments.git.show_ahead_behind()
            }),
            ("git", 't', |state, theme| {
                state.active_theme(theme).segments.git.show_stash()
            }),
        ];
        let theme = ThemeConfig::default();
        for (segment, code, read) in cases {
            let mut state = test_state(make_config("default", "default"));
            highlight(&mut state, segment);
            let before = read(&state, &theme);

            let outcome = press(&mut state, &theme, code);

            assert_eq!(outcome, (InputMode::Normal, WizardAction::Continue));
            assert_eq!(read(&state, &theme), !before, "'{code}' on {segment}");
        }
    }

    #[test]
    fn segment_edit_keys_are_inert_unless_their_segment_is_highlighted() {
        let theme = ThemeConfig::default();
        for code in ['[', ']', 'v', ':', 'm', 'b', 'a', 't'] {
            for highlighted in [None, Some("status")] {
                let mut state = test_state(make_config("default", "default"));
                if let Some(segment) = highlighted {
                    highlight(&mut state, segment);
                }

                let outcome = press(&mut state, &theme, code);

                assert_eq!(outcome, (InputMode::Normal, WizardAction::Continue));
                assert!(!state.dirty(), "'{code}' with {highlighted:?} highlighted");
            }
        }
    }

    #[test]
    fn question_mark_opens_help_overlay() {
        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::Normal;
        let theme = ThemeConfig::default();

        let (new_mode, action) = handle_key(&mut state, mode, &theme, key(KeyCode::Char('?')));

        assert_eq!(action, WizardAction::Continue);
        assert_eq!(new_mode, InputMode::Help);
    }

    #[test]
    fn help_overlay_swallows_navigation_and_selection_keys() {
        let mut state = test_state(make_config("default", "default"));
        let mut mode = InputMode::Help;
        let theme = ThemeConfig::default();
        let focus_before = state.focus();
        let theme_before = state.selected_theme().to_owned();

        for code in [
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Char(' '),
            KeyCode::Enter,
            KeyCode::Char('s'),
        ] {
            let (new_mode, action) = handle_key(&mut state, mode, &theme, key(code));
            assert_eq!(action, WizardAction::Continue);
            mode = new_mode;
        }

        assert_eq!(mode, InputMode::Help, "overlay should still be open");
        assert_eq!(state.focus(), focus_before);
        assert_eq!(state.selected_theme(), theme_before);
    }

    #[test]
    fn question_mark_closes_help_overlay() {
        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::Help;
        let theme = ThemeConfig::default();

        let (new_mode, action) = handle_key(&mut state, mode, &theme, key(KeyCode::Char('?')));

        assert_eq!(action, WizardAction::Continue);
        assert_eq!(new_mode, InputMode::Normal);
    }

    #[test]
    fn escape_closes_help_overlay_and_restores_previous_focus() {
        let mut state = test_state(make_config("default", "default"));
        state.next_section();
        let focus_before = state.focus();
        let mode = InputMode::Help;
        let theme = ThemeConfig::default();

        let (new_mode, action) = handle_key(&mut state, mode, &theme, key(KeyCode::Esc));

        assert_eq!(action, WizardAction::Continue);
        assert_eq!(new_mode, InputMode::Normal);
        assert_eq!(state.focus(), focus_before);
    }

    #[test]
    fn escape_while_wizard_dirty_and_help_closed_still_confirms_quit() {
        let mut state = test_state(make_config("default", "default"));
        state.toggle_segment("git");
        let mode = InputMode::Normal;
        let theme = ThemeConfig::default();

        let (new_mode, action) = handle_key(&mut state, mode, &theme, key(KeyCode::Esc));

        assert_eq!(action, WizardAction::Continue);
        assert_eq!(new_mode, InputMode::ConfirmQuit);
    }

    #[test]
    fn slash_key_enters_filter_mode() {
        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::Normal;
        let theme = ThemeConfig::default();

        let (new_mode, action) = handle_key(&mut state, mode, &theme, key(KeyCode::Char('/')));

        assert_eq!(action, WizardAction::Continue);
        assert_eq!(new_mode, InputMode::Filter);
    }

    #[test]
    fn typed_char_while_filtering_appends_to_filter_query() {
        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::Filter;
        let theme = ThemeConfig::default();

        let (mode_after_d, _) = handle_key(&mut state, mode, &theme, key(KeyCode::Char('d')));
        let (mode_after_e, _) =
            handle_key(&mut state, mode_after_d, &theme, key(KeyCode::Char('e')));

        assert_eq!(state.filter_query(), "de");
        assert_eq!(
            mode_after_e,
            InputMode::Filter,
            "typing should not exit filter mode"
        );
    }

    #[test]
    fn backspace_while_filtering_removes_last_query_char() {
        let mut state = test_state(make_config("default", "default"));
        state.push_filter_char('d');
        state.push_filter_char('e');
        let mode = InputMode::Filter;
        let theme = ThemeConfig::default();

        handle_key(&mut state, mode, &theme, key(KeyCode::Backspace));

        assert_eq!(state.filter_query(), "d");
    }

    #[test]
    fn escape_while_filtering_clears_query_and_exits_filter_mode() {
        let mut state = test_state(make_config("default", "default"));
        state.push_filter_char('d');
        let mode = InputMode::Filter;
        let theme = ThemeConfig::default();

        let (new_mode, action) = handle_key(&mut state, mode, &theme, key(KeyCode::Esc));

        assert_eq!(action, WizardAction::Continue);
        assert_eq!(new_mode, InputMode::Normal);
        assert_eq!(state.filter_query(), "");
    }

    #[test]
    fn enter_and_space_while_filtering_activate_cursor_and_stay_in_filter_mode() {
        let mut state = test_state(make_config("default", "default"));
        state.next_section();
        state.next_section();
        assert_eq!(state.focus(), Section::Segments);
        let target = state
            .available_segments()
            .first()
            .cloned()
            .expect("at least one builtin segment should exist");
        let before = state.is_segment_enabled(&target);
        let mode = InputMode::Filter;
        let theme = ThemeConfig::default();

        let (new_mode, _) = handle_key(&mut state, mode, &theme, key(KeyCode::Char(' ')));

        assert_ne!(state.is_segment_enabled(&target), before);
        assert_eq!(
            new_mode,
            InputMode::Filter,
            "activating should not exit filter mode"
        );
    }

    #[test]
    fn arrows_move_cursor_while_filtering() {
        let mut state = test_state(make_config("default", "default"));
        state.next_section();
        state.next_section();
        assert_eq!(state.focus(), Section::Segments);
        assert!(
            state.available_segments().len() >= 2,
            "test relies on at least two builtin segments existing"
        );
        let start = state.segment_cursor();
        let mode = InputMode::Filter;
        let theme = ThemeConfig::default();

        let (new_mode, _) = handle_key(&mut state, mode, &theme, key(KeyCode::Down));

        assert_eq!(state.segment_cursor(), start + 1);
        assert_eq!(new_mode, InputMode::Filter);
    }

    #[test]
    fn tab_and_backtab_switch_sections_while_filtering_without_clearing_query() {
        let mut state = test_state(make_config("default", "default"));
        state.push_filter_char('d');
        assert_eq!(state.focus(), Section::Theme);
        let mode = InputMode::Filter;
        let theme = ThemeConfig::default();

        let (mode_after_tab, tab_action) = handle_key(&mut state, mode, &theme, key(KeyCode::Tab));
        assert_eq!(tab_action, WizardAction::Continue);
        assert_eq!(state.focus(), Section::Palette);
        assert_eq!(
            state.filter_query(),
            "d",
            "Tab must not clear the shared filter query"
        );
        assert_eq!(
            mode_after_tab,
            InputMode::Filter,
            "Tab must not exit filter mode"
        );

        let (mode_after_backtab, backtab_action) =
            handle_key(&mut state, mode_after_tab, &theme, key(KeyCode::BackTab));
        assert_eq!(backtab_action, WizardAction::Continue);
        assert_eq!(state.focus(), Section::Theme);
        assert_eq!(state.filter_query(), "d");
        assert_eq!(mode_after_backtab, InputMode::Filter);
    }

    #[test]
    fn save_quit_help_chars_become_filter_query_text_while_filtering() {
        let mut state = test_state(make_config("default", "default"));
        let mut mode = InputMode::Filter;
        let theme = ThemeConfig::default();

        for code in [KeyCode::Char('s'), KeyCode::Char('q'), KeyCode::Char('?')] {
            let (new_mode, action) = handle_key(&mut state, mode, &theme, key(code));
            assert_eq!(action, WizardAction::Continue);
            mode = new_mode;
        }

        assert_eq!(state.filter_query(), "sq?");
        assert_eq!(
            mode,
            InputMode::Filter,
            "'s'/'q'/'?' should not exit filter mode"
        );
    }
}
