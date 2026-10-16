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
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::{self, Stdout};
use std::sync::atomic::{AtomicBool, Ordering};

/// Terminal I/O behind a trait so tests can inject failures without a TTY.
trait TerminalOps {
    fn enable_raw_mode(&self) -> io::Result<()>;
    /// Best-effort: failures are swallowed (see `TerminalGuard::drop`).
    fn disable_raw_mode(&self);
    fn enter_alt_screen(&self) -> io::Result<()>;
    /// Best-effort, like `disable_raw_mode`.
    fn leave_alt_screen(&self);
}

struct RealTerminalOps;

impl TerminalOps for RealTerminalOps {
    fn enable_raw_mode(&self) -> io::Result<()> {
        enable_raw_mode()
    }

    fn disable_raw_mode(&self) {
        let _ = disable_raw_mode();
    }

    fn enter_alt_screen(&self) -> io::Result<()> {
        execute!(io::stdout(), EnterAlternateScreen)
    }

    fn leave_alt_screen(&self) {
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

/// Disables raw mode on drop unless disarmed.
struct RawModeGuard<'a, Ops: TerminalOps> {
    ops: &'a Ops,
    armed: bool,
}

impl<'a, Ops: TerminalOps> RawModeGuard<'a, Ops> {
    fn new(ops: &'a Ops) -> Result<Self> {
        ops.enable_raw_mode().map_err(|e| {
            crate::Error::config(format!("Failed to enable terminal raw mode: {e}"))
        })?;
        Ok(Self { ops, armed: true })
    }

    /// Hand restoration off to `TerminalGuard` so it happens exactly once.
    const fn disarm(&mut self) {
        self.armed = false;
    }
}

impl<Ops: TerminalOps> Drop for RawModeGuard<'_, Ops> {
    fn drop(&mut self) {
        if self.armed {
            self.ops.disable_raw_mode();
        }
    }
}

/// Leaves the alternate screen on drop unless disarmed.
struct AltScreenGuard<'a, Ops: TerminalOps> {
    ops: &'a Ops,
    armed: bool,
}

impl<'a, Ops: TerminalOps> AltScreenGuard<'a, Ops> {
    fn new(ops: &'a Ops) -> Result<Self> {
        ops.enter_alt_screen()
            .map_err(|e| crate::Error::config(format!("Failed to enter alternate screen: {e}")))?;
        Ok(Self { ops, armed: true })
    }

    const fn disarm(&mut self) {
        self.armed = false;
    }
}

impl<Ops: TerminalOps> Drop for AltScreenGuard<'_, Ops> {
    fn drop(&mut self) {
        if self.armed {
            self.ops.leave_alt_screen();
        }
    }
}

/// Stages 1–2 of setup, rolled back if backend init fails.
/// Field order matters: fields drop in declaration order, so raw mode is
/// disabled before leaving the alt screen, matching `install_panic_hook`.
struct StagedTerminal<'a, Ops: TerminalOps> {
    raw_mode: RawModeGuard<'a, Ops>,
    alt_screen: AltScreenGuard<'a, Ops>,
}

impl<'a, Ops: TerminalOps> StagedTerminal<'a, Ops> {
    fn new(ops: &'a Ops) -> Result<Self> {
        let raw_mode = RawModeGuard::new(ops)?;
        let alt_screen = AltScreenGuard::new(ops)?;
        Ok(Self {
            raw_mode,
            alt_screen,
        })
    }

    fn finish(mut self) {
        self.raw_mode.disarm();
        self.alt_screen.disarm();
    }
}

/// Restores the terminal on drop, including on early return or panic.
struct TerminalGuard<Ops: TerminalOps> {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    ops: Ops,
}

impl TerminalGuard<RealTerminalOps> {
    fn new() -> Result<Self> {
        Self::new_with_ops(RealTerminalOps, || {
            Terminal::new(CrosstermBackend::new(io::stdout()))
        })
    }
}

impl<Ops: TerminalOps> TerminalGuard<Ops> {
    /// `init_backend` is injectable so tests can force stage 3 to fail.
    fn new_with_ops(
        ops: Ops,
        init_backend: impl FnOnce() -> io::Result<Terminal<CrosstermBackend<Stdout>>>,
    ) -> Result<Self> {
        let staged = StagedTerminal::new(&ops)?;
        let terminal = init_backend().map_err(|e| {
            crate::Error::config(format!("Failed to initialize terminal backend: {e}"))
        })?;
        staged.finish();
        Ok(Self { terminal, ops })
    }
}

