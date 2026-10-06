//! PTY-level regression coverage for wizard terminal restoration (#460).
//!
//! `#451` fixed `TerminalGuard::new` leaking raw mode / the alternate screen
//! when `gpy config wizard` startup failed partway through. That fix landed
//! with unit tests (`gpy-agent/src/commands/wizard/mod.rs`'s `mod tests`)
//! proving the cleanup *call sequence* is correct via a `RecordingTerminalOps`
//! double — but `cargo test` has no TTY, so no unit test can prove a *real*
//! terminal's termios state actually comes back. These tests supply that
//! proof: they spawn the compiled `gpy` binary attached to a real
//! pseudo-terminal (via `portable-pty`), force a startup failure at each
//! rollback-relevant stage through the hidden `gpy __wizard_force_fail`
//! subcommand, and assert on the pty's real termios state and the raw bytes
//! it wrote.
//!
//! Gated on `all(unix, feature = "test-support")`: the pty crate and the
//! `__wizard_force_fail` subcommand it drives are both Unix-only and
//! `test-support`-only, so under default features (every release build) or
//! on a non-Unix target, this whole file — not just individual tests within
//! it — is simply not compiled. Windows is additionally never exercised via
//! CI (`cross-platform-test.yml`'s `windows-latest` leg only runs
//! `cargo test --locked --lib`, never `tests/`), so this file's exclusion
//! there is doubly guaranteed, not merely implicit.
#![cfg(all(unix, feature = "test-support"))]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![allow(missing_docs)]

use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[path = "common/cli_harness.rs"]
mod cli_harness;
use cli_harness::CliTestEnv;

const ENTER_ALT_SCREEN: &[u8] = b"\x1b[?1049h";
const LEAVE_ALT_SCREEN: &[u8] = b"\x1b[?1049l";
const EXIT_DEADLINE: Duration = Duration::from_secs(10);
const OUTPUT_DEADLINE: Duration = Duration::from_secs(10);

/// Point `cmd`'s environment at `env`'s isolated `HOME`/`XDG_*` dirs.
///
/// The same variables `tests/common/cli_harness.rs::run_command_with_env` sets for every
/// other CLI integration test, translated onto `portable_pty::CommandBuilder` (whose `.env()`
/// mirrors `std::process::Command`: overrides on top of an inherited base environment, not a
/// replacement of it).
fn configure_gpy_env(cmd: &mut CommandBuilder, env: &CliTestEnv) {
    cmd.env("HOME", env.root());
    cmd.env("XDG_CONFIG_HOME", env.xdg_config_home());
    cmd.env("XDG_RUNTIME_DIR", env.xdg_runtime_dir());
    cmd.env("XDG_CACHE_HOME", env.xdg_cache_home());
}

/// Spawn a background thread that drains `reader` into the returned buffer
/// until EOF (i.e. until every copy of the pty slave fd, including the
/// child's, has closed) or a read error.
fn spawn_reader_thread(mut reader: Box<dyn Read + Send>) -> Arc<Mutex<Vec<u8>>> {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let captured_for_thread = Arc::clone(&captured);
    thread::spawn(move || {
        let mut buf = [0_u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => captured_for_thread
                    .lock()
                    .expect("captured output lock")
                    .extend_from_slice(buf.get(..n).expect("read() never returns n > buf.len()")),
            }
        }
    });
    captured
}

