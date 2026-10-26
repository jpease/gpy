#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_panics_doc
)]

use super::{DirectoryEnv, DirectoryResolver};
use crate::config::Config;
use crate::formatter::{IsFirst, IsLast, SegmentPosition};
use crate::template::VariableResolver;
use crate::theme::ThemeConfig;

#[test]
fn path_is_present_for_nonempty_cwd() {
    let (config, theme) = (Config::default(), ThemeConfig::default());
    let resolver = DirectoryResolver::new(
        "/home/user/project",
        false,
        DirectoryEnv::unprobed(),
        &config,
        &theme,
        SegmentPosition::new(IsLast::No, IsFirst::No),
    );
    assert!(resolver.resolve("path").is_some());
}

#[test]
fn read_only_none_when_false() {
    let (config, theme) = (Config::default(), ThemeConfig::default());
    let resolver = DirectoryResolver::new(
        "/tmp",
        false,
        DirectoryEnv::unprobed(),
        &config,
        &theme,
        SegmentPosition::new(IsLast::No, IsFirst::No),
    );
    assert_eq!(resolver.resolve("read_only"), None);
}

/// The lock glyph for a read-only `cwd` under `config`/`theme`.
fn read_only_marker(config: &Config, theme: &ThemeConfig) -> Option<String> {
    DirectoryResolver::new(
        "/tmp",
        true,
        DirectoryEnv::unprobed(),
        config,
        theme,
        SegmentPosition::MIDDLE,
    )
    .resolve("read_only")
}

#[test]
fn read_only_some_when_true() {
    let (config, theme) = (Config::default(), ThemeConfig::default());
    assert_eq!(
        read_only_marker(&config, &theme),
        Some("\u{1f512}".to_owned())
    );
}

#[test]
fn read_only_with_plain_glyphs_renders_no_emoji_lock() {
    let mut config = Config::default();
    config.ui.show_icons = false;
    let theme = ThemeConfig::default();
    assert_eq!(read_only_marker(&config, &theme), Some("ro".to_owned()));
}

#[test]
fn read_only_icon_comes_from_the_theme() {
    let mut theme = ThemeConfig::default();
    theme.segments.directory.read_only_icon =
        Some(crate::config::types::Icon::new("\u{f023}").unwrap());
    let config = Config::default();
    assert_eq!(
        read_only_marker(&config, &theme),
        Some("\u{f023}".to_owned())
    );

    let mut plain = Config::default();
    plain.ui.show_icons = false;
    assert_eq!(
        read_only_marker(&plain, &theme),
        Some("ro".to_owned()),
        "plain glyphs drop a Nerd Font lock for the text fallback"
    );
}

#[test]
fn style_contains_bg_color() {
    let (config, theme) = (Config::default(), ThemeConfig::default());
    let resolver = DirectoryResolver::new(
        "/tmp",
        false,
        DirectoryEnv::unprobed(),
        &config,
        &theme,
        SegmentPosition::new(IsLast::No, IsFirst::No),
    );
    let style = resolver.resolve("style").expect("style present");
    assert!(style.contains("bg:"), "got {style}");
}

#[test]
fn unknown_variable_is_none() {
    let (config, theme) = (Config::default(), ThemeConfig::default());
    let resolver = DirectoryResolver::new(
        "/tmp",
        false,
        DirectoryEnv::unprobed(),
        &config,
        &theme,
        SegmentPosition::new(IsLast::No, IsFirst::No),
    );
    assert_eq!(resolver.resolve("nonsuch"), None);
}

