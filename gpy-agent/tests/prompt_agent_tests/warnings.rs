//! A bad `GPY_*` value warns on stderr once per shell session, however many
//! different ones recur on every prompt, and the warning log records each.

use crate::harness::{Run, Shell};
use serial_test::serial;

/// Two values `gpy prompt` ignores with a warning on every run, with agent
/// mode off so a new shell's first prompt starts no agent.
const BAD: [(&str, &str); 3] = [
    ("GPY_COLOR_DEPTH", "lots"),
    ("GPY_DEBUG_LOG", "debug.log"),
    ("GPY_AGENT_ENABLED", "0"),
];

/// What each of the two warnings names.
const WARNINGS: [&str; 2] = ["GPY_COLOR_DEPTH=\"lots\"", "GPY_DEBUG_LOG=\"debug.log\""];

/// A prompt of the shell `instance` with the [`BAD`] values.
fn prompt(shell: &Shell, instance: &str) -> Run {
    let mut command = shell.command(shell.root());
    command.envs(BAD).args(["--instance", instance]);
    Shell::run(&mut command)
}

/// How many times each of [`WARNINGS`] is in `text`.
fn counts(text: &str) -> [usize; 2] {
    WARNINGS.map(|warning| text.matches(warning).count())
}

#[test]
#[serial]
fn each_bad_value_warns_once_per_shell_session() {
    let shell = Shell::new(r#""directory""#);
    // The cache root the warning log goes in, as a running agent leaves it.
    let cache_root = shell.sandbox.env.xdg_cache_home().join("gpy");
    std::fs::create_dir_all(&cache_root).expect("create the cache root");

    let first = prompt(&shell, "first-shell");
    assert_eq!(
        counts(&first.stderr),
        [1, 1],
        "first prompt: {}",
        first.stderr
    );
    for _ in 0_u8..3 {
        let next = prompt(&shell, "first-shell");
        assert_eq!(
            counts(&next.stderr),
            [0, 0],
            "later prompt: {}",
            next.stderr
        );
    }

    let other = prompt(&shell, "second-shell");
    assert_eq!(
        counts(&other.stderr),
        [1, 1],
        "another shell: {}",
        other.stderr
    );

    let log = std::fs::read_to_string(cache_root.join("agent.log")).expect("read agent.log");
    assert!(
        counts(&log).iter().all(|&count| count > 0),
        "agent.log lacks a warning: {log}"
    );
}
