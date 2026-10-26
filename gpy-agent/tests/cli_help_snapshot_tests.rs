//! Snapshots of the `gpy` and `gpy-agent` help surface. Hidden items
//! (`__complete`) must never appear. Review changes with `cargo insta review`.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

mod common;

use common::cli_harness::CliTestEnv;
use insta::{assert_snapshot, with_settings};

/// Public `gpy` subcommands and nested actions that must stay reachable.
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
    &["config", "list"],
    &["config", "set"],
    &["doctor"],
    &["debug"],
    &["debug", "paths"],
    &["completions"],
];

/// Subcommands that only exist for tests or shell glue and are marked
/// `hide = true`; neither may leak into user-facing help.
const HIDDEN_SUBCOMMANDS: &[&str] = &["__complete"];

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

/// Names listed under `Commands:` in clap help, excluding the implicit `help`.
/// Continuation lines of wrapped descriptions are indented deeper than names.
fn listed_commands(help: &str) -> Vec<String> {
    help.lines()
        .skip_while(|line| *line != "Commands:")
        .skip(1)
        .take_while(|line| !line.is_empty())
        .filter_map(|line| {
            line.strip_prefix("  ")
                .filter(|rest| !rest.starts_with(' '))
        })
        .filter_map(|rest| rest.split_whitespace().next())
        .filter(|name| *name != "help")
        .map(str::to_owned)
        .collect()
}

fn walk_command_tree(env: &CliTestEnv, prefix: &[String], found: &mut Vec<Vec<String>>) {
    let args: Vec<&str> = prefix.iter().map(String::as_str).collect();
    let help = help_of(env, &args);
    assert_no_hidden_items(&help, &format!("gpy {}", prefix.join(" ")));
    for name in listed_commands(&help) {
        let mut path = prefix.to_vec();
        path.push(name);
        found.push(path.clone());
        walk_command_tree(env, &path, found);
    }
}

#[test]
fn gpy_command_tree_lists_public_commands_and_hides_internal_ones() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let mut found = Vec::new();
    walk_command_tree(&env, &[], &mut found);

    for path in GPY_SUBCOMMANDS {
        assert!(
            found.iter().any(|found_path| found_path
                .iter()
                .map(String::as_str)
                .eq(path.iter().copied())),
            "gpy {} missing from the command tree: {found:?}",
            path.join(" ")
        );
    }
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

/// `--force` also lets an import shadow a builtin/plugin name; the
/// help for both import commands must say so.
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
