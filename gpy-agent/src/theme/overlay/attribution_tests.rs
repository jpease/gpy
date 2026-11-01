//! Error attribution through an overlay merge (ADR-0012): each
//! failure names the file and line that caused it, and the chain.

use super::attribute::{rebuild, table_error};
use super::resolve::{Candidates, OwnName, link_for_path};
use super::{ChainLink, LoadedTheme, ThemeChain, load_with};
use crate::config::discovery::Source;
use crate::config::validation::templates::validate_segment_templates;
use crate::template::Palette;
use crate::theme::from_table;
use std::borrow::Cow;
use std::path::PathBuf;

/// A temporary user themes directory.
struct Themes {
    dir: tempfile::TempDir,
}

impl Themes {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("tempdir"),
        }
    }

    fn write(&self, name: &str, content: &str) -> PathBuf {
        let path = self.dir.path().join(format!("{name}.toml"));
        std::fs::write(&path, content).expect("write theme");
        path
    }

    fn load(&self, name: &str) -> crate::Result<LoadedTheme> {
        let path = self.dir.path().join(format!("{name}.toml"));
        load_with(
            link_for_path(&path),
            OwnName::Stem,
            &mut Candidates::fixed(self.dir.path().to_path_buf(), Vec::new()),
        )
    }

    /// The load error of overlay `content`, saved as `mine.toml`, and its path.
    fn error(&self, content: &str) -> (String, String) {
        let path = self.write("mine", content);
        let message = match self.load("mine") {
            Ok(_) => panic!("overlay must fail:\n{content}"),
            Err(error) => error.to_string(),
        };
        (message, path.display().to_string())
    }
}

fn assert_contains(message: &str, needle: &str) {
    assert!(
        message.contains(needle),
        "missing {needle:?} in:\n{message}"
    );
}

fn assert_user_fault(message: &str) {
    assert!(!message.contains("GPY bug"), "{message}");
    assert!(!message.contains("set at builtin"), "{message}");
}

#[test]
fn plugin_segment_key_names_the_overlay_line() {
    let themes = Themes::new();
    let (message, path) =
        themes.error("extends = \"starship\"\n\n[segments.username]\nformat = \"$user\"\n");

    assert_contains(&message, "Invalid theme 'mine'");
    assert_contains(
        &message,
        "segments.username.format: unknown plugin segment style key",
    );
    assert_contains(&message, &format!("set at {path}:4:1"));
    assert_contains(&message, "(chain: mine (user) → starship (builtin))");
    assert_user_fault(&message);
}

#[test]
fn typed_key_names_the_overlay_line() {
    let themes = Themes::new();
    let (message, path) =
        themes.error("extends = \"starship\"\n\n[segments.clock]\nbg_color = \"not-a-colour\"\n");

    assert_contains(&message, "segments.clock.bg_color: ");
    assert_contains(&message, &format!("set at {path}:4:1"));
    assert_user_fault(&message);
}

#[test]
fn table_valued_key_names_the_table_and_the_overlay_line() {
    let themes = Themes::new();
    let (message, path) =
        themes.error("extends = \"starship\"\n\n[ui.recommended]\nenabled_segments = 5\n");

    assert_contains(&message, "Diagnostic: ui.recommended: ");
    assert_contains(&message, &format!("set at {path}:4:1"));
    assert_user_fault(&message);
}

#[test]
fn non_table_plugin_entry_names_the_overlay_line() {
    let themes = Themes::new();
    let (message, path) = themes.error("extends = \"starship\"\n\n[segments]\nfoo = 1\n");

    assert_contains(&message, "Diagnostic: segments.foo: ");
    assert_contains(&message, &format!("set at {path}:4:1"));
    assert_user_fault(&message);
}

#[test]
fn flat_language_key_is_named_in_the_overlay_spelling() {
    let themes = Themes::new();
    let (message, path) = themes
        .error("extends = \"starship\"\n\n[segments.language]\nrust_bg_color = \"not-a-colour\"\n");

    assert_contains(&message, "Diagnostic: segments.language.rust_bg_color: ");
    assert_contains(&message, &format!("set at {path}:4:1"));
    assert_user_fault(&message);
}

#[test]
fn bad_format_caught_by_template_validation_names_the_overlay_line() {
    let themes = Themes::new();
    let path = themes.write(
        "mine",
        "extends = \"starship\"\n\n[segments.git]\nformat = \"[$branch](fg:nonsuch)\"\n",
    );
    let loaded = themes.load("mine").expect("the overlay itself loads");

    let error =
        validate_segment_templates(&loaded.theme, &Palette::default()).expect_err("unknown colour");
    let message = loaded.template_error(error).to_string();

    assert_contains(&message, "segments.git.format: segment 'git'");
    assert_contains(&message, &format!("set at {}:4:1", path.display()));
    assert_user_fault(&message);
}

