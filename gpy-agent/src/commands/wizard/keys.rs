//! Pure key-event → wizard-action mapping, decoupled from crossterm/ratatui.
//!
//! This makes the mapping unit-testable without a terminal. `wizard/mod.rs`'s
//! render loop is the only caller and does nothing except: read a crossterm
//! `KeyEvent`, call `handle_key`, and act on the returned `WizardAction`.

use super::state::WizardState;
use crate::theme::ThemeConfig;
use crossterm::event::{KeyCode, KeyEvent};

/// What the render loop should do after a key event, decided purely from
/// `WizardState` + the key pressed — no I/O.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardAction {
    /// Keep looping; `state` (and possibly `confirming_quit`) may have changed.
    Continue,
    /// Persist the current selection and exit.
    Save,
    /// Exit without persisting.
    ExitWithoutSaving,
}

/// The wizard's transient UI-flow mode: which non-navigation overlay/prompt
/// (if any) is currently capturing key input.
///
/// Lives alongside `WizardState` in the render loop's owned state (not
/// inside `WizardState` itself) — it's transient UI-flow state, not
/// selection state, and doesn't participate in `WizardState::dirty()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputMode {
    /// Ordinary wizard navigation: Tab/arrows/space/enter/`s`/`q` and the
    /// segment-picker bindings all behave as documented on
    /// `handle_normal_key`.
    #[default]
    Normal,
    /// Showing the "discard changes?" prompt (entered from `Normal` via
    /// `q`/`Esc` while `WizardState::dirty()`). Every key is interpreted as
    /// an answer to that prompt instead of normal wizard navigation.
    ConfirmQuit,
    /// Showing the `?` contextual help overlay. Only `?`/`Esc` (close) are
    /// handled; every other key is swallowed — the overlay never mutates
    /// `WizardState`.
    Help,
    /// In `/` filter-typing mode. The filter query itself is shared across
    /// all three sections (see `WizardState::filter_query`) — this variant
    /// only tracks whether keystrokes are currently routed to editing that
    /// query instead of the normal bindings.
    Filter,
}

/// Handle one key event: a pure reduce from `(mode, key)` to the next mode
/// plus the action the render loop should take.
///
/// `theme` is the currently-selected theme as loaded from disk (the render
/// loop's cached `ThemeConfig`) — the same `base` argument
/// `WizardState::patch_active_theme` expects, needed here because the
/// clock/duration pickers' key bindings (`[`/`]`/`:`/`m`) call it indirectly
/// via `cycle_clock_time_format`/`toggle_clock_show_seconds`/
/// `toggle_duration_show_milliseconds`. Unused outside `InputMode::Normal` —
/// none of the other modes touch theme-scoped fields.
pub fn handle_key(
    state: &mut WizardState,
    mode: InputMode,
    theme: &ThemeConfig,
    key: KeyEvent,
) -> (InputMode, WizardAction) {
    match mode {
        InputMode::ConfirmQuit => handle_confirm_key(key),
        InputMode::Help => handle_help_key(key),
        InputMode::Filter => handle_filter_key(state, key),
        InputMode::Normal => handle_normal_key(state, theme, key),
    }
}

/// Answer the discard-changes confirmation prompt: `y`/`Y`/`Enter` confirms
/// exiting without saving, anything else cancels back to normal navigation.
const fn handle_confirm_key(key: KeyEvent) -> (InputMode, WizardAction) {
    match key.code {
        KeyCode::Char('y' | 'Y') | KeyCode::Enter => {
            (InputMode::Normal, WizardAction::ExitWithoutSaving)
        }
        _ => (InputMode::Normal, WizardAction::Continue),
    }
}

/// Swallow every key while the help overlay is open except `?`/`Esc`, which
/// close it — the overlay never mutates `state`.
const fn handle_help_key(key: KeyEvent) -> (InputMode, WizardAction) {
    if matches!(key.code, KeyCode::Char('?') | KeyCode::Esc) {
        (InputMode::Normal, WizardAction::Continue)
    } else {
        (InputMode::Help, WizardAction::Continue)
    }
}

