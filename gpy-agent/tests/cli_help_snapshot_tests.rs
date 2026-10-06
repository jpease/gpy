//! Snapshots of the `gpy` and `gpy-agent` help surface (#648).
//!
//! `--help` is the first thing a user reads, and the previous coverage was a
//! handful of substring checks. These `insta` snapshots pin the whole text
//! for the top level and every public subcommand, so a renamed flag, a
//! dropped description or a leaked hidden command shows up as a diff to
//! review rather than going unnoticed. Hidden items (`__complete`,
//! `__wizard_force_fail`, `theme use --apply-layout`) must never appear.
//!
//! `gpy debug paths --format json` is snapshotted with the isolated
//! environment's root redacted, so the *shape* (every key, in order) is
//! pinned without tying it to a machine.
//!
//! Review a changed snapshot with `cargo insta review` (or set
//! `INSTA_UPDATE=always` once to accept everything).
//!
//! Hermetic (no socket, no agent), so it runs on every platform the CLI
//! builds for (#653).
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![allow(missing_docs)]

mod common;

use common::CliTestEnv;
use insta::{assert_snapshot, with_settings};

/// Every public `gpy` subcommand, with the nested actions users can reach.
const GPY_SUBCOMMANDS: &[&[&str]] = &[
    &["start"],
    &["stop"],
    &["restart"],
    &["status"],
    &["theme"],
    &["theme", "use"],
    &["theme", "import"],
    &["palette"],
    &["palette", "import"],
    &["plugin"],
    &["plugin", "new"],
    &["enable"],
    &["disable"],
    &["segments"],
    &["lang"],
    &["lang", "versions"],
    &["config"],
    &["config", "set"],
    &["doctor"],
    &["debug"],
    &["debug", "paths"],
    &["completions"],
];

/// Subcommands that only exist for tests or shell glue and are marked
/// `hide = true`; neither may leak into user-facing help.
const HIDDEN_SUBCOMMANDS: &[&str] = &["__complete", "__wizard_force_fail"];

fn help_of(env: &CliTestEnv, args: &[&str]) -> String {
    let mut full: Vec<&str> = args.to_vec();
    full.push("--help");
    let result = env.run_gpy(&full).expect("spawn gpy");
    result.assert_success(&format!("gpy {} --help", args.join(" ")));
    assert!(
        result.stderr.is_empty(),
        "help goes to stdout only: {result:?}"
    );
    strip_trailing_spaces(&result.stdout)
}

/// clap pads some wrapped help lines with a trailing space; the repo's
/// pre-commit hook trims trailing whitespace from every committed file, so
/// the snapshots are stored (and compared) without it.
fn strip_trailing_spaces(text: &str) -> String {
    let mut out: String = text
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn assert_no_hidden_items(help: &str, label: &str) {
    for hidden in HIDDEN_SUBCOMMANDS {
        assert!(
            !help.contains(hidden),
            "{label}: hidden item {hidden} leaked into --help:\n{help}"
        );
    }
}

#[test]
fn gpy_top_level_help() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let help = help_of(&env, &[]);
    assert_no_hidden_items(&help, "gpy");
    with_settings!({description => "gpy --help"}, {
        assert_snapshot!("gpy_help", help);
    });
}

#[test]
fn gpy_subcommand_help() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    for args in GPY_SUBCOMMANDS {
        let label = format!("gpy {}", args.join(" "));
        let help = help_of(&env, args);
        assert_no_hidden_items(&help, &label);
        let name = format!("gpy_{}_help", args.join("_"));
        with_settings!({description => label.clone()}, {
            assert_snapshot!(name, help);
        });
    }
}

/// `theme use --apply-layout` is a deprecated alias for `--force` and is
/// `hide = true`; `theme import --apply-layout` is a different, public flag.
#[test]
fn gpy_theme_use_hides_the_deprecated_alias() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let help = help_of(&env, &["theme", "use"]);
    assert!(help.contains("--force"), "{help}");
    assert!(
        !help.contains("--apply-layout"),
        "the deprecated alias must stay hidden:\n{help}"
    );
}

#[test]
fn gpy_agent_top_level_help() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let result = env.run_gpy_agent(&["--help"]).expect("spawn gpy-agent");
    result.assert_success("gpy-agent --help");
    let help = strip_trailing_spaces(&result.stdout);
    assert_no_hidden_items(&help, "gpy-agent");
    with_settings!({description => "gpy-agent --help"}, {
        assert_snapshot!("gpy_agent_help", help);
    });
}

#[test]
fn gpy_version_names_the_binary() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let result = env.run_gpy(&["--version"]).expect("spawn gpy");
    result.assert_success("gpy --version");
    assert_eq!(
        result.stdout.trim(),
        format!("gpy {}", gpy_agent::VERSION),
        "{result:?}"
    );
}

#[test]
fn debug_paths_json_shape() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let result = env
        .run_gpy(&["debug", "paths", "--format", "json"])
        .expect("spawn gpy");
    result.assert_success("gpy debug paths --format json");

    // Every path lives under the isolated root; redact it so the snapshot is
    // the same on every machine. Both spellings cover macOS, where the
    // tempdir may come back as /var/... while the CLI resolves /private/var/...
    let root = env.root().to_string_lossy().into_owned();
    let canonical = env
        .root()
        .canonicalize()
        .map_or_else(|_| root.clone(), |p| p.to_string_lossy().into_owned());
    let redacted = result
        .stdout
        .replace(&canonical, "<ROOT>")
        .replace(&root, "<ROOT>");
    assert!(
        !redacted.contains(&root),
        "root must be fully redacted: {redacted}"
    );

    let parsed: serde_json::Value = serde_json::from_str(&redacted).expect("valid JSON");
    let object = parsed.as_object().expect("a JSON object");
    for key in [
        "runtime_root",
        "socket",
        "shell_registry_dir",
        "cache_root",
        "instant_prompts_dir",
        "theme_export_file",
        "config_path",
        "config_candidates",
        "theme_dir",
    ] {
        assert!(object.contains_key(key), "missing key {key}: {redacted}");
    }

    with_settings!({description => "gpy debug paths --format json (root redacted)"}, {
        assert_snapshot!("gpy_debug_paths_json", redacted);
    });
}

/// Since #691 `--force` also lets an import shadow a builtin/plugin name; the
/// help for both import commands must say so (#821).
#[test]
fn import_force_help_mentions_shadowing_a_builtin() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    for args in [&["theme", "import"][..], &["palette", "import"][..]] {
        let help = help_of(&env, args);
        let flat = help.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            flat.contains("shadow a builtin"),
            "`gpy {} --help` must say --force shadows a builtin:\n{help}",
            args.join(" ")
        );
    }
}