#[test]
fn empty_delimiter_table_over_default_names_the_table() {
    let themes = Themes::new();
    let (message, path) = themes.error("extends = \"default\"\n\n[segments.username.open]\n");

    assert_contains(&message, "Diagnostic: segments.username.open: ");
    assert_contains(&message, &format!("set at {path}:3:"));
    assert_user_fault(&message);
}

#[test]
fn unset_delimiter_icon_names_the_unset_element() {
    let themes = Themes::new();
    let (message, path) =
        themes.error("extends = \"default\"\nunset = [\"segments.clock.open.icon\"]\n");

    assert_contains(&message, "Diagnostic: segments.clock.open: ");
    assert_contains(&message, &format!("unset at {path}:2:10"));
    assert_contains(&message, "Remove the `unset` entry");
    assert_user_fault(&message);
}

#[test]
fn unset_ui_names_the_unset_element() {
    let themes = Themes::new();
    let (message, path) = themes.error("extends = \"default\"\nunset = [\"ui\"]\n");

    assert_contains(&message, "Diagnostic: ui: missing required [ui] table");
    assert_contains(&message, &format!("unset at {path}:2:10"));
    assert_user_fault(&message);
}

/// `Username` sorts before the builtin's `username`, so validation names the
/// builtin's key; attribution maps both and names the overlay.
#[test]
fn duplicate_section_ignoring_case_names_the_overlay_section() {
    let themes = Themes::new();
    let (message, path) =
        themes.error("extends = \"default\"\n\n[segments.Username]\nbg_color = \"red\"\n");

    assert_contains(&message, "segments.Username, segments.username: ");
    assert_contains(&message, &format!("set at {path}:3:"));
    assert_user_fault(&message);
}

#[test]
fn token_collision_names_both_sections() {
    let themes = Themes::new();
    let base = themes.write(
        "base",
        "[ui]\n[segments]\n\n[segments.foo_bar]\nbg_color = \"red\"\n",
    );
    let (message, path) =
        themes.error("extends = \"base\"\n\n[segments.Foo-bar]\nbg_color = \"blue\"\n");

    assert_contains(&message, "segments.Foo-bar, segments.foo_bar: ");
    assert_contains(&message, &format!("set at {path}:3:"));
    assert_contains(&message, &format!("set at {}:4:", base.display()));
    assert_contains(&message, "(chain: mine (user) → base (user))");
    assert_user_fault(&message);
}

/// A two-link chain over an in-memory "builtin" `x` with `builtin_text`.
fn builtin_chain_error(overlay_text: &str, builtin_text: &str) -> String {
    let chain = ThemeChain {
        top: ChainLink {
            name: "mine".to_owned(),
            source: Source::User,
            path: PathBuf::from("/themes/mine.toml"),
            on_disk: true,
        },
        bases: vec![ChainLink {
            name: "x".to_owned(),
            source: Source::Builtin,
            path: PathBuf::from("/themes/x.toml"),
            on_disk: false,
        }],
    };
    let texts = vec![
        Cow::Owned(overlay_text.to_owned()),
        Cow::Owned(builtin_text.to_owned()),
    ];
    let (merged, _) = rebuild(&texts).expect("texts parse");
    let error = from_table(merged, "mine").expect_err("merged theme fails");
    table_error(&chain, &texts, error).to_string()
}

#[test]
fn failure_only_a_builtin_writes_is_a_gpy_bug() {
    let message = builtin_chain_error(
        "extends = \"x\"\n[segments.clock]\ntime_format = \"24\"\n",
        "[ui]\n[segments.clock.open]\nicon_color = \"red\"\n",
    );

    assert_contains(&message, "Diagnostic: segments.clock.open: ");
    assert_contains(&message, "set at builtin theme 'x' (line 2, column");
    assert_contains(&message, "This is a GPY bug");
}

#[test]
fn failure_an_overlay_also_writes_is_not_a_gpy_bug() {
    let message = builtin_chain_error(
        "extends = \"x\"\n[segments.clock.open]\nbg_color = \"blue\"\n",
        "[ui]\n[segments.clock.open]\nicon_color = \"red\"\n",
    );

    assert_contains(&message, "set at /themes/mine.toml:3:1");
    assert_user_fault(&message);
}
