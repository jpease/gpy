use super::run_gpy_command;
use crate::cli_harness::CliTestEnv;

#[test]
fn test_completions_fish() {
    let output =
        run_gpy_command(&["completions", "fish"]).expect("Failed to generate fish completions");
    assert!(
        output.contains("function __fish_gpy"),
        "Fish completions should define functions"
    );
    assert!(
        output.contains("complete -c gpy"),
        "Fish completions should have complete commands"
    );
}

#[test]
fn test_completions_bash() {
    let output =
        run_gpy_command(&["completions", "bash"]).expect("Failed to generate bash completions");
    assert!(!output.is_empty(), "Bash completions should not be empty");
    assert!(
        output.contains("_gpy") || output.contains("complete"),
        "Bash completions should define completion functions"
    );
}

#[test]
fn test_completions_zsh() {
    let output =
        run_gpy_command(&["completions", "zsh"]).expect("Failed to generate zsh completions");
    assert!(!output.is_empty(), "Zsh completions should not be empty");
    assert!(
        output.contains("#compdef"),
        "Zsh completions should start with #compdef"
    );
}

/// `hide`-marked items (e.g. `__complete`) must not leak into completions; `clap_complete` ignores `is_hide_set()`.
/// The visible `theme import --apply-layout` must still complete, confined to `theme import`.
#[test]
fn test_completions_omit_hidden_items() {
    for shell in ["fish", "bash", "zsh"] {
        let output = run_gpy_command(&["completions", shell])
            .unwrap_or_else(|e| panic!("Failed to generate {shell} completions: {e}"));
        assert!(
            !output.contains("__complete"),
            "{shell} completions must not leak the hidden __complete command:\n{output}"
        );
        assert_apply_layout_confined_to_theme_import(shell, &output);
        assert!(
            output.contains("theme"),
            "{shell} completions should still offer the visible `theme` command:\n{output}"
        );
        assert!(
            output.contains("force"),
            "{shell} completions should still offer the visible `--force` flag:\n{output}"
        );
        assert!(
            output.contains("apply-layout"),
            "{shell} completions should still offer the visible `theme import --apply-layout` flag:\n{output}"
        );
    }
}

/// Every `apply-layout` occurrence must fall under the `import` subcommand
/// block, never `use`; each generator marks subcommand blocks differently.
fn assert_apply_layout_confined_to_theme_import(shell: &str, output: &str) {
    match shell {
        "fish" => {
            for line in output.lines().filter(|l| l.contains("apply-layout")) {
                assert!(
                    line.contains("from import") && !line.contains("from use"),
                    "fish completion for apply-layout must be scoped to `theme import`, not `theme use`:\n{line}"
                );
            }
        }
        "bash" | "zsh" => {
            let mut current_subcommand: Option<&str> = None;
            for line in output.lines() {
                let trimmed = line.trim();
                if trimmed.ends_with("theme__subcmd__use)") || trimmed == "(use)" {
                    current_subcommand = Some("use");
                } else if trimmed.ends_with("theme__subcmd__import)") || trimmed == "(import)" {
                    current_subcommand = Some("import");
                } else if trimmed == ";;" {
                    current_subcommand = None;
                }
                if line.contains("apply-layout") {
                    assert_eq!(
                        current_subcommand,
                        Some("import"),
                        "{shell} completion for apply-layout must be scoped to `theme import`, not {current_subcommand:?}:\n{line}"
                    );
                }
            }
        }
        other => panic!("unexpected shell {other}"),
    }
}

#[test]
fn test_complete_is_hidden_from_help() {
    let output = run_gpy_command(&["--help"]).expect("Failed to run gpy --help");
    assert!(
        !output.contains("__complete"),
        "Help output should not list the hidden __complete command:\n{output}"
    );
}

#[test]
fn test_complete_theme_emits_plain_names() {
    let output =
        run_gpy_command(&["__complete", "theme"]).expect("Failed to run gpy __complete theme");
    assert!(!output.trim().is_empty(), "Expected at least one theme");
    for line in output.lines() {
        assert!(
            !line.contains('*') && !line.contains('['),
            "theme completion line should be undecorated: {line:?}"
        );
    }
}

#[test]
fn test_complete_palette_emits_plain_names() {
    let output =
        run_gpy_command(&["__complete", "palette"]).expect("Failed to run gpy __complete palette");
    assert!(!output.trim().is_empty(), "Expected at least one palette");
    for line in output.lines() {
        assert!(
            !line.contains('*') && !line.contains('['),
            "palette completion line should be undecorated: {line:?}"
        );
    }
}

#[test]
fn test_complete_segment_emits_expected_builtins() {
    let output =
        run_gpy_command(&["__complete", "segment"]).expect("Failed to run gpy __complete segment");
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(
        lines,
        vec![
            "clock",
            "duration",
            "language",
            "directory",
            "git",
            "status"
        ]
    );
}

#[test]
fn test_complete_rejects_invalid_kind() {
    let result = CliTestEnv::new()
        .expect("Failed to create CLI test environment")
        .run_gpy(&["__complete", "bogus"])
        .expect("Failed to invoke gpy __complete bogus");
    assert_ne!(
        result.exit_code, 0_i32,
        "gpy __complete bogus should exit non-zero"
    );
}
