//! `gpy config wizard` — interactive TUI for theme, palette, and segment
//! configuration with a live preview.
//!
//! Terminal setup/teardown lives in this module (`TerminalGuard`); `run()`'s
//! event loop wires the rest together: `state` (selection + navigation),
//! `facts`/`preview` (the live preview line), `detail` (the master/detail
//! panel's per-item description), `keys` (pure key handling), `ui` (widget
//! composition), and `save` (persistence).

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

/// The four terminal I/O operations `TerminalGuard::new` stages through.
///
/// Abstracted so tests can inject failures at any step and observe exactly which cleanup
/// operations ran, in what order — without a real TTY (`cargo test` typically has none, so
/// calling `enable_raw_mode()` for real would itself fail before the scenario under test ever
/// runs).
///
/// [`RealTerminalOps`] is the only production implementation; tests use a
/// recording double (see `tests::RecordingTerminalOps`).
trait TerminalOps {
    /// # Errors
    ///
    /// Returns an error if raw mode cannot be enabled.
    fn enable_raw_mode(&self) -> io::Result<()>;
    /// Best-effort: see [`TerminalGuard`]'s `Drop` note on why failures here
    /// are swallowed rather than propagated.
    fn disable_raw_mode(&self);
    /// # Errors
    ///
    /// Returns an error if the alternate screen cannot be entered.
    fn enter_alt_screen(&self) -> io::Result<()>;
    /// Best-effort: see [`TerminalGuard`]'s `Drop` note on why failures here
    /// are swallowed rather than propagated.
    fn leave_alt_screen(&self);
}

/// [`TerminalOps`] impl that performs the real crossterm calls.
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

/// RAII guard for "raw mode is currently enabled."
///
/// Constructing it enables raw mode; dropping it disables raw mode again — unless
/// [`Self::disarm`] has been called, in which case drop is a no-op because a longer-lived
/// guard has taken over responsibility for restoration.
struct RawModeGuard<'a, Ops: TerminalOps> {
    ops: &'a Ops,
    armed: bool,
}

impl<'a, Ops: TerminalOps> RawModeGuard<'a, Ops> {
    /// # Errors
    ///
    /// Returns an error if raw mode cannot be enabled.
    fn new(ops: &'a Ops) -> Result<Self> {
        ops.enable_raw_mode().map_err(|e| {
            crate::Error::config(format!("Failed to enable terminal raw mode: {e}"))
        })?;
        Ok(Self { ops, armed: true })
    }

    /// Suppress this guard's own `Drop` action: call once responsibility for
    /// undoing raw mode has been handed off elsewhere (`TerminalGuard`'s own
    /// `Drop`), so restoration happens exactly once, not once here and again
    /// later.
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

/// RAII guard for "the alternate screen is currently entered." Mirrors
/// [`RawModeGuard`]; see its docs.
struct AltScreenGuard<'a, Ops: TerminalOps> {
    ops: &'a Ops,
    armed: bool,
}

impl<'a, Ops: TerminalOps> AltScreenGuard<'a, Ops> {
    /// # Errors
    ///
    /// Returns an error if the alternate screen cannot be entered.
    fn new(ops: &'a Ops) -> Result<Self> {
        ops.enter_alt_screen()
            .map_err(|e| crate::Error::config(format!("Failed to enter alternate screen: {e}")))?;
        Ok(Self { ops, armed: true })
    }

    /// See [`RawModeGuard::disarm`].
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

/// The result of successfully completing terminal setup stages 1 and 2 (raw mode, then the
/// alternate screen).
///
/// These are the two stages that must be rolled back if a *later* step (stage 3, initializing
/// the `ratatui` backend) fails, since raw mode / the alt screen are already-applied terminal
/// state at that point with no other owner yet.
/// Field order here is deliberate, not incidental: struct fields drop in
/// declaration order, so `raw_mode` (disabling raw mode) runs before
/// `alt_screen` (leaving the alternate screen) — matching
/// `install_panic_hook`'s restoration order (`disable_raw_mode` then
/// `LeaveAlternateScreen`). This is asserted by
/// `tests::staged_terminal_rolls_back_both_stages_when_dropped_before_finish`.
struct StagedTerminal<'a, Ops: TerminalOps> {
    raw_mode: RawModeGuard<'a, Ops>,
    alt_screen: AltScreenGuard<'a, Ops>,
}

