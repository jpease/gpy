//! `gpy config list`: registry keys and dynamic key families.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests fail loudly on a broken sandbox or an unexpected listing"
)]

mod common;

use common::cli_harness::CliTestEnv;
use std::collections::BTreeSet;

/// Dynamic key families `config list` documents with one `<name>` row each.
const FAMILY_ROWS: &[&str] = &["language.icons.<lang>"];

/// The keys listed in `config list` output (the `   <key> [<access>]` lines).
fn listed_keys(stdout: &str) -> Vec<&str> {
    let key_line =
        regex::Regex::new(r"^   (\S+) \[(?:read-write|read-only )\]$").expect("valid regex");
    stdout
        .lines()
        .filter_map(|line| key_line.captures(line)?.get(1).map(|m| m.as_str()))
        .collect()
}

/// `gpy config list` prints exactly the keys in `CONFIG_KEYS` plus the
/// documented family rows, with a matching footer, and a prefix filter
/// narrows the list.
#[test]
fn config_list_matches_registry() {
    let env = CliTestEnv::new().expect("failed to create CLI test env");

    let all = env.run_gpy(&["config", "list"]).expect("spawn gpy");
    all.assert_success("gpy config list");
    let listed: BTreeSet<&str> = listed_keys(&all.stdout).into_iter().collect();
    let expected: BTreeSet<&str> = gpy_agent::config::metadata::CONFIG_KEYS
        .iter()
        .map(|meta| meta.key)
        .chain(FAMILY_ROWS.iter().copied())
        .collect();
    assert_eq!(
        listed, expected,
        "listed keys differ from CONFIG_KEYS plus the family rows"
    );

    let footer = format!("Total: {} keys", expected.len());
    assert!(
        all.stdout.lines().any(|line| line == footer),
        "missing footer `{footer}` in:\n{}",
        all.stdout
    );

    let filtered = env
        .run_gpy(&["config", "list", "git.icons"])
        .expect("spawn gpy");
    filtered.assert_success("gpy config list git.icons");
    let keys = listed_keys(&filtered.stdout);
    assert!(
        !keys.is_empty(),
        "filter matched nothing:\n{}",
        filtered.stdout
    );
    assert!(
        keys.iter().all(|key| key.starts_with("git.icons.")),
        "filter leaked other keys: {keys:?}"
    );
}

/// `language.icons.<lang>` is settable but not in `CONFIG_KEYS`; the listing
/// shows the family as one row.
#[test]
fn config_list_shows_the_language_icons_family() {
    let env = CliTestEnv::new().expect("failed to create CLI test env");

    for filter in ["language.icons", "language.icons.rust"] {
        let result = env.run_gpy(&["config", "list", filter]).expect("spawn gpy");
        result.assert_success(&format!("gpy config list {filter}"));
        assert_eq!(
            listed_keys(&result.stdout),
            ["language.icons.<lang>"],
            "filter {filter}:\n{}",
            result.stdout
        );
        assert!(
            result.stdout.contains("Total: 1 key"),
            "filter {filter}:\n{}",
            result.stdout
        );
    }
}
