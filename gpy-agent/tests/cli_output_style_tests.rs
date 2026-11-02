//! The shared look of `gpy` reports: underlined headings, one detail indent,
//! and validation hints chosen by failure kind rather than by message text.
#![allow(
    clippy::expect_used,
    reason = "the shared harness in tests/common panics on setup failure"
)]

mod common;

use common::cli_harness::CliTestEnv;

const DETAIL_INDENT: &str = "   ";

#[test]
fn list_commands_open_with_an_underlined_heading() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");

    for args in [
        &["theme", "list"][..],
        &["palette", "list"][..],
        &["segments"][..],
        &["plugin", "list"][..],
        &["config", "list"][..],
    ] {
        let result = env.run_gpy(args).expect("spawn gpy");
        result.assert_success(&args.join(" "));
        let mut lines = result.stdout.lines();
        let title = lines.next().expect("title line");
        assert!(title.ends_with(':'), "gpy {args:?}: {title:?}");
        assert_eq!(
            lines.next(),
            Some("=".repeat(title.chars().count()).as_str()),
            "gpy {args:?}: underline as wide as the title"
        );
        assert_eq!(lines.next(), Some(""), "gpy {args:?}: blank line after");
    }
}

#[test]
fn theme_and_palette_lists_share_one_entry_format() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");

    let themes = env.run_gpy(&["theme", "list"]).expect("spawn gpy");
    assert!(
        themes
            .stdout
            .contains(&format!("\n{DETAIL_INDENT}starship [builtin]\n")),
        "{themes:?}"
    );
    assert!(
        themes
            .stdout
            .contains(&format!("\n{DETAIL_INDENT}default * [")),
        "the active theme carries the marker: {themes:?}"
    );
    let palettes = env.run_gpy(&["palette", "list"]).expect("spawn gpy");
    assert!(
        palettes
            .stdout
            .contains(&format!("\n{DETAIL_INDENT}default * [builtin]\n")),
        "{palettes:?}"
    );
}

#[test]
fn config_list_indents_details_one_level_deeper_than_keys() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");

    let result = env
        .run_gpy(&["config", "list", "git.icons.ahead"])
        .expect("spawn gpy");
    result.assert_success("gpy config list");
    assert!(
        result.stdout.contains(&format!(
            "\n{DETAIL_INDENT}git.icons.ahead [read-write]\n{DETAIL_INDENT}{DETAIL_INDENT}"
        )),
        "{result:?}"
    );
    assert!(result.stdout.ends_with("Total: 1 key\n"), "{result:?}");
}

fn hints_of(stdout: &str) -> Vec<&str> {
    stdout
        .lines()
        .filter_map(|line| line.strip_prefix(&format!("{DETAIL_INDENT}- ")))
        .collect()
}

#[test]
fn theme_failures_get_the_hint_for_their_kind() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let dir = env.root();

    let bad_value = dir.join("bad-value.toml");
    std::fs::write(
        &bad_value,
        "[segments.character]\nsuccess_color = \"not-a-color\"\n",
    )
    .expect("write theme");
    let bad_syntax = dir.join("bad-syntax.toml");
    std::fs::write(&bad_syntax, "[ui\n").expect("write theme");

    let cases = [
        (
            bad_value.to_str().unwrap(),
            "Use named colors, ANSI 0-255, or #RRGGBB values.",
        ),
        (
            bad_syntax.to_str().unwrap(),
            "Run `gpy theme validate <theme-file>` after fixing TOML syntax.",
        ),
        (
            dir.to_str().unwrap(),
            "Pass a theme name or a direct path to a .toml file.",
        ),
        (
            "no-such-theme",
            "Confirm the theme is in ~/.config/gpy/themes or provided by a plugin.",
        ),
    ];
    for (target, hint) in cases {
        let result = env
            .run_gpy(&["theme", "validate", target])
            .expect("spawn gpy");
        assert_eq!(result.exit_code, 1_i32, "{target}: {result:?}");
        assert!(
            hints_of(&result.stdout).contains(&hint),
            "{target}: expected hint {hint:?} in {result:?}"
        );
    }
}

#[test]
fn palette_failures_get_the_hint_for_their_kind() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");
    let dir = env.root();

    let bad_value = dir.join("bad-value.toml");
    std::fs::write(&bad_value, "name = \"x\"\n[colors]\nfoo = \"zzz\"\n").expect("write palette");

    let cases = [
        (
            bad_value.to_str().unwrap(),
            "Use named colors, ANSI 0-255, or #RRGGBB values.",
        ),
        (
            dir.to_str().unwrap(),
            "Pass a palette name or a direct path to a .toml file.",
        ),
        (
            "no-such-palette",
            "Confirm the palette is in ~/.config/gpy/palettes or is a known builtin.",
        ),
    ];
    for (target, hint) in cases {
        let result = env
            .run_gpy(&["palette", "validate", target])
            .expect("spawn gpy");
        assert_eq!(result.exit_code, 1_i32, "{target}: {result:?}");
        assert!(
            hints_of(&result.stdout).contains(&hint),
            "{target}: expected hint {hint:?} in {result:?}"
        );
    }
}

#[test]
fn invalid_names_read_the_same_for_themes_and_palettes() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");

    for (kind, args) in [
        ("theme", &["theme", "use", "../escape"][..]),
        ("palette", &["palette", "use", "../escape"][..]),
        ("theme", &["theme", "new", "../escape"][..]),
        ("theme", &["theme", "save", "../escape"][..]),
    ] {
        let result = env.run_gpy(args).expect("spawn gpy");
        assert_eq!(result.exit_code, 1_i32, "{args:?}: {result:?}");
        assert!(
            result.stderr.contains(&format!(
                "invalid {kind} name '../escape': must not contain path separators, '..', or control characters"
            )),
            "{args:?}: {result:?}"
        );
    }
}

#[test]
fn doctor_reports_use_the_shared_detail_indent() {
    let env = CliTestEnv::new().expect("create isolated CLI test env");

    let result = env.run_gpy(&["doctor"]).expect("spawn gpy");
    assert!(
        result.stdout.contains(&format!(
            "[Plugins]\n{DETAIL_INDENT}Discovered Plugins:\n{DETAIL_INDENT}{DETAIL_INDENT}"
        )),
        "{result:?}"
    );
    assert!(
        result.stdout.starts_with(
            "GPY Doctor - System Health Report\n=================================\n\n"
        ),
        "{result:?}"
    );
}