/// Poll `child` until it exits or `deadline` elapses (avoids a fixed sleep —
/// this repo has hit flakiness before from fixed-sleep e2e tests).
fn wait_for_exit(child: &mut dyn Child, deadline: Duration) -> portable_pty::ExitStatus {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("poll child exit status") {
            return status;
        }
        assert!(
            start.elapsed() < deadline,
            "gpy did not exit within {deadline:?}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

/// Poll `captured` until it contains `needle` or `deadline` elapses.
fn wait_for_output(captured: &Arc<Mutex<Vec<u8>>>, needle: &[u8], deadline: Duration) {
    let start = Instant::now();
    loop {
        if captured
            .lock()
            .expect("captured output lock")
            .windows(needle.len())
            .any(|w| w == needle)
        {
            return;
        }
        assert!(
            start.elapsed() < deadline,
            "did not observe expected output within {deadline:?}: {:?}",
            String::from_utf8_lossy(&captured.lock().expect("captured output lock"))
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn last_position(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).rposition(|w| w == needle)
}

/// Assert the byte stream shows the alternate screen was entered, and that the *last* enter
/// is followed by a leave — i.e. it doesn't end "still entered".
///
/// There's no termios-style OS flag for this (unlike raw mode); it's purely an
/// escape-sequence protocol, so this is a substring check rather than a full VT parse (not
/// needed for this narrow question).
fn assert_alt_screen_entered_and_left(captured: &Arc<Mutex<Vec<u8>>>) {
    let output = captured.lock().expect("captured output lock").clone();
    let enter = last_position(&output, ENTER_ALT_SCREEN);
    let leave = last_position(&output, LEAVE_ALT_SCREEN);
    assert!(
        enter.is_some(),
        "expected the alternate screen to have been entered; captured: {:?}",
        String::from_utf8_lossy(&output)
    );
    assert!(
        leave.is_some_and(|leave_at| leave_at > enter.expect("checked above")),
        "expected the alternate screen to have been left after the last enter; captured: {:?}",
        String::from_utf8_lossy(&output)
    );
}

/// Open a pty, capture its pristine termios (before anything touches it), and return it
/// alongside the pair.
///
/// So callers can spawn into `slave` and re-query `master` afterward. Snapshotting
/// immediately after `openpty()` (rather than after spawning) avoids a race against the
/// child's own startup.
fn open_pty_and_snapshot_termios() -> (portable_pty::PtyPair, Box<dyn Read + Send>) {
    let pair = native_pty_system()
        .openpty(PtySize::default())
        .expect("open pty");
    let reader = pair.master.try_clone_reader().expect("clone pty reader");
    (pair, reader)
}

#[test]
fn ac1_forced_failure_after_raw_mode_restores_terminal() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let (pair, reader) = open_pty_and_snapshot_termios();
    let before = pair.master.get_termios();

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_gpy"));
    cmd.args(["__wizard_force_fail", "after-raw-mode"]);
    configure_gpy_env(&mut cmd, &env);

    let captured = spawn_reader_thread(reader);
    let mut child = pair.slave.spawn_command(cmd).expect("spawn gpy in pty");
    drop(pair.slave);

    let status = wait_for_exit(child.as_mut(), EXIT_DEADLINE);
    assert!(!status.success(), "forced-failure run should exit non-zero");

    let after = pair.master.get_termios();
    assert_eq!(
        before, after,
        "termios state was not restored after a forced failure entering the alt screen"
    );

    // The alt-screen step is the one forced to fail here, so it should never
    // have been entered at all — nothing to assert about leaving it.
    let output = captured.lock().expect("captured output lock").clone();
    assert!(
        last_position(&output, ENTER_ALT_SCREEN).is_none(),
        "alt screen should never have been entered when the AfterRawMode stage is forced to fail"
    );
}

#[test]
fn ac2_forced_failure_after_alt_screen_restores_terminal() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let (pair, reader) = open_pty_and_snapshot_termios();
    let before = pair.master.get_termios();

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_gpy"));
    cmd.args(["__wizard_force_fail", "after-alt-screen"]);
    configure_gpy_env(&mut cmd, &env);

    let captured = spawn_reader_thread(reader);
    let mut child = pair.slave.spawn_command(cmd).expect("spawn gpy in pty");
    drop(pair.slave);

    let status = wait_for_exit(child.as_mut(), EXIT_DEADLINE);
    assert!(!status.success(), "forced-failure run should exit non-zero");

    let after = pair.master.get_termios();
    assert_eq!(
        before, after,
        "termios state was not restored after a forced failure initializing the backend"
    );
    assert_alt_screen_entered_and_left(&captured);
}

#[test]
fn ac3_clean_exit_restores_terminal() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let (pair, reader) = open_pty_and_snapshot_termios();
    let before = pair.master.get_termios();

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_gpy"));
    cmd.args(["config", "wizard"]);
    configure_gpy_env(&mut cmd, &env);

    let captured = spawn_reader_thread(reader);
    let mut writer = pair.master.take_writer().expect("take pty writer");
    let mut child = pair.slave.spawn_command(cmd).expect("spawn gpy in pty");
    drop(pair.slave);

    // Wait until the wizard is actually drawing before sending a keystroke —
    // sending it too early would race the wizard's own startup.
    wait_for_output(&captured, ENTER_ALT_SCREEN, OUTPUT_DEADLINE);

    // `q`/`Esc` only opens a discard-changes confirmation when the session
    // is dirty (`keys::handle_normal_key`); a pristine session exits
    // directly. Also send `y` shortly after as a harmless fallback in case a
    // confirmation prompt does appear — it isn't bound to anything outside
    // that prompt, so it's a no-op otherwise.
    writer.write_all(b"q").expect("write quit key");
    writer.flush().expect("flush quit key");
    thread::sleep(Duration::from_millis(200));
    if child.try_wait().expect("poll child exit status").is_none() {
        writer.write_all(b"y").expect("write confirm key");
        writer.flush().expect("flush confirm key");
    }

    let status = wait_for_exit(child.as_mut(), EXIT_DEADLINE);
    assert!(status.success(), "clean-exit run should exit successfully");

    let after = pair.master.get_termios();
    assert_eq!(
        before, after,
        "termios state was not restored after a clean exit"
    );
    assert_alt_screen_entered_and_left(&captured);
}

/// The save path end to end (#648).
///
/// Open the wizard, move the theme cursor to the second entry, activate it,
/// press `s`, and check that `ui.theme` on disk is that theme and the
/// terminal came back. `#398` owns the key/state/UI snapshots; this only
/// proves a real session can select and save through a real terminal.
#[test]
fn ac4_select_second_theme_and_save_writes_config() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");

    // The wizard lists themes in discovery order, the same order
    // `gpy theme list` prints; the cursor starts on the active one
    // (`default`), so one Down lands on the second listed theme.
    let listed = env.run_gpy(&["theme", "list"]).expect("spawn gpy");
    listed.assert_success("gpy theme list");
    let themes: Vec<String> = listed
        .stdout
        .lines()
        .filter(|line| line.starts_with("  "))
        .map(|line| {
            line.split_whitespace()
                .next()
                .expect("theme name")
                .to_owned()
        })
        .collect();
    assert!(
        themes.len() >= 2,
        "need at least two discoverable themes, got {themes:?}"
    );
    assert_eq!(themes.first().map(String::as_str), Some("default"));
    let second = themes.get(1).expect("second theme").clone();

    let (pair, reader) = open_pty_and_snapshot_termios();
    let before = pair.master.get_termios();

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_gpy"));
    cmd.args(["config", "wizard"]);
    configure_gpy_env(&mut cmd, &env);

    let captured = spawn_reader_thread(reader);
    let mut writer = pair.master.take_writer().expect("take pty writer");
    let mut child = pair.slave.spawn_command(cmd).expect("spawn gpy in pty");
    drop(pair.slave);

    wait_for_output(&captured, ENTER_ALT_SCREEN, OUTPUT_DEADLINE);
    // Let the first frame render before the keys arrive, then: Down (cursor
    // to the second theme), Enter (activate it), `s` (save and exit).
    wait_for_output(&captured, b"default", OUTPUT_DEADLINE);
    writer.write_all(b"\x1b[B").expect("write Down");
    writer.write_all(b"\r").expect("write Enter");
    writer.write_all(b"s").expect("write save key");
    writer.flush().expect("flush keys");

    let status = wait_for_exit(child.as_mut(), EXIT_DEADLINE);
    assert!(
        status.success(),
        "save-and-exit run should exit successfully; captured: {:?}",
        String::from_utf8_lossy(&captured.lock().expect("captured output lock"))
    );

    let after = pair.master.get_termios();
    assert_eq!(before, after, "termios state was not restored after save");
    assert_alt_screen_entered_and_left(&captured);

    let config = std::fs::read_to_string(env.config_path()).expect("read config");
    let theme_line = config
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("theme"))
        .unwrap_or_default()
        .replace(' ', "");
    assert_eq!(
        theme_line,
        format!("theme=\"{second}\""),
        "the saved config selects the second theme; config:\n{config}"
    );

    let shown = env.run_gpy(&["theme", "show"]).expect("spawn gpy");
    assert_eq!(shown.stdout.trim(), second, "{shown:?}");
}

