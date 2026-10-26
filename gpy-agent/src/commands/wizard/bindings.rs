//! The wizard's key bindings. This one table drives key dispatch, the help
//! overlay, the footer hints, the detail panel's key hints, and the key map in
//! `docs/user/cli-reference.md`.

use super::keys::InputMode;
use super::state::SegmentEdit;
use crate::plugin::BuiltinSegment;
use crossterm::event::KeyCode;

/// What a bound key does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    NextSection,
    PrevSection,
    CursorUp,
    CursorDown,
    /// Select the theme or palette, or toggle the segment, under the cursor.
    Activate,
    OpenFilter,
    /// Clear the filter query and leave filter mode.
    ClearFilter,
    DeleteFilterChar,
    OpenHelp,
    CloseHelp,
    Save,
    /// Exit, or ask first when there are unsaved changes.
    Quit,
    /// Answer the discard prompt with yes.
    Discard,
    /// Bound only while the edit's segment is highlighted.
    Edit(SegmentEdit),
}

struct Binding {
    keys: &'static [KeyCode],
    /// Modes in which `keys` run `command`.
    modes: &'static [InputMode],
    command: Command,
    description: &'static str,
}

impl Binding {
    /// Whether the binding is active in `mode` with `focused` highlighted.
    fn is_active(&self, mode: InputMode, focused: Option<BuiltinSegment>) -> bool {
        self.modes.contains(&mode)
            && match self.command {
                Command::Edit(edit) => focused == Some(edit.segment()),
                _ => true,
            }
    }
}

const LISTS: &[InputMode] = &[InputMode::Normal, InputMode::Filter];
const NORMAL: &[InputMode] = &[InputMode::Normal];
const FILTER: &[InputMode] = &[InputMode::Filter];
const HELP: &[InputMode] = &[InputMode::Help];
const CONFIRM_QUIT: &[InputMode] = &[InputMode::ConfirmQuit];

const fn bind(
    keys: &'static [KeyCode],
    modes: &'static [InputMode],
    command: Command,
    description: &'static str,
) -> Binding {
    Binding {
        keys,
        modes,
        command,
        description,
    }
}

const fn edit(
    keys: &'static [KeyCode],
    segment_edit: SegmentEdit,
    description: &'static str,
) -> Binding {
    bind(keys, NORMAL, Command::Edit(segment_edit), description)
}

/// Every binding, in help and documentation order.
const BINDINGS: &[Binding] = &[
    bind(&[KeyCode::Tab], LISTS, Command::NextSection, "next section"),
    bind(
        &[KeyCode::BackTab],
        LISTS,
        Command::PrevSection,
        "previous section",
    ),
    // Sections render as wrapped lines, so Left/Right alias Up/Down.
    bind(
        &[KeyCode::Up, KeyCode::Left],
        LISTS,
        Command::CursorUp,
        "previous item",
    ),
    bind(
        &[KeyCode::Down, KeyCode::Right],
        LISTS,
        Command::CursorDown,
        "next item",
    ),
    bind(
        &[KeyCode::Char(' '), KeyCode::Enter],
        LISTS,
        Command::Activate,
        "select/toggle",
    ),
    bind(
        &[KeyCode::Char('/')],
        NORMAL,
        Command::OpenFilter,
        "filter all sections",
    ),
    bind(&[KeyCode::Char('?')], NORMAL, Command::OpenHelp, "help"),
    bind(
        &[KeyCode::Char('s')],
        NORMAL,
        Command::Save,
        "save and exit",
    ),
    bind(
        &[KeyCode::Char('q'), KeyCode::Esc],
        NORMAL,
        Command::Quit,
        "quit",
    ),
    edit(
        &[KeyCode::Char('[')],
        SegmentEdit::PrevDirectoryDisplay,
        "previous display style",
    ),
    edit(
        &[KeyCode::Char(']')],
        SegmentEdit::NextDirectoryDisplay,
        "next display style",
    ),
    edit(
        &[KeyCode::Char('['), KeyCode::Char(']')],
        SegmentEdit::ToggleLanguageDisplay,
        "switch icon/text display",
    ),
    edit(
        &[KeyCode::Char('v')],
        SegmentEdit::ToggleLanguageShowVersions,
        "show/hide versions",
    ),
    edit(
        &[KeyCode::Char('['), KeyCode::Char(']')],
        SegmentEdit::ToggleClockTimeFormat,
        "switch 12h/24h time",
    ),
    edit(
        &[KeyCode::Char(':')],
        SegmentEdit::ToggleClockShowSeconds,
        "show/hide seconds",
    ),
    edit(
        &[KeyCode::Char('m')],
        SegmentEdit::ToggleDurationShowMilliseconds,
        "show/hide milliseconds",
    ),
    edit(
        &[KeyCode::Char('b')],
        SegmentEdit::ToggleGitShowBranch,
        "show/hide branch",
    ),
    edit(
        &[KeyCode::Char('a')],
        SegmentEdit::ToggleGitShowAheadBehind,
        "show/hide ahead/behind",
    ),
    edit(
        &[KeyCode::Char('t')],
        SegmentEdit::ToggleGitShowStash,
        "show/hide stash",
    ),
    bind(
        &[KeyCode::Esc],
        FILTER,
        Command::ClearFilter,
        "clear and leave filter",
    ),
    bind(
        &[KeyCode::Backspace],
        FILTER,
        Command::DeleteFilterChar,
        "delete last filter character",
    ),
    bind(
        &[KeyCode::Char('?'), KeyCode::Esc],
        HELP,
        Command::CloseHelp,
        "close help",
    ),
    bind(
        &[KeyCode::Char('y'), KeyCode::Char('Y')],
        CONFIRM_QUIT,
        Command::Discard,
        "discard changes and quit",
    ),
];