impl<'a, Ops: TerminalOps> StagedTerminal<'a, Ops> {
    /// Enable raw mode, then enter the alternate screen. If entering the
    /// alternate screen fails, raw mode is disabled again before returning
    /// (the local `raw_mode` guard drops, still armed) — the fix for the bug
    /// this module exists to prevent: a step-2 failure used to leak raw mode
    /// because no guard existed yet to roll it back.
    ///
    /// # Errors
    ///
    /// Returns an error if raw mode cannot be enabled or the alternate
    /// screen cannot be entered.
    fn new(ops: &'a Ops) -> Result<Self> {
        let raw_mode = RawModeGuard::new(ops)?;
        let alt_screen = AltScreenGuard::new(ops)?;
        Ok(Self {
            raw_mode,
            alt_screen,
        })
    }

    /// Hand off restoration responsibility to a longer-lived guard: disarm
    /// both stages so this value's own drop (when it goes out of scope right
    /// after this call) is a no-op, then the caller's own guard becomes the
    /// sole owner of "undo raw mode / leave the alt screen on drop."
    fn finish(mut self) {
        self.raw_mode.disarm();
        self.alt_screen.disarm();
    }
}

/// RAII guard that restores the terminal (raw mode + alt screen) on drop.
///
/// Constructing this is the *only* way this module enters raw mode / the alt
/// screen, and dropping it is the *only* way it leaves them — this guarantees
/// teardown runs even if the render loop returns early via `?` or panics,
/// so a wizard crash never leaves the user's shell in a broken terminal state.
///
/// Setup is staged through [`StagedTerminal`] specifically so a failure
/// partway through (entering the alt screen after raw mode is already on, or
/// initializing the backend after both are already on) rolls back whatever
/// already succeeded instead of leaking it — see `new_with_ops`.
struct TerminalGuard<Ops: TerminalOps> {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    ops: Ops,
}

impl TerminalGuard<RealTerminalOps> {
    /// # Errors
    ///
    /// Returns an error if raw mode cannot be enabled, the alternate screen
    /// cannot be entered, or the terminal backend cannot be initialized.
    fn new() -> Result<Self> {
        Self::new_with_ops(RealTerminalOps, || {
            Terminal::new(CrosstermBackend::new(io::stdout()))
        })
    }
}

impl<Ops: TerminalOps> TerminalGuard<Ops> {
    /// The testable core of [`Self::new`]: `ops` supplies the raw-mode /
    /// alt-screen operations (real or recording), and `init_backend` supplies
    /// stage 3 (constructing the `ratatui` backend) as a closure so tests can
    /// force it to fail without touching a real terminal.
    ///
    /// If `init_backend` fails, `staged` (still holding both armed guards)
    /// drops here, rolling back stage 1 and stage 2 in that order — see
    /// [`StagedTerminal`]'s field-order doc. On success, `staged.finish()`
    /// disarms both guards so this struct's own `Drop` is the sole owner of
    /// eventual restoration (proved by
    /// `tests::staged_terminal_finish_does_not_restore`), routed through the
    /// same `ops` this guard was built with (proved by
    /// `tests::terminal_guard_drop_restores_via_ops_on_success_path`).
    ///
    /// # Errors
    ///
    /// Returns an error if raw mode cannot be enabled, the alternate screen
    /// cannot be entered, or `init_backend` fails.
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
        // Best-effort: a Drop impl cannot propagate errors, and failing to
        // restore the terminal is exactly the failure mode this guard exists
        // to prevent, so swallow errors here rather than panicking during
        // unwind (a panic-in-drop during an existing panic aborts the process).
        // Routed through `TerminalOps` (rather than calling crossterm
        // directly) so a `RecordingTerminalOps` double observes teardown the
        // same way it observes setup.
        self.ops.disable_raw_mode();
        self.ops.leave_alt_screen();
    }
}

/// Install a panic hook that restores the terminal (raw mode + alt screen)
/// before the default hook prints the panic message.
///
/// This way a panic mid-render doesn't leave the backtrace printed into a
/// broken alt-screen/raw-mode terminal, then chains into the previous hook.
/// `run()` calls this before `TerminalGuard::new` enters raw mode / the alt
/// screen.
///
/// Installation is process-wide and happens at most once (#461). Each install
/// wraps whatever hook is current, so calling this once per `run()` would
/// nest one restoration closure per invocation — harmless today only because
/// the wizard is a one-shot CLI command and the restoration calls are
/// idempotent, but unbounded growth as soon as the wizard runs more than once
/// in a process.
///
/// Returns `true` when this call performed the installation, `false` when a
/// previous call already did.
fn install_panic_hook() -> bool {
    static INSTALLED: AtomicBool = AtomicBool::new(false);

    if INSTALLED.swap(true, Ordering::SeqCst) {
        return false;
    }

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // A process-wide hook can fire independent of (and can outlive) any
        // specific `TerminalGuard`, so it can't borrow a live guard's `ops`.
        // `RealTerminalOps` is a stateless unit struct, so a fresh one is
        // free to construct here — this goes through the same `TerminalOps`
        // trait methods `TerminalGuard::drop` uses, instead of calling
        // crossterm directly a second time.
        RealTerminalOps.disable_raw_mode();
        RealTerminalOps.leave_alt_screen();
        default_hook(info);
    }));
    true
}