/// Save feedback must reach the normal screen (#801).
///
/// Anything printed while the alternate screen is active is discarded when the wizard leaves
/// it, so both the save confirmation and the not-reloaded notice (no agent runs in the isolated
/// env) must come after the last leave-alternate-screen sequence.
#[test]
fn ac5_save_feedback_printed_after_leaving_alt_screen() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let (pair, reader) = open_pty_and_snapshot_termios();

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_gpy"));
    cmd.args(["config", "wizard"]);
    configure_gpy_env(&mut cmd, &env);

    let captured = spawn_reader_thread(reader);
    let mut writer = pair.master.take_writer().expect("take pty writer");
    let mut child = pair.slave.spawn_command(cmd).expect("spawn gpy in pty");
    drop(pair.slave);

    wait_for_output(&captured, ENTER_ALT_SCREEN, OUTPUT_DEADLINE);
    wait_for_output(&captured, b"default", OUTPUT_DEADLINE);
    writer.write_all(b"s").expect("write save key");
    writer.flush().expect("flush save key");

    let status = wait_for_exit(child.as_mut(), EXIT_DEADLINE);
    assert!(
        status.success(),
        "save-and-exit run should exit successfully"
    );

    let output = captured.lock().expect("captured output lock").clone();
    let shown = String::from_utf8_lossy(&output);
    let leave = last_position(&output, LEAVE_ALT_SCREEN).expect("alt screen left");
    let saved = last_position(&output, b"Saved configuration to")
        .unwrap_or_else(|| panic!("no save confirmation; captured: {shown:?}"));
    let notice = last_position(&output, b"Agent not reloaded")
        .unwrap_or_else(|| panic!("no not-reloaded notice; captured: {shown:?}"));
    assert!(
        saved > leave,
        "save confirmation must follow leaving the alternate screen; captured: {shown:?}"
    );
    assert!(
        notice > saved,
        "not-reloaded notice must follow the confirmation; captured: {shown:?}"
    );
}