/// The command `code` runs in `mode` with `focused` highlighted.
pub fn lookup(mode: InputMode, focused: Option<BuiltinSegment>, code: KeyCode) -> Option<Command> {
    BINDINGS
        .iter()
        .find(|binding| binding.is_active(mode, focused) && binding.keys.contains(&code))
        .map(|binding| binding.command)
}

/// Display name of a key, as shown in the wizard and the docs.
fn key_name(code: KeyCode) -> String {
    match code {
        KeyCode::Tab => "Tab".to_owned(),
        KeyCode::BackTab => "Shift+Tab".to_owned(),
        KeyCode::Up => "Up".to_owned(),
        KeyCode::Down => "Down".to_owned(),
        KeyCode::Left => "Left".to_owned(),
        KeyCode::Right => "Right".to_owned(),
        KeyCode::Enter => "Enter".to_owned(),
        KeyCode::Esc => "Esc".to_owned(),
        KeyCode::Backspace => "Backspace".to_owned(),
        KeyCode::Char(' ') => "Space".to_owned(),
        KeyCode::Char(c) => c.to_string(),
        other => format!("{other:?}"),
    }
}

fn key_names(keys: &[KeyCode], separator: &str) -> String {
    keys.iter()
        .map(|&code| key_name(code))
        .collect::<Vec<_>>()
        .join(separator)
}

/// Keys bound to any of `commands`, for hints like `press [ / ] to cycle`.
pub fn key_hint(commands: &[Command]) -> String {
    let keys: Vec<KeyCode> = BINDINGS
        .iter()
        .filter(|binding| commands.contains(&binding.command))
        .flat_map(|binding| binding.keys.iter().copied())
        .collect();
    key_names(&keys, " / ")
}

/// Help overlay lines: browsing keys, the highlighted segment's edits, filter
/// keys, and how to close the overlay.
pub fn help_lines(focused: Option<BuiltinSegment>) -> Vec<String> {
    let mut lines = Vec::new();
    let mut heading = String::new();
    for binding in BINDINGS {
        let group = match binding.command {
            Command::Edit(edit) if focused == Some(edit.segment()) => {
                format!("{} segment", edit.segment().as_str())
            }
            Command::Edit(_) => continue,
            _ if binding.modes.contains(&InputMode::Normal) => "Browsing".to_owned(),
            _ if binding.modes == FILTER => "While filtering".to_owned(),
            _ if binding.modes == HELP => "In this help".to_owned(),
            _ => continue,
        };
        if group != heading {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.push(group.clone());
            heading = group;
        }
        lines.push(format!(
            "  {:<12}{}",
            key_names(binding.keys, "/"),
            binding.description
        ));
    }
    lines
}