/// Re-resolve the theme/palette pair when `state`'s selection differs from
/// `*cached_selection`, updating all three cache cells in place.
///
/// A no-op when the selection hasn't changed since the last frame — see
/// `run()`'s cache rationale.
///
/// # Errors
///
/// Returns an error if the selected theme fails to load.
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

/// Owns every piece of state the render loop reads or mutates each frame.
///
/// Bundled into one struct so `draw_frame` can be split out of `run()`
/// without tripping `clippy.toml`'s `too-many-arguments-threshold`.
struct WizardRuntime {
    state: state::WizardState,
    mode: keys::InputMode,
    /// The config as loaded at wizard startup; cloned into a fresh
    /// `preview_config` each frame (see `draw_frame`) so the live preview
    /// reflects the in-progress selection without mutating this baseline.
    config: crate::config::Config,
    facts: facts::PreviewFacts,
    // Cache theme/palette resolution keyed on the last-rendered selection, so
    // navigation/toggle keys that don't change theme/palette don't re-parse a
    // theme/palette file every frame. Segment toggles don't need this
    // treatment: `apply_to` is a cheap in-memory field mutation, only the
    // theme/palette *file* loads are worth caching.
    cached_selection: Option<(String, String)>,
    cached_theme: std::sync::Arc<crate::theme::ThemeConfig>,
    cached_palette: crate::template::Palette,
    /// Why the last theme activation was refused (the theme failed to load);
    /// shown in the detail panel until the next key press (#802).
    theme_error: Option<String>,
}

impl WizardRuntime {
    /// # Errors
    ///
    /// Returns an error if the config cannot be read, the configured theme
    /// fails to load, or gathering the initial live-preview facts fails.
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

/// Render one wizard frame: refresh the theme/palette cache, compute the
/// preview/detail lines for the current selection, and draw into `guard`'s
/// terminal.
///
/// # Errors
///
/// Returns an error if drawing the frame fails, or the very first theme load
/// fails (no previously loaded theme to fall back to). A later theme that
/// fails to load is refused instead: the selection reverts to the last theme
/// that loaded and the failure is shown in the detail panel (#802).
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
        // Only the first line: it names the file and the failure; the rest is
        // a TOML excerpt that would not fit the detail panel.
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
    // Theme-scoped picker edits (clock/duration/git, #407) live in
    // `state.pending_theme()`, not `cached_theme` (the unedited on-disk
    // theme) — the preview must render the pending edit, not the on-disk
    // value, the moment the user toggles it.
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

/// Which [`TerminalGuard::new_with_ops`] stage [`run_with_forced_failure`] should force to fail.
///
/// Used only by the `gpy __wizard_force_fail` hidden subcommand, which exists so a PTY-backed
/// integration test can prove `TerminalGuard`'s rollback restores a *real* terminal's termios
/// state (`cargo test` has no TTY, so the in-process unit tests below can only prove the
/// cleanup *call sequence* is correct via `RecordingTerminalOps`, not that a real terminal
/// comes back out of raw mode / the alt screen).
///
#[cfg(all(unix, feature = "test-support"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForcedFailureStage {
    /// Fail entering the alternate screen, after raw mode has genuinely been
    /// enabled — exercises the same rollback as
    /// `new_with_ops_rolls_back_raw_mode_when_alt_screen_fails`.
    AfterRawMode,
    /// Fail initializing the `ratatui` backend, after raw mode and the
    /// alternate screen have both genuinely been entered — exercises the
    /// same rollback as
    /// `new_with_ops_rolls_back_both_stages_when_backend_init_fails`.
    AfterAltScreen,
}

/// [`TerminalOps`] impl that performs the real crossterm calls for every stage except the one
/// [`ForcedFailureStage`] names.
///
/// That named stage fails instead, so the stages before it apply real terminal state for
/// [`run_with_forced_failure`] to prove gets rolled back.
///
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

