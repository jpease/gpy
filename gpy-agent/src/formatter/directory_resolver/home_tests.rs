#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_panics_doc,
    reason = "test code may panic on failure"
)]

use super::{DirectoryEnv, DirectoryResolver};
use crate::config::Config;
use crate::config::types::{
    DirectoryDisplay, DirectoryTruncationLength, DirectoryTruncationSymbol,
};
use crate::formatter::SegmentPosition;
use crate::template::VariableResolver;
use crate::theme::ThemeConfig;
use std::fs;
use tempfile::tempdir;

fn full_config() -> Config {
    let mut config = Config::default();
    config.ui.directory.display = DirectoryDisplay::Full;
    config
}

/// `$path` as rendered after a probe with `home` in place of `$HOME` (no env mutation).
fn path_with_home(cwd: &str, config: &Config, theme: &ThemeConfig, home: &str) -> String {
    let env = DirectoryEnv::probe_with_home(cwd, &config.ui.directory, Some(home.to_owned()));
    DirectoryResolver::new(cwd, false, &env, config, theme, SegmentPosition::MIDDLE)
        .resolve("path")
        .expect("path present")
}

#[test]
fn trailing_slash_home_contracts_cwd_under_home() {
    let (config, theme) = (full_config(), ThemeConfig::default());
    assert_eq!(
        path_with_home("/home/u/proj", &config, &theme, "/home/u/"),
        "~/proj"
    );
}

#[test]
fn trailing_slash_home_contracts_cwd_equal_to_home() {
    let (config, theme) = (full_config(), ThemeConfig::default());
    for home in ["/home/u/", "/home/u//"] {
        assert_eq!(path_with_home("/home/u", &config, &theme, home), "~");
    }
}

#[test]
fn trailing_slash_home_does_not_contract_sibling() {
    let (config, theme) = (full_config(), ThemeConfig::default());
    assert_eq!(
        path_with_home("/home/user_name/proj", &config, &theme, "/home/user/"),
        "/home/user_name/proj"
    );
}

#[test]
fn trailing_slash_home_is_still_not_an_anchor_root() {
    let tmp = tempdir().expect("temp dir");
    let root = fs::canonicalize(tmp.path()).expect("canonicalize root");
    fs::create_dir_all(root.join(".git")).expect("mk .git");
    let mut config = full_config();
    config.ui.directory.truncate_to_repo = true;
    config.ui.directory.truncation_length = DirectoryTruncationLength::new(3).expect("3 valid");
    config.ui.directory.truncation_symbol =
        DirectoryTruncationSymbol::new("…/".to_owned()).expect("valid");
    let cwd = root.to_str().expect("utf8 cwd");

    let env = DirectoryEnv::probe_with_home(
        cwd,
        &config.ui.directory,
        Some(format!("{}/", root.display())),
    );

    assert_eq!(env.repo, None);
}
