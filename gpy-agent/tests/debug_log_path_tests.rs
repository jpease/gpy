//! `GPY_DEBUG_LOG` as `gpy` resolves it: a relative value is ignored rather
//! than resolved against each command's directory, and an absolute one is
//! written byte for byte.
#![allow(clippy::expect_used, reason = "test setup fails loudly")]

mod common;

use common::cli_harness::CliTestEnv;
use std::ffi::OsStr;
use std::path::Path;
use std::process::Output;

/// A config `gpy prompt` cannot load, so every prompt warns once.
const BROKEN_CONFIG: &str = "[agent\nenabled = false\n";

/// `gpy prompt bash` in `dir` with agent mode off and `GPY_DEBUG_LOG` set to
/// `debug_log`.
fn prompt_with_debug_log(env: &CliTestEnv, dir: &Path, debug_log: &OsStr) -> Output {
    env.gpy_command()
        .args(["prompt", "bash", "--status", "0", "--pid"])
        .arg(std::process::id().to_string())
        .current_dir(dir)
        .env("PWD", dir)
        .env("GPY_AGENT_ENABLED", "0")
        .env("GPY_DEBUG_LOG", debug_log)
        .output()
        .expect("run gpy prompt")
}

/// A relative `GPY_DEBUG_LOG` would put a log in every directory a prompt
/// is drawn in: it is ignored, with a warning that names it, and warnings
/// go to `agent.log` instead.
#[test]
fn relative_debug_log_is_ignored_with_a_warning() {
    let env = CliTestEnv::with_config(BROKEN_CONFIG).expect("test env");
    let dir = env.root().join("project");
    std::fs::create_dir_all(&dir).expect("mkdir project");
    let cache_root = env.xdg_cache_home().join("gpy");
    std::fs::create_dir_all(&cache_root).expect("mkdir cache root");

    let output = prompt_with_debug_log(&env, &dir, OsStr::new("trace.log"));

    assert!(output.status.success(), "{output:?}");
    assert!(
        !dir.join("trace.log").exists(),
        "a relative GPY_DEBUG_LOG must not be resolved against the prompt's directory"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ignoring GPY_DEBUG_LOG=\"trace.log\": expected an absolute path"),
        "{stderr}"
    );
    let agent_log =
        std::fs::read_to_string(cache_root.join("agent.log")).expect("warnings go to agent.log");
    assert!(agent_log.contains("Config loading"), "{agent_log}");
}

/// A `GPY_DEBUG_LOG` that is not UTF-8 names the file the log is written to,
/// unchanged. Linux only: APFS refuses non-UTF-8 file names.
#[cfg(target_os = "linux")]
#[test]
fn non_utf8_debug_log_is_written_where_it_points() {
    use std::os::unix::ffi::OsStrExt as _;

    let env = CliTestEnv::with_config(BROKEN_CONFIG).expect("test env");
    let log = env.root().join(OsStr::from_bytes(b"trace-\xff.log"));

    let output = prompt_with_debug_log(&env, env.root(), log.as_os_str());

    assert!(output.status.success(), "{output:?}");
    let written = std::fs::read_to_string(&log).expect("the log at the exact non-UTF-8 path");
    assert!(written.contains("Config loading"), "{written}");
}