/// Run [`TerminalGuard::new_with_ops`] against a real terminal, forcing it to
/// fail at `stage`, for the `gpy __wizard_force_fail` hidden subcommand. See
/// [`ForcedFailureStage`] for why this exists.
///
/// # Errors
///
/// Always errors — that is the point (see [`ForcedFailureStage`]).
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
/// Returns an error if the terminal cannot be initialized, or if drawing a
/// frame or reading an input event fails.
pub fn run() -> Result<()> {
    // Return value is only of interest to the idempotence test; a repeat
    // invocation legitimately finds the hook already installed.
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

    // Restore the terminal before anything is printed: output written while the
    // alternate screen is active is discarded when it is left (#801).
    drop(guard);

    if should_save {
        let path = save::save(&runtime.state)?;
        println!("✅ Saved configuration to {path}");
        crate::commands::utils::reload_agent_and_notify();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    #![allow(clippy::missing_panics_doc)]

    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Lets a `Rc<T>` stand in for `T` as a [`TerminalOps`] implementation.
    ///
    /// Only needed by test code: `TerminalGuard::new_with_ops` now takes
    /// owned `ops` (so it can store it for the lifetime of the guard), but a
    /// test that wants to inspect a `RecordingTerminalOps`'s log *after* a
    /// failed `new_with_ops` call (which drops `ops` internally on the error
    /// path, since no `Self` is ever constructed to hold it) needs a clone
    /// that survives the call. `RealTerminalOps`/`FailInjectingOps` don't
    /// need this: ownership transfer of a stateless/single-use struct is
    /// free.
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

    /// One recorded call into [`TerminalOps`], in call order.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Op {
        EnableRawMode,
        DisableRawMode,
        EnterAltScreen,
        LeaveAltScreen,
    }

    /// [`TerminalOps`] test double: records every call in order.
    ///
    /// Can be told to fail `enter_alt_screen` (simulating a stage-2 failure) so tests can
    /// exercise the rollback paths deterministically, without a real TTY.
    ///
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

    /// Failure after raw-mode enablement (stage 2 fails): raw mode must be
    /// disabled again, and the alternate screen must never be entered or
    /// left (it was never entered).
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

    /// #461: `run()` calls `install_panic_hook()` on every invocation, and the hook wraps
    /// whatever hook is current.
    ///
    /// Without a guard, N invocations in one process nest N restoration closures, each calling
    /// the next — unbounded growth the moment the wizard is driven more than once per process
    /// (a restart flow, a long-lived host, an in-process test harness).
    ///
    /// This test claims the process-wide install itself, so it necessarily
    /// leaves the wizard's hook in place; it resets to the default hook
    /// afterwards so a later panic in the same test binary still prints
    /// normally.
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

        // Drop the wizard hook we just installed; take_hook() restores the
        // default.
        let _ = std::panic::take_hook();
    }

    /// Failure after alternate-screen entry (stage 3 — backend init — fails).
    ///
    /// Both raw mode and the alt screen must be rolled back, in the same order
    /// `install_panic_hook` restores them (`disable_raw_mode` then `LeaveAlternateScreen`).
    ///
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

    /// `TerminalGuard`'s own `Drop` now routes through `TerminalOps`.
    ///
    /// It used to call crossterm directly, so a `RecordingTerminalOps`
    /// double can now observe the full success-path lifecycle end-to-end —
    /// not just the failure-path setup rollback the tests above cover.
    #[test]
    fn terminal_guard_drop_restores_via_ops_on_success_path() {
        let ops = Rc::new(RecordingTerminalOps::default());
        let guard = TerminalGuard::new_with_ops(Rc::clone(&ops), || {
            Terminal::new(CrosstermBackend::new(io::stdout()))
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

    /// `StagedTerminal::finish` must disarm both stages so completing setup does not itself
    /// trigger restoration.
    ///
    /// Restoration must happen exactly once, later, when the owning `TerminalGuard` drops (not
    /// once here during construction and again there).
    ///
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

    /// Dropping a `StagedTerminal` without calling `finish()` must roll back both stages in
    /// order.
    ///
    /// Exactly what happens in `new_with_ops` when `init_backend` fails — matching
    /// `new_with_ops_rolls_back_both_stages_when_backend_init_fails` at the `StagedTerminal`
    /// level directly.
    ///
    #[test]
    fn staged_terminal_rolls_back_both_stages_when_dropped_before_finish() {
        let ops = RecordingTerminalOps::default();

        {
            let _staged = StagedTerminal::new(&ops).expect("both stages succeed");
            // Deliberately not calling `.finish()` — simulates stage 3
            // failing before it can be called.
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
