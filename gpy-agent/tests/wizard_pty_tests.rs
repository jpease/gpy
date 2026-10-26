//! Wizard terminal restoration on a real pty: `cargo test` has no TTY, so
//! unit tests cannot prove termios and the alternate screen come back.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

use portable_pty::CommandBuilder;
use std::io::Write;
use std::time::Duration;

mod common;
use common::child_guard::PtyChild;
use common::cli_harness::CliTestEnv;
use common::pty::{
    Captured, EXIT_DEADLINE, OUTPUT_DEADLINE, exit_within, open_pty, spawn_reader_thread,
    wait_for_exit, wait_for_output,
};

const ENTER_ALT_SCREEN: &[u8] = b"\x1b[?1049h";
const LEAVE_ALT_SCREEN: &[u8] = b"\x1b[?1049l";

/// Point `cmd`'s environment at `env`'s isolated `HOME`/`XDG_*` dirs.
fn configure_gpy_env(cmd: &mut CommandBuilder, env: &CliTestEnv) {
    cmd.env("HOME", env.root());
    cmd.env("XDG_CONFIG_HOME", env.xdg_config_home());
    cmd.env("XDG_RUNTIME_DIR", env.xdg_runtime_dir());
    cmd.env("XDG_CACHE_HOME", env.xdg_cache_home());
}

fn last_position(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).rposition(|w| w == needle)
}

/// The alternate screen was entered and the *last* enter is followed by a
/// leave. There is no termios flag for it, so this is a substring check.
fn assert_alt_screen_entered_and_left(captured: &Captured) {
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

#[test]
fn clean_exit_restores_terminal() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let (pair, reader) = open_pty();
    let before = pair.master.get_termios();

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_gpy"));
    cmd.args(["config", "wizard"]);
    configure_gpy_env(&mut cmd, &env);

    let captured = spawn_reader_thread(reader);
    let mut writer = pair.master.take_writer().expect("take pty writer");
    let mut child = PtyChild::new(pair.slave.spawn_command(cmd).expect("spawn gpy in pty"));
    drop(pair.slave);

    // Sending a key before the wizard draws would race its startup.
    wait_for_output(&captured, ENTER_ALT_SCREEN, OUTPUT_DEADLINE);

    // `q` exits a pristine session directly; `y` covers a discard
    // confirmation if one appears and is a no-op otherwise.
    writer.write_all(b"q").expect("write quit key");
    writer.flush().expect("flush quit key");
    let exited = exit_within(&mut *child, Duration::from_millis(200)).is_some();
    if !exited {
        writer.write_all(b"y").expect("write confirm key");
        writer.flush().expect("flush confirm key");
    }

    let status = wait_for_exit(&mut *child, EXIT_DEADLINE);
    assert!(status.success(), "clean-exit run should exit successfully");

    let after = pair.master.get_termios();
    assert_eq!(
        before, after,
        "termios state was not restored after a clean exit"
    );
    assert_alt_screen_entered_and_left(&captured);
}

/// The save path end to end: Down to the second theme, Enter, `s`.
#[test]
fn selecting_second_theme_and_saving_writes_config() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");

    // The cursor starts on the active theme (`default`), in `theme list` order.
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

    let (pair, reader) = open_pty();
    let before = pair.master.get_termios();

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_gpy"));
    cmd.args(["config", "wizard"]);
    configure_gpy_env(&mut cmd, &env);

    let captured = spawn_reader_thread(reader);
    let mut writer = pair.master.take_writer().expect("take pty writer");
    let mut child = PtyChild::new(pair.slave.spawn_command(cmd).expect("spawn gpy in pty"));
    drop(pair.slave);

    wait_for_output(&captured, ENTER_ALT_SCREEN, OUTPUT_DEADLINE);
    // Let the first frame render before the keys arrive.
    wait_for_output(&captured, b"default", OUTPUT_DEADLINE);
    writer.write_all(b"\x1b[B").expect("write Down");
    writer.write_all(b"\r").expect("write Enter");
    writer.write_all(b"s").expect("write save key");
    writer.flush().expect("flush keys");

    let status = wait_for_exit(&mut *child, EXIT_DEADLINE);
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

/// Output while the alternate screen is active is discarded on leaving it, so
/// save feedback must come after the last leave sequence.
#[test]
fn save_feedback_printed_after_leaving_alt_screen() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let (pair, reader) = open_pty();

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_gpy"));
    cmd.args(["config", "wizard"]);
    configure_gpy_env(&mut cmd, &env);

    let captured = spawn_reader_thread(reader);
    let mut writer = pair.master.take_writer().expect("take pty writer");
    let mut child = PtyChild::new(pair.slave.spawn_command(cmd).expect("spawn gpy in pty"));
    drop(pair.slave);

    wait_for_output(&captured, ENTER_ALT_SCREEN, OUTPUT_DEADLINE);
    wait_for_output(&captured, b"default", OUTPUT_DEADLINE);
    writer.write_all(b"s").expect("write save key");
    writer.flush().expect("flush save key");

    let status = wait_for_exit(&mut *child, EXIT_DEADLINE);
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

/// A user theme that fails to parse must not end the session.
/// `aaa-broken.toml` sorts before `default`, so Up lands on it.
#[test]
fn wizard_survives_selecting_malformed_theme() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    std::fs::create_dir_all(env.themes_dir()).expect("create user themes dir");
    std::fs::write(
        env.themes_dir().join("aaa-broken.toml"),
        "this is = = not toml\n",
    )
    .expect("write malformed theme");

    let (pair, reader) = open_pty();

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_gpy"));
    cmd.args(["config", "wizard"]);
    configure_gpy_env(&mut cmd, &env);

    let captured = spawn_reader_thread(reader);
    let mut writer = pair.master.take_writer().expect("take pty writer");
    let mut child = PtyChild::new(pair.slave.spawn_command(cmd).expect("spawn gpy in pty"));
    drop(pair.slave);

    wait_for_output(&captured, ENTER_ALT_SCREEN, OUTPUT_DEADLINE);
    wait_for_output(&captured, b"aaa-broken", OUTPUT_DEADLINE);
    writer.write_all(b"\x1b[A").expect("write Up");
    writer.write_all(b"\r").expect("write Enter");
    writer.flush().expect("flush Up/Enter");
    wait_for_output(&captured, b"Cannot", OUTPUT_DEADLINE);
    writer.write_all(b"s").expect("write save key");
    writer.flush().expect("flush save key");

    let status = wait_for_exit(&mut *child, EXIT_DEADLINE);
    assert!(
        status.success(),
        "the wizard must survive a malformed theme; captured: {:?}",
        String::from_utf8_lossy(&captured.lock().expect("captured output lock"))
    );

    let config = std::fs::read_to_string(env.config_path()).expect("read config");
    let theme_line = config
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("theme"))
        .unwrap_or_default()
        .replace(' ', "");
    assert_eq!(
        theme_line, "theme=\"default\"",
        "the previous valid theme is saved; config:\n{config}"
    );
}