/// Route a key while `/` filter-typing mode is active.
///
/// `Esc` clears the shared filter query and exits filter mode; `Tab`/`BackTab`
/// switch which section has focus *without* clearing the query or leaving
/// filter mode (the whole point of a single shared filter is that you can
/// freely Tab between sections while it's narrowing all of them — only `Esc`
/// clears it); cursor movement and activation stay live within the
/// focused section's filtered subset; any other printable character is
/// appended to the query.
fn handle_filter_key(state: &mut WizardState, key: KeyEvent) -> (InputMode, WizardAction) {
    match key.code {
        KeyCode::Esc => {
            state.clear_filter();
            return (InputMode::Normal, WizardAction::Continue);
        }
        KeyCode::Tab => state.next_section(),
        KeyCode::BackTab => state.prev_section(),
        KeyCode::Backspace => state.pop_filter_char(),
        KeyCode::Up | KeyCode::Left => state.move_cursor_up(),
        KeyCode::Down | KeyCode::Right => state.move_cursor_down(),
        KeyCode::Char(' ') | KeyCode::Enter => state.activate_cursor(),
        KeyCode::Char(c) => state.push_filter_char(c),
        _ => {}
    }
    (InputMode::Filter, WizardAction::Continue)
}

/// Normal (non-confirm, non-help, non-filter) wizard navigation bindings.
fn handle_normal_key(
    state: &mut WizardState,
    theme: &ThemeConfig,
    key: KeyEvent,
) -> (InputMode, WizardAction) {
    if let Some(action) = handle_segment_picker_key(state, theme, key) {
        return (InputMode::Normal, action);
    }
    match key.code {
        KeyCode::Char('?') => (InputMode::Help, WizardAction::Continue),
        KeyCode::Char('/') => (InputMode::Filter, WizardAction::Continue),
        KeyCode::Tab => {
            state.next_section();
            (InputMode::Normal, WizardAction::Continue)
        }
        KeyCode::BackTab => {
            state.prev_section();
            (InputMode::Normal, WizardAction::Continue)
        }
        KeyCode::Char('s') => (InputMode::Normal, WizardAction::Save),
        KeyCode::Char('q') | KeyCode::Esc => {
            if state.dirty() {
                (InputMode::ConfirmQuit, WizardAction::Continue)
            } else {
                (InputMode::Normal, WizardAction::ExitWithoutSaving)
            }
        }
        // Left/Right are aliases for Up/Down: each section renders its items
        // as a horizontally-wrapped line (see `ui.rs`), so Left/Right matches
        // the visible layout as well as the list-order Up/Down does.
        KeyCode::Up | KeyCode::Left => {
            state.move_cursor_up();
            (InputMode::Normal, WizardAction::Continue)
        }
        KeyCode::Down | KeyCode::Right => {
            state.move_cursor_down();
            (InputMode::Normal, WizardAction::Continue)
        }
        KeyCode::Char(' ') | KeyCode::Enter => {
            state.activate_cursor();
            (InputMode::Normal, WizardAction::Continue)
        }
        _ => (InputMode::Normal, WizardAction::Continue),
    }
}

