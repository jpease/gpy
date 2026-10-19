//! `gpy config wizard`: interactive TUI for theme, palette, and segment
//! configuration with a live preview.

mod detail;
mod facts;
mod fuzzy;
mod keys;
mod preview;
mod save;
mod state;
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

/// Re-resolve theme/palette only when the selection changed.
fn refresh_theme_palette_cache(
    state: &state::WizardState,
    cached_selection: &mut Option<(String, String)>,
    cached_theme: &mut std::sync::Arc<crate::theme::ThemeConfig>,
    cached_palette: &mut crate::template::Palette,
) -> Result<()> {
    let current_selection = (
        state.selected_theme().to_owned(),
        state.selected_palette().to_owned(),
    );
    if cached_selection.as_ref() == Some(&current_selection) {
        return Ok(());
    }
    *cached_theme = crate::theme::ThemeManager::new(state.selected_theme())?.get();
    *cached_palette = crate::palette::resolve::palette_by_name(state.selected_palette());
    *cached_selection = Some(current_selection);
    Ok(())
}

/// Per-frame render-loop state.
struct WizardRuntime {
    state: state::WizardState,
    mode: keys::InputMode,
    /// Baseline config; cloned per frame for the preview.
    config: crate::config::Config,
    facts: facts::PreviewFacts,
    // Avoid re-parsing theme/palette files on every keypress.
    cached_selection: Option<(String, String)>,
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
        let initial_theme = crate::theme::ThemeManager::new(config.ui.theme.as_str())?.get();
        let facts = facts::gather(&cwd, &config, &initial_theme)?;
        let state = state::WizardState::new(config.clone());
        let cached_palette = crate::palette::resolve::palette_by_name(state.selected_palette());

        Ok(Self {
            state,
            mode: keys::InputMode::default(),
            config,
            facts,
            cached_selection: None,
            cached_theme: initial_theme,
            cached_palette,
            theme_error: None,
        })
    }
}

/// Render one frame.
///
/// # Errors
///
/// Returns an error if drawing fails or the very first theme load fails;
/// later load failures revert the selection and show in the detail panel.
fn draw_frame(terminal: &mut DefaultTerminal, runtime: &mut WizardRuntime) -> Result<()> {
    if let Err(error) = refresh_theme_palette_cache(
        &runtime.state,
        &mut runtime.cached_selection,
        &mut runtime.cached_theme,
        &mut runtime.cached_palette,
    ) {
        let Some((previous_theme, _)) = runtime.cached_selection.clone() else {
            return Err(error);
        };
        // First line only; the rest is a TOML excerpt.
        let reason = error.to_string();
        runtime.theme_error = Some(format!(
            "Cannot select theme \"{}\": {}",
            runtime.state.selected_theme(),
            reason.lines().next().unwrap_or_default()
        ));
        runtime.state.select_theme(&previous_theme);
    }

    let mut preview_config = runtime.config.clone();
    runtime.state.apply_to(&mut preview_config);
    // Pending theme edits live in `pending_theme()`, not the on-disk theme.
    let preview_theme = runtime
        .state
        .pending_theme()
        .unwrap_or_else(|| runtime.cached_theme.as_ref());
    let preview_lines = preview::render_preview_line(
        &runtime.state,
        &preview_config,
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
        let path = save::save(&runtime.state)?;
        println!("Saved configuration to {path}");
        crate::commands::utils::reload_agent_and_notify();
    }

    Ok(())
}
