//! The `gpy` error surface: user mistakes exit `1` with one human-readable
//! `Error:` line, usage errors exit `2`, and a closed stdout never panics.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

mod common;

use common::child_guard::KillOnDrop;
use common::cli_harness::CliTestEnv;
use std::io::Read;
use std::process::{Command, Stdio};

struct MistakeCase {
    args: &'static [&'static str],
    names: &'static str,
}

const MISTAKES: &[MistakeCase] = &[
    MistakeCase {
        args: &["theme", "use", "nope"],
        names: "'nope'",
    },
    MistakeCase {
        args: &["enable", "nope"],
        names: "'nope'",
    },
    MistakeCase {
        args: &["config", "get", "nope.key"],
        names: "nope.key",
    },
    MistakeCase {
        args: &["config", "set", "git.enabled", "maybe"],
        names: "maybe",
    },
    MistakeCase {
        args: &["config", "show", "nosuch"],
        names: "nosuch",
    },
];

#[test]
fn user_mistakes_exit_1_with_one_human_line() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");

    for case in MISTAKES {
        let result = env.run_gpy(case.args).expect("spawn gpy");
        let label = case.args.join(" ");

        assert_eq!(result.exit_code, 1_i32, "gpy {label}: {result:?}");
        assert!(
            result.stdout.is_empty(),
            "gpy {label}: a failure prints nothing on stdout: {result:?}"
        );

        let lines: Vec<&str> = result.stderr.lines().collect();
        assert_eq!(
            lines.len(),
            1,
            "gpy {label}: exactly one stderr line: {result:?}"
        );
        let line = lines.first().copied().unwrap_or_default();
        assert!(
            line.starts_with("Error: "),
            "gpy {label}: the line starts with `Error: `: {result:?}"
        );
        assert!(
            line.contains(case.names),
            "gpy {label}: the message must name {}: {result:?}",
            case.names
        );
        assert!(
            !line.contains("{ message:") && !line.contains("source: None"),
            "gpy {label}: no Debug-formatted enum: {result:?}"
        );
    }
}

#[test]
fn clap_usage_error_exits_2_with_possible_values() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");

    let result = env
        .run_gpy(&["lang", "versions", "maybe"])
        .expect("spawn gpy");
    assert_eq!(result.exit_code, 2_i32, "{result:?}");
    assert!(
        result.stderr.contains("possible values"),
        "clap lists the accepted values: {result:?}"
    );
    assert!(result.stderr.contains("on"), "{result:?}");
    assert!(result.stderr.contains("off"), "{result:?}");
}

#[test]
fn unknown_subcommand_exits_2() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");

    let result = env.run_gpy(&["frobnicate"]).expect("spawn gpy");
    assert_eq!(result.exit_code, 2_i32, "{result:?}");
    assert!(
        result.stderr.contains("unrecognized subcommand"),
        "{result:?}"
    );
}

/// Spawn `gpy <args>` with stdout piped, read `keep` bytes, then drop the
/// reader so the child sees a closed pipe on its next write.
fn run_with_reader_dropped_after(env: &CliTestEnv, args: &[&str], keep: usize) -> i32 {
    let mut child = KillOnDrop::new(
        Command::new(env!("CARGO_BIN_EXE_gpy"))
            .args(args)
            .env("HOME", env.root())
            .env("XDG_CONFIG_HOME", env.xdg_config_home())
            .env("XDG_RUNTIME_DIR", env.xdg_runtime_dir())
            .env("XDG_CACHE_HOME", env.xdg_cache_home())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn gpy"),
    );

    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut first = vec![0_u8; keep];
    stdout.read_exact(&mut first).expect("read the first bytes");
    drop(stdout);

    let output = child.wait_with_output().expect("wait for gpy");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panicked"),
        "gpy {} must not panic on a closed stdout: {stderr}",
        args.join(" ")
    );
    output.status.code().unwrap_or(-1)
}

/// `theme export` is the agent binary's largest stdout writer.
#[test]
fn agent_theme_export_into_a_closed_pipe_does_not_panic() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");

    let mut child = KillOnDrop::new(
        Command::new(env!("CARGO_BIN_EXE_gpy-agent"))
            .args(["theme", "export", "--shell", "fish"])
            .env("HOME", env.root())
            .env("XDG_CONFIG_HOME", env.xdg_config_home())
            .env("XDG_RUNTIME_DIR", env.xdg_runtime_dir())
            .env("XDG_CACHE_HOME", env.xdg_cache_home())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn gpy-agent"),
    );
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut first = [0_u8; 1];
    stdout.read_exact(&mut first).expect("read the first byte");
    drop(stdout);
    let output = child.wait_with_output().expect("wait for gpy-agent");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("PANIC"), "{stderr}");
    let code = output.status.code().unwrap_or(-1_i32);
    assert!(
        code == 0_i32 || code == 1_i32,
        "expected a clean exit, never 101, got {code}"
    );
}