/// Segment-picker key bindings: cycling a highlighted segment's display-style value (`[`/`]`)
/// or toggling one of its boolean flags (`v`/`:`/`m`/`b`/`a`/`t`).
///
/// Returns `None` for any other key, letting `handle_normal_key` fall through to its own
/// bindings — split out solely to keep `handle_normal_key` under `.clippy.toml`'s
/// `too-many-lines-threshold` once the duration/git pickers' toggles joined
/// directory/language/clock's.
///
/// Cycles the highlighted segment's display-style value when it has one —
/// `"directory"`'s display style (`ui.directory.display`), `"language"`'s
/// (`language.display`), or `"clock"`'s time format
/// (`segments.clock.time_format`, theme-scoped — see
/// `cycle_clock_time_format`); a no-op for any other highlighted segment.
/// Every `cycle_*` call guards internally on its own segment being focused
/// (see `WizardState::focused_builtin`), and only one of the three can ever
/// match at once (the cursor sits on exactly one segment), so calling all
/// three unconditionally is safe. Likewise `v`/`:`/`m` each guard internally
/// on their own segment via `focused_builtin`, so only the highlighted
/// segment's toggle ever takes effect.
///
/// `:` (clock show-seconds) and `m` (duration show-milliseconds) were
/// chosen because every short letter tied to "seconds" collides with an
/// existing binding (`s` is save) — `:` instead evokes the clock's own
/// `HH:MM:SS` punctuation, while `m` is milliseconds' natural mnemonic.
/// The git toggles use `b`ranch/`a`head-behind/s`t`ash (`t` sidesteps the
/// same `s`-is-save collision), each guarding on `focused_builtin`.
fn handle_segment_picker_key(
    state: &mut WizardState,
    theme: &ThemeConfig,
    key: KeyEvent,
) -> Option<WizardAction> {
    match key.code {
        KeyCode::Char('[') => {
            state.cycle_directory_display(false);
            state.cycle_language_display(false);
            state.cycle_clock_time_format(theme);
            Some(WizardAction::Continue)
        }
        KeyCode::Char(']') => {
            state.cycle_directory_display(true);
            state.cycle_language_display(true);
            state.cycle_clock_time_format(theme);
            Some(WizardAction::Continue)
        }
        KeyCode::Char('v') => {
            state.toggle_language_show_versions();
            Some(WizardAction::Continue)
        }
        KeyCode::Char(':') => {
            state.toggle_clock_show_seconds(theme);
            Some(WizardAction::Continue)
        }
        KeyCode::Char('m') => {
            state.toggle_duration_show_milliseconds(theme);
            Some(WizardAction::Continue)
        }
        // The git segment's content-visibility multi-select (#410): each key
        // toggles one independent `GitTheme` `show_*` flag while `"git"` is
        // highlighted, a no-op otherwise (each `toggle_git_*` guards on
        // `focused_builtin`). Mnemonics: `b`ranch, `a`head/behind,
        // s`t`ash (`s` is save).
        KeyCode::Char('b') => {
            state.toggle_git_show_branch(theme);
            Some(WizardAction::Continue)
        }
        KeyCode::Char('a') => {
            state.toggle_git_show_ahead_behind(theme);
            Some(WizardAction::Continue)
        }
        KeyCode::Char('t') => {
            state.toggle_git_show_stash(theme);
            Some(WizardAction::Continue)
        }
        _ => None,
    }
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

    /// Fixed, deterministic stand-in for `WizardState::new`'s real discovery.
    ///
    /// See `state.rs`'s own `FIXED_THEMES`/`FIXED_PALETTES`/`FIXED_SEGMENTS`
    /// for the rationale (#607). Kept in sync with those exact values.
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
    fn confirm_enter_returns_exit_action() {
        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::ConfirmQuit;
        let theme = ThemeConfig::default();

        let (new_mode, action) = handle_key(&mut state, mode, &theme, key(KeyCode::Enter));

        assert_eq!(action, WizardAction::ExitWithoutSaving);
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

    /// Move `state`'s cursor onto the `"directory"` segment within
    /// `Section::Segments`.
    fn focus_directory_segment(state: &mut WizardState) {
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
    }

    #[test]
    fn bracket_keys_cycle_directory_display_when_directory_segment_focused() {
        use crate::config::types::DirectoryDisplay;

        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::Normal;
        let theme = ThemeConfig::default();
        focus_directory_segment(&mut state);
        assert_eq!(state.directory_display(), DirectoryDisplay::Basename);

        let (mode_after_close, action) =
            handle_key(&mut state, mode, &theme, key(KeyCode::Char(']')));
        assert_eq!(action, WizardAction::Continue);
        assert_eq!(mode_after_close, InputMode::Normal);
        assert_eq!(state.directory_display(), DirectoryDisplay::Abbreviated);

        handle_key(
            &mut state,
            mode_after_close,
            &theme,
            key(KeyCode::Char('[')),
        );
        assert_eq!(state.directory_display(), DirectoryDisplay::Basename);
    }

    #[test]
    fn bracket_keys_are_noop_when_directory_segment_not_focused() {
        use crate::config::types::DirectoryDisplay;

        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::Normal;
        let theme = ThemeConfig::default();
        assert_eq!(state.focus(), Section::Theme);

        handle_key(&mut state, mode, &theme, key(KeyCode::Char(']')));

        assert_eq!(state.directory_display(), DirectoryDisplay::Basename);
    }

    /// Move `state`'s cursor onto the `"language"` segment within
    /// `Section::Segments`.
    fn focus_language_segment(state: &mut WizardState) {
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
    }

    #[test]
    fn bracket_keys_cycle_language_display_when_language_segment_focused() {
        use crate::config::types::LanguageDisplay;

        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::Normal;
        let theme = ThemeConfig::default();
        focus_language_segment(&mut state);
        assert_eq!(state.language_display(), LanguageDisplay::Icon);

        let (mode_after_close, action) =
            handle_key(&mut state, mode, &theme, key(KeyCode::Char(']')));
        assert_eq!(action, WizardAction::Continue);
        assert_eq!(mode_after_close, InputMode::Normal);
        assert_eq!(state.language_display(), LanguageDisplay::Text);

        handle_key(
            &mut state,
            mode_after_close,
            &theme,
            key(KeyCode::Char('[')),
        );
        assert_eq!(state.language_display(), LanguageDisplay::Icon);
    }

    #[test]
    fn bracket_keys_do_not_affect_language_display_when_directory_segment_focused() {
        use crate::config::types::LanguageDisplay;

        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::Normal;
        let theme = ThemeConfig::default();
        focus_directory_segment(&mut state);

        handle_key(&mut state, mode, &theme, key(KeyCode::Char(']')));

        assert_eq!(state.language_display(), LanguageDisplay::Icon);
    }

    #[test]
    fn v_key_toggles_language_show_versions_when_language_segment_focused() {
        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::Normal;
        let theme = ThemeConfig::default();
        focus_language_segment(&mut state);
        let before = state.language_show_versions();

        let (new_mode, action) = handle_key(&mut state, mode, &theme, key(KeyCode::Char('v')));

        assert_eq!(action, WizardAction::Continue);
        assert_eq!(new_mode, InputMode::Normal);
        assert_eq!(state.language_show_versions(), !before);
    }

    #[test]
    fn v_key_is_noop_when_language_segment_not_focused() {
        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::Normal;
        let theme = ThemeConfig::default();
        let before = state.language_show_versions();
        assert_eq!(state.focus(), Section::Theme);

        handle_key(&mut state, mode, &theme, key(KeyCode::Char('v')));

        assert_eq!(state.language_show_versions(), before);
    }

    /// Move `state`'s cursor onto the `"clock"` segment within
    /// `Section::Segments`.
    fn focus_clock_segment(state: &mut WizardState) {
        state.next_section(); // Theme -> Palette
        state.next_section(); // Palette -> Segments
        while state
            .available_segments()
            .get(state.segment_cursor())
            .map(String::as_str)
            != Some("clock")
        {
            state.move_cursor_down();
        }
    }

    #[test]
    fn bracket_keys_cycle_clock_time_format_when_clock_segment_focused() {
        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::Normal;
        let theme = ThemeConfig::default();
        focus_clock_segment(&mut state);
        assert_eq!(state.clock_time_format(&theme), "12");

        let (mode_after_close, action) =
            handle_key(&mut state, mode, &theme, key(KeyCode::Char(']')));
        assert_eq!(action, WizardAction::Continue);
        assert_eq!(mode_after_close, InputMode::Normal);
        assert_eq!(state.clock_time_format(&theme), "24");

        handle_key(
            &mut state,
            mode_after_close,
            &theme,
            key(KeyCode::Char('[')),
        );
        assert_eq!(state.clock_time_format(&theme), "12");
    }

    #[test]
    fn bracket_keys_do_not_affect_clock_time_format_when_directory_segment_focused() {
        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::Normal;
        let theme = ThemeConfig::default();
        focus_directory_segment(&mut state);

        handle_key(&mut state, mode, &theme, key(KeyCode::Char(']')));

        assert_eq!(state.clock_time_format(&theme), "12");
    }

    #[test]
    fn colon_key_toggles_clock_show_seconds_when_clock_segment_focused() {
        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::Normal;
        let theme = ThemeConfig::default();
        focus_clock_segment(&mut state);
        let before = state.clock_show_seconds(&theme);

        let (new_mode, action) = handle_key(&mut state, mode, &theme, key(KeyCode::Char(':')));

        assert_eq!(action, WizardAction::Continue);
        assert_eq!(new_mode, InputMode::Normal);
        assert_eq!(state.clock_show_seconds(&theme), !before);
    }

    #[test]
    fn colon_key_is_noop_when_clock_segment_not_focused() {
        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::Normal;
        let theme = ThemeConfig::default();
        let before = state.clock_show_seconds(&theme);
        assert_eq!(state.focus(), Section::Theme);

        handle_key(&mut state, mode, &theme, key(KeyCode::Char(':')));

        assert_eq!(state.clock_show_seconds(&theme), before);
    }

    /// Move `state`'s cursor onto the `"duration"` segment within
    /// `Section::Segments`.
    fn focus_duration_segment(state: &mut WizardState) {
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
    }

    #[test]
    fn m_key_toggles_duration_show_milliseconds_when_duration_segment_focused() {
        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::Normal;
        let theme = ThemeConfig::default();
        focus_duration_segment(&mut state);
        let before = state.duration_show_milliseconds(&theme);

        let (new_mode, action) = handle_key(&mut state, mode, &theme, key(KeyCode::Char('m')));

        assert_eq!(action, WizardAction::Continue);
        assert_eq!(new_mode, InputMode::Normal);
        assert_eq!(state.duration_show_milliseconds(&theme), !before);
    }

    #[test]
    fn m_key_is_noop_when_duration_segment_not_focused() {
        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::Normal;
        let theme = ThemeConfig::default();
        let before = state.duration_show_milliseconds(&theme);
        assert_eq!(state.focus(), Section::Theme);

        handle_key(&mut state, mode, &theme, key(KeyCode::Char('m')));

        assert_eq!(state.duration_show_milliseconds(&theme), before);
    }

    /// Move `state`'s cursor onto the `"git"` segment within
    /// `Section::Segments`.
    fn focus_git_segment(state: &mut WizardState) {
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
    }

    #[test]
    fn git_content_keys_toggle_their_flags_when_git_segment_focused() {
        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::Normal;
        let theme = ThemeConfig::default();
        focus_git_segment(&mut state);

        // b -> branch, a -> ahead/behind, t -> stash.
        for (code, before) in [
            ('b', state.git_show_branch(&theme)),
            ('a', state.git_show_ahead_behind(&theme)),
            ('t', state.git_show_stash(&theme)),
        ] {
            let (new_mode, action) = handle_key(&mut state, mode, &theme, key(KeyCode::Char(code)));
            assert_eq!(action, WizardAction::Continue);
            assert_eq!(new_mode, InputMode::Normal);
            let after = match code {
                'b' => state.git_show_branch(&theme),
                'a' => state.git_show_ahead_behind(&theme),
                _ => state.git_show_stash(&theme),
            };
            assert_eq!(after, !before, "key '{code}' should flip its flag");
        }
    }

    #[test]
    fn git_content_keys_are_noop_when_git_segment_not_focused() {
        let mut state = test_state(make_config("default", "default"));
        let mode = InputMode::Normal;
        let theme = ThemeConfig::default();
        assert_eq!(state.focus(), Section::Theme);

        for code in ['b', 'a', 't'] {
            handle_key(&mut state, mode, &theme, key(KeyCode::Char(code)));
        }

        assert!(state.git_show_branch(&theme));
        assert!(state.git_show_ahead_behind(&theme));
        assert!(state.git_show_stash(&theme));
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
        // Guards against a help-overlay regression short-circuiting the
        // existing dirty-state discard-changes confirmation on Esc.
        let mut state = test_state(make_config("default", "default"));
        state.toggle_segment("git");
        let mode = InputMode::Normal;
        let theme = ThemeConfig::default();

        let (new_mode, action) = handle_key(&mut state, mode, &theme, key(KeyCode::Esc));

        assert_eq!(action, WizardAction::Continue);
        assert_eq!(new_mode, InputMode::ConfirmQuit);
    }

    // --- filter mode tests ---

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