/// Caps ≤ `ELLIPSIS_LEN` (3) show the raw tail; larger caps get `"..."` + tail.
#[test]
fn max_length_boundary_ascii_1_through_6() {
    let cases: [(usize, &str); 6] = [
        (1, "h"),
        (2, "gh"),
        (3, "fgh"),
        (4, "...h"),
        (5, "...gh"),
        (6, "...fgh"),
    ];
    for (max, expected) in cases {
        let mut config = Config::default();
        config.ui.directory.max_length =
            crate::config::types::MaxPathLength::new(max).expect("valid max_length");
        let theme = ThemeConfig::default();
        let resolver = DirectoryResolver::new(
            "/abcdefgh",
            false,
            DirectoryEnv::unprobed(),
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        let path = resolver.resolve("path").expect("path present");
        assert_eq!(path, expected, "max_length={max} got {path:?}");
        assert!(
            path.chars().count() <= max,
            "max_length={max} exceeded cap: {path:?}"
        );
    }
}

/// CJK basename: the small-cap policy counts `char`s, not bytes.
#[test]
fn max_length_boundary_unicode_1_through_6() {
    let cases: [(usize, &str); 6] = [
        (1, "九"),
        (2, "八九"),
        (3, "七八九"),
        (4, "...九"),
        (5, "...八九"),
        (6, "...七八九"),
    ];
    for (max, expected) in cases {
        let mut config = Config::default();
        config.ui.directory.max_length =
            crate::config::types::MaxPathLength::new(max).expect("valid max_length");
        let theme = ThemeConfig::default();
        let resolver = DirectoryResolver::new(
            "/一二三四五六七八九",
            false,
            DirectoryEnv::unprobed(),
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        let path = resolver.resolve("path").expect("path present");
        assert_eq!(path, expected, "max_length={max} got {path:?}");
        assert!(
            path.chars().count() <= max,
            "max_length={max} exceeded cap: {path:?}"
        );
    }
}

#[test]
fn unicode_cjk_path_truncated_correctly() {
    let mut config = Config::default();
    // 3 ellipsis + 3 tail
    config.ui.directory.max_length =
        crate::config::types::MaxPathLength::new(6).expect("6 is valid");
    let theme = ThemeConfig::default();
    let cjk = "/一二三四五六七八九";
    let resolver = DirectoryResolver::new(
        cjk,
        false,
        DirectoryEnv::unprobed(),
        &config,
        &theme,
        SegmentPosition::new(IsLast::No, IsFirst::No),
    );
    // Basename "一二三四五六七八九" (9 chars) → "...七八九"
    let path = resolver.resolve("path").expect("path present");
    assert_eq!(path.chars().count(), 6, "got {path:?}");
    assert!(path.starts_with("..."), "got {path:?}");
}

#[test]
fn sep_close_is_half_circle_when_last() {
    let (config, theme) = (Config::default(), ThemeConfig::default());
    let resolver = DirectoryResolver::new(
        "/tmp",
        false,
        DirectoryEnv::unprobed(),
        &config,
        &theme,
        SegmentPosition::new(IsLast::Yes, IsFirst::No),
    );
    assert_eq!(resolver.resolve("sep_close"), Some("\u{e0b4}".to_owned()));
}

#[test]
fn sep_close_is_triangle_when_not_last() {
    let (config, theme) = (Config::default(), ThemeConfig::default());
    let resolver = DirectoryResolver::new(
        "/tmp",
        false,
        DirectoryEnv::unprobed(),
        &config,
        &theme,
        SegmentPosition::new(IsLast::No, IsFirst::No),
    );
    assert_eq!(resolver.resolve("sep_close"), Some("\u{e0bc}".to_owned()));
}

#[test]
fn sep_gap_is_space_when_not_last() {
    let (config, theme) = (Config::default(), ThemeConfig::default());
    let resolver = DirectoryResolver::new(
        "/tmp",
        false,
        DirectoryEnv::unprobed(),
        &config,
        &theme,
        SegmentPosition::new(IsLast::No, IsFirst::No),
    );
    assert_eq!(resolver.resolve("sep_gap"), Some(" ".to_owned()));
}

#[test]
fn sep_gap_is_none_when_last() {
    let (config, theme) = (Config::default(), ThemeConfig::default());
    let resolver = DirectoryResolver::new(
        "/tmp",
        false,
        DirectoryEnv::unprobed(),
        &config,
        &theme,
        SegmentPosition::new(IsLast::Yes, IsFirst::No),
    );
    assert_eq!(resolver.resolve("sep_gap"), None);
}

#[test]
fn bg_is_some_with_directory_bg_color() {
    let (config, theme) = (Config::default(), ThemeConfig::default());
    let resolver = DirectoryResolver::new(
        "/tmp",
        false,
        DirectoryEnv::unprobed(),
        &config,
        &theme,
        SegmentPosition::new(IsLast::No, IsFirst::No),
    );
    assert!(
        resolver.resolve("bg").is_some(),
        "bg should be non-None for directory segment"
    );
}

#[test]
fn sep_open_is_none_when_first() {
    let (config, theme) = (Config::default(), ThemeConfig::default());
    let resolver = DirectoryResolver::new(
        "/tmp",
        false,
        DirectoryEnv::unprobed(),
        &config,
        &theme,
        SegmentPosition::new(IsLast::No, IsFirst::Yes),
    );
    assert_eq!(resolver.resolve("sep_open"), None);
}

#[test]
fn sep_open_is_glyph_when_not_first() {
    let (config, theme) = (Config::default(), ThemeConfig::default());
    let resolver = DirectoryResolver::new(
        "/tmp",
        false,
        DirectoryEnv::unprobed(),
        &config,
        &theme,
        SegmentPosition::new(IsLast::No, IsFirst::No),
    );
    assert_eq!(resolver.resolve("sep_open"), Some("\u{e0ba}".to_owned()));
}

use super::abbreviate_path;
use super::anchor_root;
use super::contract_home;
use super::normalize_cwd;
use super::repo_anchored_path;
use super::truncate_to_components;
use crate::config::types::DirectoryDisplay;
use std::path::Path;

// contract_home: other tests avoid the real $HOME, so these cover contraction.

#[test]
fn contract_home_exact_match_becomes_tilde() {
    assert_eq!(contract_home("/home/user", Some("/home/user")), "~");
}

#[test]
fn contract_home_prefix_on_slash_boundary_contracts() {
    assert_eq!(
        contract_home("/home/user/project", Some("/home/user")),
        "~/project"
    );
}

#[test]
fn contract_home_prefix_match_without_slash_boundary_is_unchanged() {
    // Raw-string prefix but no '/' boundary: must not contract.
    assert_eq!(
        contract_home("/home/user_name/project", Some("/home/user")),
        "/home/user_name/project"
    );
}

#[test]
fn contract_home_no_prefix_match_is_unchanged() {
    assert_eq!(
        contract_home("/var/other", Some("/home/user")),
        "/var/other"
    );
}

#[test]
fn contract_home_none_is_unchanged() {
    assert_eq!(contract_home("/anything", None), "/anything");
}

#[test]
fn contract_home_empty_string_is_unchanged() {
    // Empty HOME means "no home", not "matches everything".
    assert_eq!(contract_home("/anything", Some("")), "/anything");
}

#[test]
fn anchor_full_shows_whole_in_repo_path() {
    let root = Path::new("/home/u/dev/x/myrepo");
    let cwd = Path::new("/home/u/dev/x/myrepo/a/b/c/d");
    assert_eq!(
        repo_anchored_path(cwd, root, DirectoryDisplay::Full, 3, "…/"),
        Some("myrepo/a/b/c/d".to_owned())
    );
}

#[test]
fn anchor_full_at_repo_root_is_basename() {
    let root = Path::new("/home/u/dev/x/myrepo");
    assert_eq!(
        repo_anchored_path(root, root, DirectoryDisplay::Full, 3, "…/"),
        Some("myrepo".to_owned())
    );
}

#[test]
fn anchor_truncated_under_or_equal_keep_has_no_symbol() {
    let root = Path::new("/home/u/dev/x/myrepo");
    // m == N (R, src, sub -> 3) -> unchanged, no symbol
    assert_eq!(
        repo_anchored_path(
            Path::new("/home/u/dev/x/myrepo/src/sub"),
            root,
            DirectoryDisplay::Truncated,
            3,
            "…/"
        ),
        Some("myrepo/src/sub".to_owned())
    );
    assert_eq!(
        repo_anchored_path(root, root, DirectoryDisplay::Truncated, 3, "…/"),
        Some("myrepo".to_owned())
    );
}

#[test]
fn anchor_truncated_deeper_than_keep_pins_repo_and_prepends_symbol() {
    let root = Path::new("/home/u/dev/x/myrepo");
    // m == 5 (R,a,b,c,d) > N=3 -> R + sym + last (N-1)=2 of [a,b,c,d] = c,d
    assert_eq!(
        repo_anchored_path(
            Path::new("/home/u/dev/x/myrepo/a/b/c/d"),
            root,
            DirectoryDisplay::Truncated,
            3,
            "…/"
        ),
        Some("myrepo/…/c/d".to_owned())
    );
    assert_eq!(
        repo_anchored_path(
            Path::new("/home/u/dev/x/myrepo/a/b/c/d"),
            root,
            DirectoryDisplay::Truncated,
            3,
            ""
        ),
        Some("myrepo/c/d".to_owned())
    );
}

#[test]
fn anchor_truncated_keep_one_collapses_to_repo_name() {
    let root = Path::new("/home/u/dev/x/myrepo");
    assert_eq!(
        repo_anchored_path(
            Path::new("/home/u/dev/x/myrepo/a/b/c/d"),
            root,
            DirectoryDisplay::Truncated,
            1,
            "…/"
        ),
        Some("myrepo".to_owned())
    );
}

// abbreviate_path: also asserted end to end by tests/fish/e2e_prompt_content.test.fish.

#[test]
fn abbreviate_root_is_unchanged() {
    assert_eq!(abbreviate_path("/"), "/");
}

#[test]
fn abbreviate_home_only_is_unchanged() {
    assert_eq!(abbreviate_path("~"), "~");
}

#[test]
fn abbreviate_single_component_under_home_is_unchanged() {
    assert_eq!(abbreviate_path("~/project"), "~/project");
}

#[test]
fn abbreviate_home_path_shortens_non_final_components() {
    assert_eq!(abbreviate_path("~/alpha/beta/project"), "~/a/b/project");
}

#[test]
fn abbreviate_absolute_path_shortens_non_final_components() {
    assert_eq!(abbreviate_path("/usr/local/bin"), "/u/l/bin");
}

#[test]
fn abbreviate_two_levels_under_home() {
    assert_eq!(abbreviate_path("~/src/main"), "~/s/main");
}

#[test]
fn abbreviate_keeps_dot_plus_first_char_for_hidden_dirs() {
    assert_eq!(abbreviate_path("~/.config/fish"), "~/.c/fish");
    assert_eq!(abbreviate_path("/.hidden/a/leaf"), "/.h/a/leaf");
    assert_eq!(abbreviate_path("~/./a/leaf"), "~/./a/leaf");
    assert_eq!(abbreviate_path("~/../a/leaf"), "~/../a/leaf");
}

#[test]
fn abbreviate_keeps_whole_leading_grapheme() {
    assert_eq!(abbreviate_path("~/🇺🇸flags/x"), "~/🇺🇸/x");
    assert_eq!(abbreviate_path("~/e\u{301}tc/x"), "~/e\u{301}/x");
    assert_eq!(
        abbreviate_path("~/👨\u{200D}👩\u{200D}👧home/x"),
        "~/👨\u{200D}👩\u{200D}👧/x"
    );
    assert_eq!(abbreviate_path("~/.🇺🇸flags/x"), "~/.🇺🇸/x");
}

#[test]
fn abbreviate_display_mode_renders_via_resolver() {
    let mut config = Config::default();
    config.ui.directory.display = crate::config::types::DirectoryDisplay::Abbreviated;
    let theme = ThemeConfig::default();
    // Absolute path so home-contraction is a no-op; final component preserved.
    let resolver = DirectoryResolver::new(
        "/usr/local/bin/foo",
        false,
        DirectoryEnv::unprobed(),
        &config,
        &theme,
        SegmentPosition::new(IsLast::No, IsFirst::No),
    );
    assert_eq!(
        resolver.resolve("path").expect("path present"),
        "/u/l/b/foo"
    );
}

#[test]
fn trailing_slash_cwd_renders_real_basename() {
    let (config, theme) = (Config::default(), ThemeConfig::default());
    for cwd in ["/usr/local/proj/", "/usr/local/proj/."] {
        let resolver = DirectoryResolver::new(
            cwd,
            false,
            DirectoryEnv::unprobed(),
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("path").as_deref(), Some("proj"), "{cwd}");
    }
    let resolver = DirectoryResolver::new(
        "/",
        false,
        DirectoryEnv::unprobed(),
        &config,
        &theme,
        SegmentPosition::new(IsLast::No, IsFirst::No),
    );
    assert_eq!(resolver.resolve("path").as_deref(), Some("/"));
    assert_eq!(contract_home(&normalize_cwd("/h/u/"), Some("/h/u")), "~");
}

/// Normalization works on the `/`-separated string on every host.
#[test]
fn normalize_cwd_works_on_the_slash_separated_string() {
    assert_eq!(normalize_cwd("/usr//local/./proj/"), "/usr/local/proj");
    assert_eq!(normalize_cwd("/"), "/");
    assert_eq!(normalize_cwd("/."), "/");
    assert_eq!(normalize_cwd("."), ".");
    assert_eq!(normalize_cwd("./a/../b"), "a/../b");
}

#[test]
fn anchor_returns_none_for_basename_and_outside_repo() {
    let root = Path::new("/home/u/dev/x/myrepo");
    assert_eq!(
        repo_anchored_path(root, root, DirectoryDisplay::Basename, 3, "…/"),
        None
    );
    assert_eq!(
        repo_anchored_path(root, root, DirectoryDisplay::Abbreviated, 3, "…/"),
        None
    );
    assert_eq!(
        repo_anchored_path(
            Path::new("/somewhere/else"),
            root,
            DirectoryDisplay::Full,
            3,
            "…/"
        ),
        None
    );
}

#[test]
fn truncate_leaves_home_and_root_markers_alone() {
    assert_eq!(truncate_to_components("~", 3, "…/"), "~");
    assert_eq!(truncate_to_components("/", 3, "…/"), "/");
}

#[test]
fn truncate_keeps_paths_at_or_under_length_unchanged() {
    assert_eq!(truncate_to_components("~/dev", 3, "…/"), "~/dev");
    assert_eq!(truncate_to_components("~/a/b/c", 3, "…/"), "~/a/b/c");
}

#[test]
fn truncate_drops_prefix_and_prepends_symbol_when_deeper() {
    assert_eq!(truncate_to_components("~/a/b/c/d", 3, "…/"), "…/b/c/d");
    assert_eq!(truncate_to_components("~/a/b/c/d", 3, ""), "b/c/d");
    assert_eq!(
        truncate_to_components("/usr/local/bin/foo", 3, "…/"),
        "…/local/bin/foo"
    );
}

#[test]
fn truncate_handles_length_one_boundary() {
    assert_eq!(truncate_to_components("/a/b/c", 1, "…/"), "…/c");
    assert_eq!(truncate_to_components("~/only", 1, "…/"), "~/only");
}

#[test]
fn truncated_display_mode_renders_via_resolver() {
    let mut config = Config::default();
    config.ui.directory.display = crate::config::types::DirectoryDisplay::Truncated;
    config.ui.directory.truncation_length =
        crate::config::types::DirectoryTruncationLength::new(2).expect("2 valid");
    config.ui.directory.truncation_symbol =
        crate::config::types::DirectoryTruncationSymbol::new("…/".to_owned()).expect("valid");
    let theme = ThemeConfig::default();
    // Absolute path not under $HOME so home-contraction is a no-op here.
    let resolver = DirectoryResolver::new(
        "/usr/local/bin/foo",
        false,
        DirectoryEnv::unprobed(),
        &config,
        &theme,
        SegmentPosition::new(IsLast::No, IsFirst::No),
    );
    assert_eq!(resolver.resolve("path").expect("path present"), "…/bin/foo");
}

use std::fs;
use tempfile::tempdir;

fn config_with_truncate_to_repo(display: crate::config::types::DirectoryDisplay) -> Config {
    let mut config = Config::default();
    config.ui.directory.truncate_to_repo = true;
    config.ui.directory.display = display;
    config.ui.directory.truncation_length =
        crate::config::types::DirectoryTruncationLength::new(3).expect("3 valid");
    config.ui.directory.truncation_symbol =
        crate::config::types::DirectoryTruncationSymbol::new("…/".to_owned()).expect("valid");
    config
}

#[test]
fn truncate_to_repo_anchors_truncated_via_resolver() {
    let tmp = tempdir().expect("temp dir");
    let root = fs::canonicalize(tmp.path()).expect("canonicalize root");
    fs::create_dir_all(root.join(".git")).expect("mk .git");
    let deep = root.join("a/b/c/d");
    fs::create_dir_all(&deep).expect("mk deep");
    let root_name = root
        .file_name()
        .and_then(|s| s.to_str())
        .expect("root basename");

    let config = config_with_truncate_to_repo(crate::config::types::DirectoryDisplay::Truncated);
    let theme = ThemeConfig::default();
    let cwd = deep.to_str().expect("utf8 cwd");
    let env = DirectoryEnv::probe(cwd, &config.ui.directory);
    let resolver =
        DirectoryResolver::new(cwd, false, &env, &config, &theme, SegmentPosition::MIDDLE);
    // m=5 (R,a,b,c,d) > N=3 -> R/…/c/d
    assert_eq!(
        resolver.resolve("path").expect("path present"),
        format!("{root_name}/…/c/d")
    );
}

#[test]
fn truncate_to_repo_full_shows_whole_in_repo_path() {
    let tmp = tempdir().expect("temp dir");
    let root = fs::canonicalize(tmp.path()).expect("canonicalize root");
    fs::create_dir_all(root.join(".git")).expect("mk .git");
    let deep = root.join("a/b/c/d");
    fs::create_dir_all(&deep).expect("mk deep");
    let root_name = root
        .file_name()
        .and_then(|s| s.to_str())
        .expect("root basename");

    let config = config_with_truncate_to_repo(crate::config::types::DirectoryDisplay::Full);
    let theme = ThemeConfig::default();
    let cwd = deep.to_str().expect("utf8 cwd");
    let env = DirectoryEnv::probe(cwd, &config.ui.directory);
    let resolver =
        DirectoryResolver::new(cwd, false, &env, &config, &theme, SegmentPosition::MIDDLE);
    assert_eq!(
        resolver.resolve("path").expect("path present"),
        format!("{root_name}/a/b/c/d")
    );
}

#[test]
fn truncate_to_repo_falls_back_when_not_in_repo() {
    let tmp = tempdir().expect("temp dir");
    // No .git anywhere under tmp.
    let dir = fs::canonicalize(tmp.path()).expect("canonicalize");
    let nested = dir.join("alpha/beta");
    fs::create_dir_all(&nested).expect("mk nested");

    let config = config_with_truncate_to_repo(crate::config::types::DirectoryDisplay::Full);
    let theme = ThemeConfig::default();
    let cwd = nested.to_str().expect("utf8 cwd");
    let env = DirectoryEnv::probe(cwd, &config.ui.directory);
    let resolver =
        DirectoryResolver::new(cwd, false, &env, &config, &theme, SegmentPosition::MIDDLE);
    // Full + not in repo -> home-contracted absolute path unchanged.
    assert_eq!(resolver.resolve("path").expect("path present"), cwd);
}

#[test]
fn anchor_root_ignores_repo_rooted_at_home() {
    let tmp = tempdir().expect("temp dir");
    let home = fs::canonicalize(tmp.path()).expect("canonicalize");
    fs::create_dir_all(home.join(".git")).expect("mk .git");
    let proj = home.join("src/proj");
    fs::create_dir_all(proj.join(".git")).expect("mk proj");

    assert_eq!(anchor_root(Some(home.clone()), Some(&home)), None);
    assert_eq!(anchor_root(Some(proj.clone()), Some(&home)), Some(proj));
    assert_eq!(anchor_root(None, Some(&home)), None);
    assert_eq!(anchor_root(Some(home.clone()), None), Some(home));
}

/// `$path` as rendered after a probe with `home` in place of `$HOME` (no env mutation).
fn path_with_home(cwd: &str, config: &Config, theme: &ThemeConfig, home: &str) -> String {
    let env = DirectoryEnv::probe_with_home(cwd, &config.ui.directory, Some(home.to_owned()));
    DirectoryResolver::new(cwd, false, &env, config, theme, SegmentPosition::MIDDLE)
        .resolve("path")
        .expect("path present")
}

#[test]
fn probe_finds_the_repository_only_when_the_path_anchors_there() {
    let tmp = tempdir().expect("temp dir");
    let root = fs::canonicalize(tmp.path()).expect("canonicalize root");
    fs::create_dir_all(root.join(".git")).expect("mk .git");
    let cwd = root.to_str().expect("utf8 cwd");

    let plain = Config::default();
    assert!(!plain.ui.directory.truncate_to_repo);
    assert_eq!(
        DirectoryEnv::probe_with_home(cwd, &plain.ui.directory, None).repo,
        None
    );
    let anchored = config_with_truncate_to_repo(DirectoryDisplay::Full);
    let env = DirectoryEnv::probe_with_home(cwd, &anchored.ui.directory, None);
    assert_eq!(env.repo.map(|repo| repo.root), Some(root));
}

#[cfg(unix)]
#[test]
fn anchor_root_compares_canonical_forms_through_symlink() {
    let tmp = tempdir().expect("temp dir");
    let real = fs::canonicalize(tmp.path()).expect("canonicalize");
    let home = real.join("home");
    fs::create_dir_all(home.join(".git")).expect("mk home");
    let link = real.join("link");
    std::os::unix::fs::symlink(&home, &link).expect("symlink");

    // Repo root reported via a symlink, home canonical.
    assert_eq!(anchor_root(Some(link), Some(&home)), None);
}

// Contraction needs `/` after the home prefix, so a Windows `\\?\C:\` home never contracts.
#[cfg(unix)]
#[test]
fn truncate_to_repo_ignores_repo_rooted_at_home_via_resolver() {
    let tmp = tempdir().expect("temp dir");
    let home = fs::canonicalize(tmp.path()).expect("canonicalize");
    fs::create_dir_all(home.join(".git")).expect("mk .git");
    let cwd_path = home.join("src/x");
    fs::create_dir_all(&cwd_path).expect("mk cwd");
    let cwd = cwd_path.to_str().expect("utf8 cwd");
    let home_str = home.to_str().expect("utf8 home").to_owned();

    let config = config_with_truncate_to_repo(crate::config::types::DirectoryDisplay::Full);
    let theme = ThemeConfig::default();
    assert_eq!(path_with_home(cwd, &config, &theme, &home_str), "~/src/x");
    assert_eq!(path_with_home(&home_str, &config, &theme, &home_str), "~");
}

#[test]
fn truncate_to_repo_still_anchors_repo_below_home_via_resolver() {
    let tmp = tempdir().expect("temp dir");
    let home = fs::canonicalize(tmp.path()).expect("canonicalize");
    fs::create_dir_all(home.join(".git")).expect("mk home .git");
    let proj = home.join("src/proj");
    fs::create_dir_all(proj.join(".git")).expect("mk proj .git");
    let cwd_path = proj.join("lib");
    fs::create_dir_all(&cwd_path).expect("mk cwd");
    let cwd = cwd_path.to_str().expect("utf8 cwd");
    let home_str = home.to_str().expect("utf8 home").to_owned();

    let config = config_with_truncate_to_repo(crate::config::types::DirectoryDisplay::Full);
    let theme = ThemeConfig::default();
    assert_eq!(path_with_home(cwd, &config, &theme, &home_str), "proj/lib");
}