/// `completions bash` (~68 `KiB`) exceeds the 64 `KiB` pipe buffer, so the child
/// is still writing when the reader goes away.
#[test]
fn completions_into_a_closed_pipe_do_not_panic() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");

    for shell in ["bash", "fish"] {
        let code = run_with_reader_dropped_after(&env, &["completions", shell], 1);
        assert!(
            code == 0_i32 || code == 1_i32,
            "completions {shell}: expected a clean exit 0 (or 1), never 101, got {code}"
        );
    }
}

/// A socket path longer than `sun_path` (104 bytes on macOS, 108 on Linux) is
/// refused up front.
#[test]
fn over_long_socket_path_is_refused_with_the_length_and_the_fix() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let long_dir = "d".repeat(150);
    let socket = format!("/tmp/{long_dir}/gpy.sock");
    let socket_len = socket.len();

    for args in [&["status"][..], &["stop"][..]] {
        let label = args.join(" ");
        let result = env
            .run_gpy_with_env(args, &[("GPY_AGENT_SOCKET_PATH", socket.as_str())])
            .expect("spawn gpy");

        let want = if args == ["status"] { 4_i32 } else { 1_i32 };
        assert_eq!(result.exit_code, want, "gpy {label}: {result:?}");
        let line = result.stderr.lines().next().unwrap_or_default();
        assert!(
            line.starts_with("Error: ") && line.contains(&format!("is {socket_len} bytes")),
            "gpy {label}: the first stderr line is the `Error:` line naming the length: {result:?}"
        );
        let second = result.stderr.lines().nth(1).unwrap_or_default();
        assert!(
            second.starts_with("  Remediation:"),
            "gpy {label}: the second stderr line is the remediation: {result:?}"
        );
        for needle in [
            format!("is {socket_len} bytes"),
            "GPY_AGENT_SOCKET_PATH".to_owned(),
            "XDG_RUNTIME_DIR".to_owned(),
        ] {
            assert!(
                result.stderr.contains(&needle),
                "gpy {label}: stderr names `{needle}`: {result:?}"
            );
        }
        assert!(
            !result.stderr.contains("SUN_LEN"),
            "gpy {label}: the kernel's bare error never reaches the user: {result:?}"
        );
    }

    let result = env
        .run_gpy_agent_with_env(&["start"], &[("GPY_AGENT_SOCKET_PATH", socket.as_str())])
        .expect("spawn gpy-agent");
    assert_ne!(result.exit_code, 0_i32, "gpy-agent start: {result:?}");
    assert!(
        result.stderr.contains(&format!("is {socket_len} bytes")),
        "gpy-agent start: stderr names the length: {result:?}"
    );

    // The diagnostic still prints the offending path rather than hiding it.
    let debug_result = env
        .run_gpy_with_env(
            &["debug", "paths", "--format", "kv"],
            &[("GPY_AGENT_SOCKET_PATH", socket.as_str())],
        )
        .expect("spawn gpy");
    assert!(
        debug_result.stdout.contains(&format!("socket={socket}")),
        "gpy debug paths prints the over-long socket: {debug_result:?}"
    );
}

/// 0.6-era `starship` override: `format`/`show_always` on a segment that became a plugin.
const STALE_THEME: &str =
    "[ui]\nseparator = \" \"\n\n[segments.username]\nformat = \"{value}\"\nshow_always = true\n";

fn write_stale_theme(env: &CliTestEnv, name: &str) {
    std::fs::create_dir_all(env.themes_dir()).expect("create themes dir");
    std::fs::write(env.themes_dir().join(format!("{name}.toml")), STALE_THEME)
        .expect("write stale theme");
}

#[test]
fn broken_user_copy_of_builtin_theme_names_the_override_and_recovery_once() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    write_stale_theme(&env, "starship");

    for args in [
        ["theme", "use", "starship"],
        ["theme", "validate", "starship"],
    ] {
        let result = env.run_gpy(&args).expect("spawn gpy");
        let label = args.join(" ");
        let output = format!("{}{}", result.stdout, result.stderr);

        assert_ne!(result.exit_code, 0_i32, "gpy {label}: {result:?}");
        assert_eq!(
            output
                .matches("overrides the builtin theme 'starship'")
                .count(),
            1,
            "gpy {label}: override statement exactly once: {result:?}"
        );
        assert_eq!(
            output.matches("gpy theme reset starship").count(),
            1,
            "gpy {label}: reset hint exactly once: {result:?}"
        );
        assert_eq!(
            output.matches("gpy theme diff starship").count(),
            1,
            "gpy {label}: diff hint exactly once: {result:?}"
        );
    }
}

#[test]
fn broken_user_theme_without_a_builtin_has_no_override_hint() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    write_stale_theme(&env, "my-custom");

    for args in [
        ["theme", "use", "my-custom"],
        ["theme", "validate", "my-custom"],
    ] {
        let result = env.run_gpy(&args).expect("spawn gpy");
        let output = format!("{}{}", result.stdout, result.stderr);

        assert_ne!(result.exit_code, 0_i32, "{args:?}: {result:?}");
        assert!(
            output.contains("my-custom") && !output.contains("overrides the builtin"),
            "{args:?}: no builtin-override hint for a non-builtin theme: {result:?}"
        );
    }
}