impl<Ops: TerminalOps> Drop for TerminalGuard<Ops> {
    fn drop(&mut self) {
        // Can't propagate from Drop, and panicking during unwind aborts.
        self.ops.disable_raw_mode();
        self.ops.leave_alt_screen();
    }
}

/// Install (once per process) a panic hook that restores the terminal before
/// printing. Returns whether this call installed it; repeated installs would
/// nest hooks.
fn install_panic_hook() -> bool {
    static INSTALLED: AtomicBool = AtomicBool::new(false);

    if INSTALLED.swap(true, Ordering::SeqCst) {
        return false;
    }

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Process-wide hook may outlive any guard; `RealTerminalOps` is stateless.
        RealTerminalOps.disable_raw_mode();
        RealTerminalOps.leave_alt_screen();
        default_hook(info);
    }));
    true
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
fn draw_frame<Ops: TerminalOps>(
    guard: &mut TerminalGuard<Ops>,
    runtime: &mut WizardRuntime,
) -> Result<()> {
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

    guard
        .terminal
        .draw(|frame| ui::draw(frame, &draw_args))
        .map_err(|e| crate::Error::config(format!("Failed to draw wizard frame: {e}")))?;
    Ok(())
}

/// Stage to fail for `gpy __wizard_force_fail`, which lets a PTY test verify
/// real termios restoration.
#[cfg(all(unix, feature = "test-support"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForcedFailureStage {
    /// Fail entering the alternate screen, after raw mode is enabled.
    AfterRawMode,
    /// Fail backend init, after raw mode and the alternate screen.
    AfterAltScreen,
}

/// Real [`TerminalOps`] that fails at the configured stage.
#[cfg(all(unix, feature = "test-support"))]
struct FailInjectingOps {
    fail_stage: ForcedFailureStage,
}

#[cfg(all(unix, feature = "test-support"))]
impl TerminalOps for FailInjectingOps {
    fn enable_raw_mode(&self) -> io::Result<()> {
        enable_raw_mode()
    }

    fn disable_raw_mode(&self) {
        let _ = disable_raw_mode();
    }

    fn enter_alt_screen(&self) -> io::Result<()> {
        if self.fail_stage == ForcedFailureStage::AfterRawMode {
            return Err(io::Error::other(
                "test-support: forced failure entering alt screen",
            ));
        }
        execute!(io::stdout(), EnterAlternateScreen)
    }

