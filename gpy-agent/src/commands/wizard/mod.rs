//! `gpy config wizard`: interactive TUI for theme, palette, and segment
//! configuration with a live preview.

mod bindings;
mod detail;
mod facts;
mod fuzzy;
mod keys;
mod preview;
mod save;
mod state;
mod theme_file;
mod ui;

use crate::Result;
use crossterm::event::{self, Event, KeyEventKind};

use ratatui::DefaultTerminal;

/// Restores the terminal on drop, including on early return.
/// `ratatui::try_init` installs the panic hook that covers panics.
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
    Ok((terminal, RestoreOnDrop))
}

/// Per-frame render-loop state.
struct WizardRuntime {
    state: state::WizardState,
    mode: keys::InputMode,
    /// Startup config with the selection applied, kept for the preview.
    preview_config: crate::config::Config,
    facts: facts::PreviewFacts,
    /// Theme/palette names `cached_theme`/`cached_palette` were loaded for,
    /// so files are re-parsed only when the selection changes.
    cached_selection: (String, String),
    cached_theme: std::sync::Arc<crate::theme::ThemeConfig>,
    cached_palette: crate::template::Palette,
    /// Why the last theme activation failed; cleared on next key.
    theme_error: Option<String>,
}

impl WizardRuntime {
    fn new() -> Result<Self> {
        let cwd = std::env::current_dir().map_err(|e| {
            crate::Error::config(format!("Failed to resolve current directory: {e}"))
        })?;
        let config = crate::commands::utils::read_config()?;
        let cached_theme = crate::theme::ThemeManager::new(config.ui.theme.as_str())?.get();
        let facts = facts::gather(&cwd, &config, &cached_theme)?;
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
            facts,
            cached_selection,
            cached_theme,
            cached_palette,
            theme_error: None,
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
                    // First line only; the rest is a TOML excerpt.
                    let reason = error.to_string();
                    self.theme_error = Some(format!(
                        "Cannot select theme \"{}\": {}",
                        self.state.selected_theme(),
                        reason.lines().next().unwrap_or_default()
                    ));
                    self.state.select_theme(&self.cached_selection.0);
                }
            }
        }
        if self.state.selected_palette() != self.cached_selection.1 {
            self.cached_palette =
                crate::palette::resolve::palette_by_name(self.state.selected_palette());
            self.state
                .selected_palette()
                .clone_into(&mut self.cached_selection.1);
        }
    }
}

/// Render one frame.
///
/// # Errors
///
/// Returns an error if drawing fails.
fn draw_frame(terminal: &mut DefaultTerminal, runtime: &mut WizardRuntime) -> Result<()> {
    runtime.refresh_theme_palette_cache();
    runtime.state.reapply_to(&mut runtime.preview_config);

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
        .draw(|frame| ui::draw(frame, &draw_args))
        .map_err(|e| crate::Error::config(format!("Failed to draw wizard frame: {e}")))?;
    Ok(())
}

/// Run the `gpy config wizard` interactive TUI.
///
/// # Errors
///
/// Returns an error if terminal setup, drawing, or event reading fails.
pub fn run() -> Result<()> {
    let (mut terminal, guard) = init_terminal()?;
    let mut runtime = WizardRuntime::new()?;

    let should_save = loop {
        draw_frame(&mut terminal, &mut runtime)?;

        let Event::Key(key) = event::read()
            .map_err(|e| crate::Error::config(format!("Failed to read terminal event: {e}")))?
        else {
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
