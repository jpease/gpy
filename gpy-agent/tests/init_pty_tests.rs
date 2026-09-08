//! PTY-driven coverage of the interactive `gpy-agent init` path (#648, pins
//! #640).
//!
//! `init_command_tests.rs` drives the real binary but always passes
//! `--non-interactive`; the `/dev/tty` confirmation branch, which is the one
//! the installers reach, had no test. These spawn `gpy-agent init` attached
//! to a real pseudo-terminal (via `portable-pty`, which makes the pty the
//! child's controlling terminal so `/dev/tty` resolves to it) and assert on
//! the config the command writes.
//!
//! Sibling of `wizard_pty_tests.rs`; Unix-only because both the pty crate
//! and `/dev/tty` are.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![allow(missing_docs)]

use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// #640's acceptance bound: an overridden init must not wait on the terminal.
const OVERRIDE_DEADLINE: Duration = Duration::from_secs(2);
const EXIT_DEADLINE: Duration = Duration::from_secs(10);
const OUTPUT_DEADLINE: Duration = Duration::from_secs(10);

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

    /// `gpy-agent init` (interactive form: no `--non-interactive`) with
    /// `GPY_NERD_FONT` set to `nerd_font`, attached to a fresh pty.
    fn command(&self, nerd_font: &str) -> CommandBuilder {
        let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_gpy-agent"));
        cmd.arg("init");
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

fn spawn_reader_thread(mut reader: Box<dyn Read + Send>) -> Arc<Mutex<Vec<u8>>> {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&captured);
    thread::spawn(move || {
        let mut buf = [0_u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => sink
                    .lock()
                    .expect("captured output lock")
                    .extend_from_slice(buf.get(..n).expect("n <= buf.len()")),
            }
        }
    });
    captured
}

fn wait_for_exit(child: &mut dyn Child, deadline: Duration) -> portable_pty::ExitStatus {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("poll child exit status") {
            return status;
        }
        assert!(
            start.elapsed() < deadline,
            "gpy-agent init did not exit within {deadline:?}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

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
            "did not observe {:?} within {deadline:?}; captured: {:?}",
            String::from_utf8_lossy(needle),
            String::from_utf8_lossy(&captured.lock().expect("captured output lock"))
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn contains(captured: &Arc<Mutex<Vec<u8>>>, needle: &[u8]) -> bool {
    captured
        .lock()
        .expect("captured output lock")
        .windows(needle.len())
        .any(|w| w == needle)
}

/// What the prompt said its default answer is.
///
/// Read from the hint the command prints (`... so the default is Yes`). The
/// child scans fonts under its own sandboxed `HOME`, so this is the only
/// oracle for "the detected default" that matches what the child saw.
fn prompted_default(captured: &Arc<Mutex<Vec<u8>>>) -> bool {
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

/// Run `init` on a pty and answer its prompt.
///
/// `answer` is written once the confirmation prompt has been printed; `None`
/// sends EOF instead. Returns success, elapsed time, and the default the
/// prompt announced.
fn run_interactive(
    env: &InitEnv,
    nerd_font: &str,
    answer: Option<&[u8]>,
) -> (bool, Duration, bool) {
    let pair = native_pty_system()
        .openpty(PtySize::default())
        .expect("open pty");
    let reader = pair.master.try_clone_reader().expect("clone pty reader");
    let captured = spawn_reader_thread(reader);
    let mut writer = pair.master.take_writer().expect("take pty writer");

    let started = Instant::now();
    let mut child = pair
        .slave
        .spawn_command(env.command(nerd_font))
        .expect("spawn gpy-agent init in pty");
    drop(pair.slave);

    wait_for_output(&captured, PROMPT_MARKER, OUTPUT_DEADLINE);
    // ^D at the start of a line is EOF on a canonical-mode tty.
    let bytes: &[u8] = answer.unwrap_or(b"\x04");
    writer.write_all(bytes).expect("write answer");
    writer.flush().expect("flush answer");

    let status = wait_for_exit(child.as_mut(), EXIT_DEADLINE);
    (
        status.success(),
        started.elapsed(),
        prompted_default(&captured),
    )
}

// ---- #640: a set override is the answer --------------------------------

#[test]
fn override_none_skips_the_prompt_and_writes_ascii() {
    let env = InitEnv::new();
    let pair = native_pty_system()
        .openpty(PtySize::default())
        .expect("open pty");
    let reader = pair.master.try_clone_reader().expect("clone pty reader");
    let captured = spawn_reader_thread(reader);
    // Deliberately no writer use: nothing ever answers.
    let _writer = pair.master.take_writer().expect("take pty writer");

    let started = Instant::now();
    let mut child = pair
        .slave
        .spawn_command(env.command("none"))
        .expect("spawn gpy-agent init in pty");
    drop(pair.slave);

    let status = wait_for_exit(child.as_mut(), OVERRIDE_DEADLINE);
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
    let pair = native_pty_system()
        .openpty(PtySize::default())
        .expect("open pty");
    let reader = pair.master.try_clone_reader().expect("clone pty reader");
    let captured = spawn_reader_thread(reader);
    let _writer = pair.master.take_writer().expect("take pty writer");

    let mut child = pair
        .slave
        .spawn_command(env.command("nerd"))
        .expect("spawn gpy-agent init in pty");
    drop(pair.slave);

    let status = wait_for_exit(child.as_mut(), OVERRIDE_DEADLINE);
    assert!(status.success(), "init must succeed");
    assert!(!contains(&captured, PROMPT_MARKER));
    assert_eq!(show_icons_value(&env.config_path), Some(true));
}

// ---- the interactive path proper ----------------------------------------
//
// `GPY_NERD_FONT=auto` is "no override": detection runs against the host
// and the prompt appears, pre-selecting the detected default.

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
