//! `gpy config wizard`: interactive TUI for theme, palette, and segment
//! configuration with a live preview.

mod bindings;
mod detail;
mod facts;
mod fuzzy;
mod keys;
mod preview;
mod save;
mod signals;
mod state;
mod theme_file;
mod ui;

use crate::Result;
use crossterm::event::{self, Event, KeyEventKind};
use ratatui::DefaultTerminal;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// How often the event loop checks for a termination signal while idle.
const SIGNAL_POLL: Duration = Duration::from_millis(100);

/// Restores the terminal on drop, including on early return.
/// A panic is covered by the hook [`init_terminal`] installs, not by this
/// guard: release builds use `panic = "abort"` (ADR-0008), which runs no
/// destructors, so only a panic hook gets to undo the terminal setup.
struct RestoreOnDrop;

impl Drop for RestoreOnDrop {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

/// Enter raw mode and the alternate screen; undo both if any step fails.
fn init_terminal() -> Result<(DefaultTerminal, RestoreOnDrop)> {
    let terminal = ratatui::try_init().map_err(|e| {
        ratatui::restore();
        crate::Error::config(format!("Failed to initialize terminal: {e}"))
    })?;
    // `try_init` wrapped the panic hook to restore raw mode and the main
    // screen. Drawing hides the cursor and only `Terminal`'s destructor shows
    // it again, which an aborting panic never runs: show it from the hook.
    let hook = std::panic::take_hook();
    std::panic::set_hook(cursor_showing_hook(std::io::stdout(), hook));
    Ok((terminal, RestoreOnDrop))
}

/// A panic hook that runs `previous`, then shows the cursor on `out`.
fn cursor_showing_hook<W>(
    sink: W,
    previous: Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Sync + Send + 'static>,
) -> Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Sync + Send + 'static>
where
    W: std::io::Write + Send + 'static,
{
    let out = std::sync::Mutex::new(sink);
    Box::new(move |info| {
        previous(info);
        // A failed write cannot be reported while panicking.
        if let Ok(mut guard) = out.lock() {
            drop(crossterm::execute!(guard, crossterm::cursor::Show));
        }
    })
}

/// Per-frame render-loop state.
struct WizardRuntime {
    state: state::WizardState,
    mode: keys::InputMode,
    /// Startup config with the selection applied, kept for the preview.
    preview_config: crate::config::Config,
    facts: facts::PreviewFacts,
    /// Directory the preview describes.
    cwd: std::path::PathBuf,
    /// Theme/palette names `cached_theme`/`cached_palette` were loaded for,
    /// so files are re-parsed only when the selection changes.
    cached_selection: (String, String),
    cached_theme: std::sync::Arc<crate::theme::ThemeConfig>,
    cached_palette: crate::template::Palette,
    /// Why the last theme activation failed; cleared on next key.
    theme_error: Option<String>,
    /// Scroll offsets of the master lists, kept across frames.
    list_scroll: ui::ListScroll,
}

impl WizardRuntime {
    fn new() -> Result<Self> {
        let cwd = std::env::current_dir().map_err(|e| {
            crate::Error::config(format!("Failed to resolve current directory: {e}"))
        })?;
        let config = crate::commands::utils::read_config()?;
        let cached_theme = crate::theme::ThemeManager::new(config.ui.theme.as_str())?.get();
        let facts = facts::gather(&cwd, &config, &cached_theme);
        let state = state::WizardState::new(config.clone());
        let cached_palette = crate::palette::resolve::palette_by_name(state.selected_palette());
        let cached_selection = (
            state.selected_theme().to_owned(),
            state.selected_palette().to_owned(),
        );

        Ok(Self {
            state,
            mode: keys::InputMode::default(),
            preview_config: config,
            cwd,
            facts,
            cached_selection,
            cached_theme,
            cached_palette,
            theme_error: None,
            list_scroll: ui::ListScroll::default(),
        })
    }

