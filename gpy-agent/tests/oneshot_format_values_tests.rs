//! `--format` values offered by clap must be exactly the four rendered formats, and the oneshot help must list exactly those.

#![allow(clippy::missing_panics_doc, clippy::unwrap_used, clippy::expect_used)]

use clap::ValueEnum;
use gpy_agent::formatter::Format;
use std::process::Command;

/// Every `Format` variant. The `match` makes this fail to compile when a
/// variant is added without being listed, so the two-way check below can
/// never silently skip one.
fn every_format() -> Vec<Format> {
    let all = vec![
        Format::Json,
        Format::Ansi,
        Format::BashPrompt,
        Format::ZshPrompt,
    ];
    for format in &all {
        match format {
            Format::Json | Format::Ansi | Format::BashPrompt | Format::ZshPrompt => {}
        }
    }
    all
}

#[test]
fn value_variants_are_all_renderable() {
    assert_eq!(Format::value_variants().to_vec(), every_format());
    for f in every_format() {
        assert_eq!(f.as_str().parse::<Format>().unwrap(), f);
    }
}

/// Each oneshot subcommand's `--help` lists only working formats and never
/// the rejected `fish-ansi`.
#[test]
fn oneshot_help_lists_only_working_formats() {
    let home = tempfile::tempdir().unwrap();
    for sub in [
        "git",
        "lang",
        "directory",
        "duration",
        "character",
        "hostname",
        "username",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_gpy-agent"))
            .args(["oneshot", sub, "--help"])
            .env("HOME", home.path())
            .env("XDG_CONFIG_HOME", home.path().join("config"))
            .env("XDG_CACHE_HOME", home.path().join("cache"))
            .env_remove("XDG_RUNTIME_DIR")
            .output()
            .expect("run gpy-agent oneshot --help");
        assert!(output.status.success(), "oneshot {sub} --help failed");
        let help = String::from_utf8_lossy(&output.stdout);
        assert!(
            !help.contains("fish-ansi"),
            "oneshot {sub} --help advertises the rejected fish-ansi:\n{help}"
        );
        let possible = help
            .lines()
            .find(|line| line.contains("[possible values:"))
            .unwrap_or_else(|| panic!("oneshot {sub} --help lists no possible values:\n{help}"));
        for format in every_format() {
            let listed = possible
                .split([',', ' ', ']', ':'])
                .any(|value| value == format.as_str());
            assert!(listed, "oneshot {sub} --help omits {}", format.as_str());
        }
        for removed in ["fish", "fish-source", "zsh", "bash-source", "zsh-source"] {
            assert!(
                !possible
                    .split([',', ' ', ']', ':'])
                    .any(|value| value == removed),
                "oneshot {sub} --help still lists removed format {removed}"
            );
        }
    }
}
