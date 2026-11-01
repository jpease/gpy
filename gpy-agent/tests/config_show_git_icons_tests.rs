//! `gpy config show git` prints every git icon with its effective glyph.
#![allow(clippy::unwrap_used, reason = "test code")]
#![allow(clippy::expect_used, reason = "test code")]

mod common;

use common::cli_harness::CliTestEnv;

fn show_git(config_toml: &str) -> String {
    let env = CliTestEnv::with_config(config_toml).unwrap();
    let result = env.run_gpy(&["config", "show", "git"]).unwrap();
    result.assert_success("gpy config show git");
    result.stdout
}

fn icons_table(stdout: &str) -> toml::Table {
    let doc: toml::Table = stdout.parse().expect("config show output is TOML");
    doc.get("git")
        .and_then(|git| git.get("icons"))
        .and_then(toml::Value::as_table)
        .expect("git.icons table")
        .clone()
}

fn icon<'a>(icons: &'a toml::Table, name: &str) -> Option<&'a str> {
    icons.get(name).and_then(toml::Value::as_str)
}

#[test]
fn config_show_git_prints_effective_icons() {
    let icons = icons_table(&show_git("[git]\nicon_set = \"nerd_font\"\n"));
    assert_eq!(icons.len(), 9, "{icons:?}");
    assert_eq!(icon(&icons, "stash"), Some("\u{f187}"));
    assert_eq!(icon(&icons, "detached"), Some("\u{f126}"));
    assert_eq!(icon(&icons, "in_progress"), Some("\u{f021}"));
    assert_eq!(icon(&icons, "ahead"), Some("↑"));

    let custom = icons_table(&show_git(
        "[git]\nicon_set = \"nerd_font\"\n[git.icons]\nstash = \"≡\"\n",
    ));
    assert_eq!(custom.len(), 9, "{custom:?}");
    assert_eq!(icon(&custom, "stash"), Some("≡"));
}