    /// Re-resolve theme/palette when the selection changed. A theme that
    /// fails to load reverts the selection and is reported in `theme_error`.
    fn refresh_theme_palette_cache(&mut self) {
        if self.state.selected_theme() != self.cached_selection.0 {
            match crate::theme::ThemeManager::new(self.state.selected_theme()) {
                Ok(manager) => {
                    self.cached_theme = manager.get();
                    self.state
                        .selected_theme()
                        .clone_into(&mut self.cached_selection.0);
                }
                Err(error) => {
                    self.theme_error = Some(format!(
                        "Cannot select theme \"{}\": {}",
                        self.state.selected_theme(),
                        first_line(&error.to_string())
                    ));
                    self.state.select_theme(&self.cached_selection.0);
                }
            }
        }
        if let Err(message) = refresh_palette(
            &mut self.state,
            &mut self.cached_selection.1,
            &mut self.cached_palette,
        ) {
            self.theme_error = Some(message);
        }
    }
}

/// Load the palette `state` selects when it differs from `cached_name`. One
/// that fails to load reverts the selection and returns the reason, the way a
/// theme does: otherwise the preview would quietly show the builtin default
/// and the save would be rejected only after the TUI has exited.
fn refresh_palette(
    state: &mut state::WizardState,
    cached_name: &mut String,
    cached: &mut crate::template::Palette,
) -> std::result::Result<(), String> {
    let name = state.selected_palette();
    if name == cached_name {
        return Ok(());
    }
    match crate::palette::resolve::load_palette(name) {
        Ok(palette) => {
            *cached = palette;
            name.clone_into(cached_name);
            Ok(())
        }
        Err(error) => {
            let message = format!(
                "Cannot select palette \"{name}\": {}",
                first_line(&error.to_string())
            );
            state.select_palette(cached_name);
            Err(message)
        }
    }
}

/// First line only; the rest of a load error is a TOML excerpt.
fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}

/// Render one frame.
///
/// # Errors
///
/// Returns an error if drawing fails.
fn draw_frame(terminal: &mut DefaultTerminal, runtime: &mut WizardRuntime) -> Result<()> {
    runtime.refresh_theme_palette_cache();
    runtime.state.reapply_to(&mut runtime.preview_config);
    runtime.facts.refresh_languages(
        &runtime.cwd,
        &runtime.cached_selection.0,
        &runtime.cached_theme,
        &runtime.preview_config.language,
    );
    if runtime.mode == keys::InputMode::Help
        && let Ok(size) = terminal.size()
    {
        let max = ui::help_max_scroll(
            ratatui::layout::Rect::new(0, 0, size.width, size.height),
            runtime.state.focused_builtin(),
        );
        runtime.state.clamp_help_scroll(max);
    }

    // Pending theme edits live in `pending_theme()`, not the on-disk theme.
    let preview_theme = runtime
        .state
        .pending_theme()
        .unwrap_or_else(|| runtime.cached_theme.as_ref());
    let preview_lines = preview::render_preview_line(
        &runtime.state,
        &runtime.preview_config,
        preview_theme,
        &runtime.cached_palette,
        &runtime.facts,
    );
    let mut detail_lines = detail::detail_lines(&runtime.state, &runtime.cached_theme);
    if let Some(message) = &runtime.theme_error {
        detail_lines.push(ratatui::text::Line::from(""));
        detail_lines.push(ratatui::text::Line::from(message.clone()));
    }
    let draw_args = ui::DrawArgs {
        state: &runtime.state,
        mode: runtime.mode,
        preview: &preview_lines,
        detail: &detail_lines,
        colors: ui::WizardColors::from_palette(&runtime.cached_palette),
    };

    terminal
        .draw(|frame| ui::draw(frame, &draw_args, &mut runtime.list_scroll))
        .map_err(|e| crate::Error::config(format!("Failed to draw wizard frame: {e}")))?;
    Ok(())
}

/// Block until a terminal event arrives, or `shutdown` is set (`None`).
fn next_event(shutdown: &AtomicBool) -> Result<Option<Event>> {
    let read_error =
        |e: std::io::Error| crate::Error::config(format!("Failed to read terminal event: {e}"));
    while !shutdown.load(Ordering::SeqCst) {
        if event::poll(SIGNAL_POLL).map_err(read_error)? {
            return event::read().map(Some).map_err(read_error);
        }
    }
    Ok(None)
}

