//! PTY coverage of the interactive icon prompt of `gpy setup --redetect-icons`,
//! which reads its answer from `/dev/tty`. Unix-only, like the pty crate.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

use portable_pty::CommandBuilder;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tempfile::TempDir;

mod common;
use common::child_guard::PtyChild;
use common::pty::{
    Captured, EXIT_DEADLINE, OUTPUT_DEADLINE, contains, open_pty, spawn_reader_thread,
    wait_for_exit, wait_for_output,
};

/// An overridden detection must not wait on the terminal.
const OVERRIDE_DEADLINE: Duration = Duration::from_secs(2);

const PROMPT_MARKER: &[u8] = b"Do the icons above display correctly";

struct InitEnv {
    _tmp: TempDir,
    home: PathBuf,
    config_path: PathBuf,
}

impl InitEnv {
    fn new() -> Self {
        let tmp = TempDir::new().expect("temp dir");
        let home = tmp.path().to_path_buf();
        let config_path = home.join("gpy-config.toml");
        Self {
            _tmp: tmp,
            home,
            config_path,
        }
    }

    /// `gpy setup --redetect-icons` (interactive) with `GPY_NERD_FONT` set to
    /// `nerd_font`, attached to a fresh pty.
    fn command(&self, nerd_font: &str) -> CommandBuilder {
        let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_gpy"));
        cmd.args(["setup", "--redetect-icons"]);
        cmd.cwd(&self.home);
        cmd.env("HOME", &self.home);
        cmd.env("XDG_CONFIG_HOME", self.home.join(".config"));
        cmd.env("GPY_CONFIG_PATH", &self.config_path);
        cmd.env("GPY_NERD_FONT", nerd_font);
        cmd
    }
}

fn show_icons_value(config_path: &Path) -> Option<bool> {
    let contents = std::fs::read_to_string(config_path).ok()?;
    contents.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("show_icons")?;
        match rest.trim_start_matches([' ', '=']).trim() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }
    })
}

/// The default the prompt announced (`... so the default is Yes`); the child
/// scans fonts under its own sandboxed `HOME`, so this is the only oracle.
fn prompted_default(captured: &Captured) -> bool {
    let text =
        String::from_utf8_lossy(&captured.lock().expect("captured output lock")).into_owned();
    if text.contains("so the default is Yes") {
        true
    } else if text.contains("so the default is No") {
        false
    } else {
        panic!("the prompt must state its default: {text}");
    }
}

/// Run the command on a pty and answer its prompt (`None` sends EOF).
/// Returns success, elapsed time, and the announced default.
fn run_interactive(
    env: &InitEnv,
    nerd_font: &str,
    answer: Option<&[u8]>,
) -> (bool, Duration, bool) {
    let (pair, reader) = open_pty();
    let captured = spawn_reader_thread(reader);
    let mut writer = pair.master.take_writer().expect("take pty writer");

    let started = Instant::now();
    let mut child = PtyChild::new(
        pair.slave
            .spawn_command(env.command(nerd_font))
            .expect("spawn gpy setup in pty"),
    );
    drop(pair.slave);

    wait_for_output(&captured, PROMPT_MARKER, OUTPUT_DEADLINE);
    // ^D at the start of a line is EOF on a canonical-mode tty.
    let bytes: &[u8] = answer.unwrap_or(b"\x04");
    writer.write_all(bytes).expect("write answer");
    writer.flush().expect("flush answer");

    let status = wait_for_exit(&mut *child, EXIT_DEADLINE);
    (
        status.success(),
        started.elapsed(),
        prompted_default(&captured),
    )
}

#[test]
fn override_none_skips_the_prompt_and_writes_ascii() {
    let env = InitEnv::new();
    let (pair, reader) = open_pty();
    let captured = spawn_reader_thread(reader);
    // Nothing ever answers.
    let _writer = pair.master.take_writer().expect("take pty writer");

    let started = Instant::now();
    let mut child = PtyChild::new(
        pair.slave
            .spawn_command(env.command("none"))
            .expect("spawn gpy setup in pty"),
    );
    drop(pair.slave);

    let status = wait_for_exit(&mut *child, OVERRIDE_DEADLINE);
    let elapsed = started.elapsed();
    assert!(status.success(), "init must succeed");
    assert!(
        elapsed < OVERRIDE_DEADLINE,
        "an overridden init must return without waiting on the terminal, took {elapsed:?}"
    );
    assert!(
        !contains(&captured, PROMPT_MARKER),
        "no confirmation prompt may be shown when GPY_NERD_FONT is set: {:?}",
        String::from_utf8_lossy(&captured.lock().expect("captured output lock"))
    );
    assert_eq!(
        show_icons_value(&env.config_path),
        Some(false),
        "GPY_NERD_FONT=none writes show_icons = false"
    );
}

#[test]
fn override_nerd_skips_the_prompt_and_writes_glyphs() {
    let env = InitEnv::new();
    let (pair, reader) = open_pty();
    let captured = spawn_reader_thread(reader);
    let _writer = pair.master.take_writer().expect("take pty writer");

    let mut child = PtyChild::new(
        pair.slave
            .spawn_command(env.command("nerd"))
            .expect("spawn gpy setup in pty"),
    );
    drop(pair.slave);

    let status = wait_for_exit(&mut *child, OVERRIDE_DEADLINE);
    assert!(status.success(), "init must succeed");
    assert!(!contains(&captured, PROMPT_MARKER));
    assert_eq!(show_icons_value(&env.config_path), Some(true));
}

// `GPY_NERD_FONT=auto` is no override: detection runs and the prompt appears.
#[test]
fn answering_y_writes_glyphs() {
    let env = InitEnv::new();
    let (ok, _, _) = run_interactive(&env, "auto", Some(b"y\n"));
    assert!(ok);
    assert_eq!(show_icons_value(&env.config_path), Some(true));
}

#[test]
fn answering_n_writes_ascii() {
    let env = InitEnv::new();
    let (ok, _, _) = run_interactive(&env, "auto", Some(b"n\n"));
    assert!(ok);
    assert_eq!(show_icons_value(&env.config_path), Some(false));
}

#[test]
fn eof_takes_the_detected_default() {
    let env = InitEnv::new();
    let (ok, _, announced_default) = run_interactive(&env, "auto", None);
    assert!(ok);
    assert_eq!(
        show_icons_value(&env.config_path),
        Some(announced_default),
        "EOF at the prompt falls back to the default the prompt announced"
    );
}

#[test]
fn empty_answer_takes_the_detected_default() {
    let env = InitEnv::new();
    let (ok, _, announced_default) = run_interactive(&env, "auto", Some(b"\n"));
    assert!(ok);
    assert_eq!(show_icons_value(&env.config_path), Some(announced_default));
}