/// Footer key hints for `mode`; the discard prompt has none.
pub fn footer_hints(mode: InputMode) -> String {
    let commands: &[Command] = match mode {
        InputMode::Normal => &[Command::OpenHelp, Command::Save, Command::Quit],
        InputMode::Filter => &[
            Command::NextSection,
            Command::Activate,
            Command::ClearFilter,
        ],
        InputMode::Help => &[Command::CloseHelp],
        InputMode::ConfirmQuit => &[],
    };
    commands
        .iter()
        .filter_map(|&command| {
            BINDINGS
                .iter()
                .find(|binding| binding.command == command && binding.modes.contains(&mode))
        })
        .map(|binding| format!("[{}] {}", key_names(binding.keys, "/"), binding.description))
        .collect::<Vec<_>>()
        .join("  ")
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        reason = "tests fail loudly on impossible String writes"
    )]

    use super::super::keys::{WizardAction, handle_key};
    use super::super::state::{Section, WizardState};
    use super::*;
    use crate::config::Config;
    use crate::theme::ThemeConfig;
    use crossterm::event::{KeyEvent, KeyModifiers};
    use std::fmt::Write as _;

    fn test_state() -> WizardState {
        let names = |list: &[&str]| list.iter().map(|&name| name.to_owned()).collect();
        WizardState::from_parts(
            Config::default(),
            names(&["default", "starship", "text"]),
            names(&["default", "nord"]),
            names(&[
                "clock",
                "duration",
                "language",
                "directory",
                "git",
                "status",
            ]),
        )
    }

    /// Focus Segments with the cursor on `segment`.
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

    /// Everything a key press can visibly change; every edit marks the state dirty.
    fn observable(state: &WizardState, mode: InputMode, action: WizardAction) -> String {
        format!(
            "{mode:?} {action:?} {:?} {} {} {}",
            state.focus(),
            state.segment_cursor(),
            state.filter_query(),
            state.dirty(),
        )
    }

    /// Every key the help overlay, footers or docs list does something in
    /// every mode it is listed for.
    #[test]
    fn every_listed_key_is_handled() {
        let theme = ThemeConfig::default();
        for binding in BINDINGS {
            for &mode in binding.modes {
                for &code in binding.keys {
                    let mut state = test_state();
                    let segment = match binding.command {
                        Command::Edit(edit) => edit.segment().as_str(),
                        _ => "language",
                    };
                    highlight(&mut state, segment);
                    if binding.command == Command::DeleteFilterChar {
                        state.push_filter_char('g');
                    }
                    let before = observable(&state, mode, WizardAction::Continue);

                    let (after_mode, action) = handle_key(
                        &mut state,
                        mode,
                        &theme,
                        KeyEvent::new(code, KeyModifiers::NONE),
                    );

                    assert_ne!(
                        observable(&state, after_mode, action),
                        before,
                        "{code:?} in {mode:?} ({}) changed nothing",
                        binding.description
                    );
                }
            }
        }
    }

    /// A key reaches at most one binding in any mode and focus.
    #[test]
    fn no_key_is_bound_twice_in_one_context() {
        let focuses = [
            None,
            Some(BuiltinSegment::Clock),
            Some(BuiltinSegment::Duration),
            Some(BuiltinSegment::Language),
            Some(BuiltinSegment::Directory),
            Some(BuiltinSegment::Git),
        ];
        let modes = [
            InputMode::Normal,
            InputMode::Filter,
            InputMode::Help,
            InputMode::ConfirmQuit,
        ];
        for mode in modes {
            for focused in focuses {
                let mut seen = Vec::new();
                for binding in BINDINGS.iter().filter(|b| b.is_active(mode, focused)) {
                    for code in binding.keys {
                        assert!(
                            !seen.contains(code),
                            "{code:?} bound twice in {mode:?} with {focused:?} highlighted"
                        );
                        seen.push(*code);
                    }
                }
            }
        }
    }

    #[test]
    fn help_lists_only_the_highlighted_segments_edits() {
        let git = help_lines(Some(BuiltinSegment::Git)).join("\n");
        assert!(git.contains("git segment"), "{git}");
        assert!(git.contains("  b           show/hide branch"), "{git}");
        assert!(!git.contains("show/hide seconds"), "{git}");

        let none = help_lines(None).join("\n");
        assert!(!none.contains("segment\n"), "{none}");
        assert!(none.contains("  Tab         next section"), "{none}");
        assert!(none.contains("  ?/Esc       close help"), "{none}");
    }

    #[test]
    fn footer_hints_name_each_modes_exit_keys() {
        assert_eq!(
            footer_hints(InputMode::Normal),
            "[?] help  [s] save and exit  [q/Esc] quit"
        );
        assert_eq!(footer_hints(InputMode::Help), "[?/Esc] close help");
        assert_eq!(
            footer_hints(InputMode::Filter),
            "[Tab] next section  [Space/Enter] select/toggle  [Esc] clear and leave filter"
        );
        assert_eq!(footer_hints(InputMode::ConfirmQuit), "");
    }

    #[test]
    fn key_hint_joins_every_key_of_the_commands() {
        assert_eq!(
            key_hint(&[
                Command::Edit(SegmentEdit::PrevDirectoryDisplay),
                Command::Edit(SegmentEdit::NextDirectoryDisplay),
            ]),
            "[ / ]"
        );
        assert_eq!(key_hint(&[Command::Activate]), "Space / Enter");
    }

    fn markdown_key_table() -> String {
        let mut table = String::from("| Key | When | Action |\n|---|---|---|\n");
        for binding in BINDINGS {
            let keys = binding
                .keys
                .iter()
                .map(|&code| format!("`{}`", key_name(code)))
                .collect::<Vec<_>>()
                .join(" / ");
            let when = match binding.command {
                Command::Edit(edit) => {
                    format!("browsing, `{}` highlighted", edit.segment().as_str())
                }
                _ => binding
                    .modes
                    .iter()
                    .map(|mode| match mode {
                        InputMode::Normal => "browsing",
                        InputMode::Filter => "filtering",
                        InputMode::Help => "help open",
                        InputMode::ConfirmQuit => "discard prompt",
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
            };
            writeln!(table, "| {keys} | {when} | {} |", binding.description)
                .expect("writing to a String cannot fail");
        }
        table
    }

    #[test]
    fn cli_reference_key_map_matches_bindings() {
        let doc = include_str!("../../../../docs/user/cli-reference.md");
        let table = markdown_key_table();
        assert!(
            doc.contains(&table),
            "docs/user/cli-reference.md `gpy config wizard` key map must be:\n{table}"
        );
    }
}