/// Run the `gpy config wizard` interactive TUI.
///
/// SIGINT, SIGTERM and SIGHUP leave without saving, restoring the terminal.
///
/// # Errors
///
/// Returns an error if terminal setup, drawing, or event reading fails.
pub fn run() -> Result<()> {
    // Built before the terminal switches, so slow git and version probes
    // don't leave a blank alternate screen.
    let mut runtime = WizardRuntime::new()?;
    let shutdown = signals::watch();
    let (mut terminal, guard) = init_terminal()?;

    let should_save = loop {
        draw_frame(&mut terminal, &mut runtime)?;

        let Some(Event::Key(key)) = next_event(&shutdown)? else {
            // Terminated by signal, or a non-key event: nothing to save.
            if shutdown.load(Ordering::SeqCst) {
                break false;
            }
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        runtime.theme_error = None;

        let (new_mode, action) =
            keys::handle_key(&mut runtime.state, runtime.mode, &runtime.cached_theme, key);
        runtime.mode = new_mode;
        match action {
            keys::WizardAction::Continue => {}
            keys::WizardAction::Save => break true,
            keys::WizardAction::ExitWithoutSaving => break false,
        }
    };

    // Output written in the alternate screen is discarded when it is left.
    drop(guard);

    if should_save {
        let saved = save::save(&runtime.state)?;
        if let Some(theme) = &saved.theme {
            crate::commands::output::plain(format_args!(
                "Saved theme edits to {}",
                theme.path.display()
            ));
            for note in &theme.notes {
                crate::commands::output::plain(note);
            }
        }
        crate::commands::output::plain(format_args!(
            "Saved configuration to {}",
            saved.config.display()
        ));
        crate::commands::utils::reload_agent_and_notify();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn selecting_a_palette_that_does_not_load_reverts_with_a_reason() {
        let mut state = state::WizardState::from_parts(
            Config::default(),
            vec!["default".to_owned()],
            vec!["default".to_owned(), "no-such-palette".to_owned()],
            vec!["directory".to_owned()],
        );
        let mut cached_name = state.selected_palette().to_owned();
        let mut palette = crate::palette::resolve::palette_by_name(&cached_name);
        let before = palette.clone();

        state.select_palette("no-such-palette");
        let result = refresh_palette(&mut state, &mut cached_name, &mut palette);

        let message = result.expect_err("a palette that does not load is refused");
        assert!(message.contains("no-such-palette"), "{message}");
        assert_eq!(state.selected_palette(), cached_name, "selection reverted");
        assert_eq!(palette, before);
    }

    #[test]
    fn selecting_a_loadable_palette_updates_the_cache() {
        let mut state = state::WizardState::from_parts(
            Config::default(),
            vec!["default".to_owned()],
            vec!["default".to_owned(), "starship".to_owned()],
            vec!["directory".to_owned()],
        );
        let mut cached_name = state.selected_palette().to_owned();
        let mut palette = crate::palette::resolve::palette_by_name(&cached_name);

        state.select_palette("starship");
        let result = refresh_palette(&mut state, &mut cached_name, &mut palette);

        assert_eq!(result, Ok(()));
        assert_eq!(cached_name, "starship");
        assert_eq!(state.selected_palette(), "starship");
    }

    /// A `Write` the test can read back after the hook ran on another thread.
    #[derive(Clone, Default)]
    struct Sink(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("sink lock").extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Under `panic = "abort"` the hook is the last code to run, so it must
    /// show the cursor itself, after the previous hook has restored the screen.
    #[test]
    fn the_panic_hook_shows_the_cursor_after_the_previous_hook() {
        let sink = Sink::default();
        let order = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = std::sync::Arc::clone(&order);
        let probe = sink.clone();
        std::panic::set_hook(cursor_showing_hook(
            sink.clone(),
            Box::new(move |_| {
                let written = probe.0.lock().expect("sink lock").len();
                seen.lock().expect("order lock").push(written);
            }),
        ));

        let panicked = std::thread::spawn(|| panic!("wizard test panic")).join();
        drop(std::panic::take_hook());

        assert!(panicked.is_err());
        assert_eq!(
            *order.lock().expect("order lock"),
            vec![0],
            "previous hook first"
        );
        assert_eq!(*sink.0.lock().expect("sink lock"), b"\x1b[?25h");
    }
}