    fn leave_alt_screen(&self) {
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

/// Run terminal setup against a real terminal, failing at `stage`.
///
/// # Errors
///
/// Always errors.
#[cfg(all(unix, feature = "test-support"))]
pub fn run_with_forced_failure(stage: ForcedFailureStage) -> Result<()> {
    let _installed = install_panic_hook();
    let ops = FailInjectingOps { fail_stage: stage };
    let _guard = TerminalGuard::new_with_ops(ops, move || {
        if stage == ForcedFailureStage::AfterAltScreen {
            Err(io::Error::other(
                "test-support: forced failure initializing backend",
            ))
        } else {
            Terminal::new(CrosstermBackend::new(io::stdout()))
        }
    })?;
    Ok(())
}

/// Run the `gpy config wizard` interactive TUI.
///
/// # Errors
///
/// Returns an error if terminal setup, drawing, or event reading fails.
pub fn run() -> Result<()> {
    let _installed = install_panic_hook();

    let mut guard = TerminalGuard::new()?;
    let mut runtime = WizardRuntime::new()?;

    let should_save = loop {
        draw_frame(&mut guard, &mut runtime)?;

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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    #![allow(clippy::missing_panics_doc)]

    use super::*;
    use ratatui::layout::Rect;
    use ratatui::{TerminalOptions, Viewport};
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Lets tests inspect a recorder's log after `new_with_ops` consumed it.
    impl<T: TerminalOps> TerminalOps for Rc<T> {
        fn enable_raw_mode(&self) -> io::Result<()> {
            (**self).enable_raw_mode()
        }

        fn disable_raw_mode(&self) {
            (**self).disable_raw_mode();
        }

        fn enter_alt_screen(&self) -> io::Result<()> {
            (**self).enter_alt_screen()
        }

        fn leave_alt_screen(&self) {
            (**self).leave_alt_screen();
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Op {
        EnableRawMode,
        DisableRawMode,
        EnterAltScreen,
        LeaveAltScreen,
    }

    /// Records calls; optionally fails `enter_alt_screen`.
    #[derive(Default)]
    struct RecordingTerminalOps {
        log: RefCell<Vec<Op>>,
        fail_enter_alt_screen: bool,
    }

    impl RecordingTerminalOps {
        fn failing_at_enter_alt_screen() -> Self {
            Self {
                log: RefCell::new(Vec::new()),
                fail_enter_alt_screen: true,
            }
        }

        fn log(&self) -> Vec<Op> {
            self.log.borrow().clone()
        }
    }

    impl TerminalOps for RecordingTerminalOps {
        fn enable_raw_mode(&self) -> io::Result<()> {
            self.log.borrow_mut().push(Op::EnableRawMode);
            Ok(())
        }

        fn disable_raw_mode(&self) {
            self.log.borrow_mut().push(Op::DisableRawMode);
        }

        fn enter_alt_screen(&self) -> io::Result<()> {
            if self.fail_enter_alt_screen {
                return Err(io::Error::other("forced enter_alt_screen failure"));
            }
            self.log.borrow_mut().push(Op::EnterAltScreen);
            Ok(())
        }

        fn leave_alt_screen(&self) {
            self.log.borrow_mut().push(Op::LeaveAltScreen);
        }
    }

    #[test]
    fn new_with_ops_rolls_back_raw_mode_when_alt_screen_fails() {
        let ops = Rc::new(RecordingTerminalOps::failing_at_enter_alt_screen());

        let result = TerminalGuard::new_with_ops(Rc::clone(&ops), || {
            Terminal::new(CrosstermBackend::new(io::stdout()))
        });

        assert!(result.is_err(), "expected alt-screen failure to propagate");
        assert_eq!(
            ops.log(),
            vec![Op::EnableRawMode, Op::DisableRawMode],
            "raw mode must be enabled then rolled back; alt screen must never be touched"
        );
    }

    /// Leaves the wizard hook installed, then resets to the default hook.
    #[test]
    fn install_panic_hook_installs_at_most_once() {
        assert!(
            install_panic_hook(),
            "the first call must install the restoration hook"
        );
        assert!(
            !install_panic_hook(),
            "a second call must not wrap the hook again"
        );
        assert!(!install_panic_hook(), "further calls must stay no-ops");

        let _ = std::panic::take_hook();
    }

    #[test]
    fn new_with_ops_rolls_back_both_stages_when_backend_init_fails() {
        let ops = Rc::new(RecordingTerminalOps::default());

        let result = TerminalGuard::new_with_ops(Rc::clone(&ops), || {
            Err(io::Error::other("forced backend failure"))
        });

        assert!(
            result.is_err(),
            "expected backend-init failure to propagate"
        );
        assert_eq!(
            ops.log(),
            vec![
                Op::EnableRawMode,
                Op::EnterAltScreen,
                Op::DisableRawMode,
                Op::LeaveAltScreen,
            ],
            "both stages must roll back, disable-raw-mode before leave-alt-screen"
        );
    }

    /// Fixed viewport: `Terminal::new` probes `/dev/tty`, absent on CI.
    #[test]
    fn terminal_guard_drop_restores_via_ops_on_success_path() {
        let ops = Rc::new(RecordingTerminalOps::default());
        let guard = TerminalGuard::new_with_ops(Rc::clone(&ops), || {
            Terminal::with_options(
                CrosstermBackend::new(io::stdout()),
                TerminalOptions {
                    viewport: Viewport::Fixed(Rect::new(0, 0, 80, 24)),
                },
            )
        })
        .expect("both stages and backend init succeed");

        drop(guard);

        assert_eq!(
            ops.log(),
            vec![
                Op::EnableRawMode,
                Op::EnterAltScreen,
                Op::DisableRawMode,
                Op::LeaveAltScreen,
            ],
            "TerminalGuard's own Drop must restore via the same TerminalOps it was built with"
        );
    }

    #[test]
    fn staged_terminal_finish_does_not_restore() {
        let ops = RecordingTerminalOps::default();

        let staged = StagedTerminal::new(&ops).expect("both stages succeed");
        staged.finish();

        assert_eq!(
            ops.log(),
            vec![Op::EnableRawMode, Op::EnterAltScreen],
            "finish() must not itself call disable_raw_mode/leave_alt_screen"
        );
    }

    #[test]
    fn staged_terminal_rolls_back_both_stages_when_dropped_before_finish() {
        let ops = RecordingTerminalOps::default();

        {
            let _staged = StagedTerminal::new(&ops).expect("both stages succeed");
            // No `.finish()`: simulates stage 3 failing.
        }

        assert_eq!(
            ops.log(),
            vec![
                Op::EnableRawMode,
                Op::EnterAltScreen,
                Op::DisableRawMode,
                Op::LeaveAltScreen,
            ]
        );
    }
}
