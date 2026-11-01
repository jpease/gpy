//! Overlay merge rules and chain resolution (ADR-0012 follow-up 1), resolved
//! against a temporary user themes directory and a fixed plugin list.

use super::resolve::{Candidates, OwnName, PluginTheme, link_for_path, own_entry};
use super::{ChainLink, LoadedTheme, MAX_EXTENDS_HOPS, load_with};
use crate::config::defaults::{DEFAULT_THEME_CONTENT, STARSHIP_THEME_CONTENT};
use crate::config::discovery::Source;
use std::path::PathBuf;

/// The smallest full theme.
const FULL: &str = "[ui]\n[segments]\n";

/// A temporary config tree: `themes/` is the user themes directory.
struct Fixture {
    dir: tempfile::TempDir,
    plugins: Vec<PluginTheme>,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("themes")).expect("themes dir");
        Self {
            dir,
            plugins: Vec::new(),
        }
    }

    fn user_dir(&self) -> PathBuf {
        self.dir.path().join("themes")
    }

    /// Write the user theme `name`.
    fn user(&self, name: &str, content: &str) -> PathBuf {
        let path = self.user_dir().join(format!("{name}.toml"));
        std::fs::write(&path, content).expect("write user theme");
        path
    }

    /// Write theme `name` of plugin `plugin_id`, listed after earlier plugins.
    fn plugin(&mut self, plugin_id: &str, name: &str, content: &str) -> PathBuf {
        let root = self.dir.path().join("plugins").join(plugin_id);
        std::fs::create_dir_all(root.join("themes")).expect("plugin themes dir");
        std::fs::write(root.join("plugin.toml"), "").expect("plugin manifest");
        let path = root.join("themes").join(format!("{name}.toml"));
        std::fs::write(&path, content).expect("write plugin theme");
        self.plugins.push(PluginTheme {
            plugin_id: plugin_id.to_owned(),
            stem: name.to_owned(),
            path: path.clone(),
        });
        path
    }

    fn candidates(&self) -> Candidates {
        Candidates::fixed(self.user_dir(), self.plugins.clone())
    }

    /// Load the user theme `name` by path.
    fn load(&self, name: &str) -> crate::Result<LoadedTheme> {
        let path = self.user_dir().join(format!("{name}.toml"));
        load_with(link_for_path(&path), OwnName::Stem, &mut self.candidates())
    }

    /// Load `path`, requested as `requested` (as `ui.theme` names it).
    fn load_requested(&self, requested: &str, path: PathBuf) -> crate::Result<LoadedTheme> {
        let top = ChainLink {
            name: requested.to_owned(),
            source: Source::User,
            on_disk: path.exists(),
            path,
        };
        load_with(top, OwnName::Listed, &mut self.candidates())
    }
}

fn chain_text(loaded: &LoadedTheme) -> String {
    loaded.chain.to_string()
}

fn error_text(result: crate::Result<LoadedTheme>) -> String {
    match result {
        Ok(loaded) => panic!("expected an error, loaded {}", chain_text(&loaded)),
        Err(error) => error.to_string(),
    }
}

#[test]
fn tables_merge_deeply_and_keep_sibling_keys() {
    let fixture = Fixture::new();
    fixture.user(
        "mine",
        "extends = \"default\"\n[segments.clock]\ntime_format = \"24\"\n",
    );
    let base = crate::theme::parse(DEFAULT_THEME_CONTENT, "default").expect("default");

    let loaded = fixture.load("mine").expect("overlay loads");

    let clock = &loaded.theme.segments.clock;
    assert_eq!(clock.time_format.as_deref(), Some("24"));
    assert_eq!(clock.show_seconds, base.segments.clock.show_seconds);
    assert_eq!(clock.bg_color, base.segments.clock.bg_color);
    assert_eq!(clock.format, base.segments.clock.format);
    assert_eq!(
        clock.open.as_ref().map(|open| open.icon.as_str()),
        base.segments
            .clock
            .open
            .as_ref()
            .map(|open| open.icon.as_str())
    );
    assert_eq!(chain_text(&loaded), "mine (user) → default (builtin)");
    assert!(loaded.chain.is_overlay());
    assert!(loaded.fingerprint.is_some());
}

#[test]
fn arrays_replace_and_other_keys_are_inherited() {
    let fixture = Fixture::new();
    fixture.user(
        "mine",
        "extends = \"starship\"\n[ui.recommended]\nenabled_segments = [\"clock\"]\n",
    );
    let base = crate::theme::parse(STARSHIP_THEME_CONTENT, "starship").expect("starship");

    let loaded = fixture.load("mine").expect("overlay loads");

    let recommended = loaded.theme.ui.recommended.expect("recommended inherited");
    let base_recommended = base.ui.recommended.expect("starship recommends");
    assert_eq!(recommended.enabled_segments, Some(vec!["clock".to_owned()]));
    assert_eq!(recommended.palette, base_recommended.palette);
    assert_eq!(recommended.directory, base_recommended.directory);
}

#[test]
fn unset_removes_a_leaf_and_unset_then_set_replaces_a_table() {
    let fixture = Fixture::new();
    fixture.user(
        "mine",
        "extends = \"starship\"\n\
         unset = [\"segments.character.format\", \"segments.language.symbols\", \"segments.nope.gone\"]\n\
         [segments.language.symbols]\nrust_symbol = \"R\"\n",
    );

    let loaded = fixture.load("mine").expect("overlay loads");

    assert_eq!(loaded.theme.segments.character.format, None);
    let language = &loaded.theme.segments.language;
    assert_eq!(language.get_symbol("rust"), Some("R"));
    assert_eq!(
        language.get_symbol("python"),
        None,
        "the base's flat symbols were unset with the nested table"
    );
}

#[test]
fn flat_and_nested_language_keys_meet_on_one_path() {
    let fixture = Fixture::new();
    fixture.user(
        "mine",
        "extends = \"starship\"\nunset = [\"segments.language.go_symbol\"]\n\
         [segments.language]\npython_symbol = \"P\"\n\
         [segments.language.symbols]\nrust_symbol = \"R\"\n",
    );

    let loaded = fixture.load("mine").expect("overlay loads");

    let language = &loaded.theme.segments.language;
    assert_eq!(language.get_symbol("rust"), Some("R"), "nested over flat");
    assert_eq!(language.get_symbol("python"), Some("P"), "flat over flat");
    assert_eq!(language.get_symbol("go"), None, "a flat unset path");
    assert_eq!(language.get_symbol("java"), Some("☕"), "inherited");
    assert_eq!(language.get_style("java"), "dimmed", "inherited style");
}

#[test]
fn self_named_overlay_extends_the_builtin_below_it() {
    let fixture = Fixture::new();
    fixture.user(
        "starship",
        "extends = \"starship\"\n[segments.directory]\nformat = \"[$path](bold red) \"\n",
    );
    let base = crate::theme::parse(STARSHIP_THEME_CONTENT, "starship").expect("starship");

    let loaded = fixture.load("starship").expect("self-named overlay loads");

    assert_eq!(chain_text(&loaded), "starship (user) → starship (builtin)");
    assert_eq!(
        loaded.theme.segments.directory.format.as_deref(),
        Some("[$path](bold red) ")
    );
    assert_eq!(loaded.theme.segments.git.format, base.segments.git.format);
}

#[test]
fn self_named_overlay_prefers_a_plugin_theme_to_the_builtin() {
    let mut fixture = Fixture::new();
    fixture.plugin(
        "acme",
        "starship",
        "[ui]\nprompt_icon = \"A\"\n[segments]\n",
    );
    fixture.user("starship", "extends = \"starship\"\n");

    let loaded = fixture.load("starship").expect("overlay loads");

    assert_eq!(
        chain_text(&loaded),
        "starship (user) → starship (plugin:acme)"
    );
    assert_eq!(loaded.theme.ui.prompt_icon.as_deref(), Some("A"));
}

#[test]
fn self_named_overlay_with_nothing_below_is_an_error() {
    let fixture = Fixture::new();
    fixture.user("mine", "extends = \"mine\"\n");

    let message = error_text(fixture.load("mine"));

    assert!(
        message.contains(
            "extends 'mine' names this theme and no plugin or builtin theme 'mine' exists"
        ),
        "{message}"
    );
}

#[test]
fn user_to_plugin_to_builtin_chain_skips_user_files_below_the_plugin() {
    let mut fixture = Fixture::new();
    fixture.user("starship", FULL);
    fixture.plugin(
        "acme",
        "acme",
        "extends = \"starship\"\n[segments.clock]\ntime_format = \"24\"\n",
    );
    fixture.user("mine", "extends = \"acme\"\n[ui]\nprompt_icon = \"M\"\n");

    let loaded = fixture.load("mine").expect("chain loads");

    assert_eq!(
        chain_text(&loaded),
        "mine (user) → acme (plugin:acme) → starship (builtin)"
    );
    assert_eq!(loaded.theme.ui.prompt_icon.as_deref(), Some("M"));
    assert_eq!(
        loaded.theme.segments.clock.time_format.as_deref(),
        Some("24")
    );
    assert!(
        loaded.theme.segments.git.format.is_some(),
        "the builtin starship, not the user's empty copy, is the base"
    );
}

#[test]
fn lexically_greatest_plugin_wins_a_name() {
    let mut fixture = Fixture::new();
    fixture.plugin("alpha", "shared", "[ui]\nprompt_icon = \"a\"\n[segments]\n");
    fixture.plugin("beta", "shared", "[ui]\nprompt_icon = \"b\"\n[segments]\n");
    fixture.user("mine", "extends = \"shared\"\n");

    let loaded = fixture.load("mine").expect("chain loads");

    assert_eq!(loaded.theme.ui.prompt_icon.as_deref(), Some("b"));
}

#[test]
fn plugin_overlay_may_extend_only_a_builtin() {
    let mut fixture = Fixture::new();
    fixture.user("mine", FULL);
    let path = fixture.plugin("acme", "acme", "extends = \"mine\"\n");

    let message = error_text(load_with(
        ChainLink {
            name: "acme".to_owned(),
            source: Source::Plugin {
                plugin_id: "acme".to_owned(),
            },
            path,
            on_disk: true,
        },
        OwnName::Listed,
        &mut fixture.candidates(),
    ));

    assert!(
        message.contains("extends 'mine', which is not a builtin theme"),
        "{message}"
    );
}

#[test]
fn a_plugin_theme_loaded_by_path_gets_the_plugin_rule() {
    let mut fixture = Fixture::new();
    fixture.user("mine", FULL);
    let path = fixture.plugin("acme", "acme", "extends = \"mine\"\n");

    let link = link_for_path(&path);

    assert!(
        matches!(link.source, Source::Plugin { .. }),
        "{:?}",
        link.source
    );
}

#[test]
fn case_variant_extends_does_not_resolve() {
    let fixture = Fixture::new();
    fixture.user("mine", "extends = \"Starship\"\n");

    let message = error_text(fixture.load("mine"));

    assert!(
        message.contains("extends 'Starship', which matches no user, plugin or builtin theme"),
        "{message}"
    );
}

#[test]
fn case_variant_request_takes_the_file_stem_as_its_own_name() {
    let fixture = Fixture::new();
    // A case-insensitive filesystem opens `Starship.toml` as this file.
    let path = fixture.user("starship", "extends = \"starship\"\n");

    let loaded = fixture
        .load_requested("Starship", path)
        .expect("resolves below itself, not as a cycle");

    assert_eq!(chain_text(&loaded), "starship (user) → starship (builtin)");
}

#[test]
fn overlay_whose_request_matches_no_entry_is_an_error() {
    let fixture = Fixture::new();
    let overlay = fixture.user("mine", "extends = \"starship\"\n");
    let full = fixture.user("full", FULL);

    let message = error_text(fixture.load_requested("other", overlay.clone()));
    assert!(
        message.contains(&format!(
            "theme 'other' opened {}, whose file name differs; set ui.theme to the file's name",
            overlay.display()
        )),
        "{message}"
    );
    assert!(
        fixture.load_requested("other", full).is_ok(),
        "a full theme needs no own name"
    );
}

/// APFS and HFS+ open an NFC name as its NFD file; neither the exact nor the
/// case-insensitive rule matches it, and GPY adds no normalisation.
#[test]
fn nfc_request_for_an_nfd_file_is_an_overlay_error() {
    let nfd = "cafe\u{301}";
    let nfc = "caf\u{e9}";
    let fixture = Fixture::new();
    let overlay = fixture.user(nfd, "extends = \"default\"\n");

    let message = error_text(fixture.load_requested(nfc, overlay.clone()));
    assert!(message.contains("whose file name differs"), "{message}");
    assert!(
        own_entry(vec![(nfd.to_owned(), overlay.clone())], nfc).is_none(),
        "no normalisation"
    );
    assert_eq!(
        own_entry(vec![(nfd.to_owned(), overlay.clone())], nfd),
        Some((nfd.to_owned(), overlay)),
        "the same spelling matches"
    );

    std::fs::write(fixture.user_dir().join(format!("{nfd}.toml")), FULL).expect("rewrite");
    let full = fixture.user_dir().join(format!("{nfd}.toml"));
    assert!(
        fixture.load_requested(nfc, full).is_ok(),
        "a full theme loads"
    );
}

#[test]
fn own_name_prefers_the_exact_stem_and_rejects_ambiguous_case_matches() {
    let entries = |names: &[&str]| -> Vec<(String, PathBuf)> {
        names
            .iter()
            .map(|name| ((*name).to_owned(), PathBuf::from(format!("{name}.toml"))))
            .collect()
    };
    assert_eq!(
        own_entry(entries(&["Mine", "mine"]), "mine").map(|(stem, _)| stem),
        Some("mine".to_owned())
    );
    assert_eq!(
        own_entry(entries(&["Mine"]), "mine").map(|(stem, _)| stem),
        Some("Mine".to_owned())
    );
    assert_eq!(own_entry(entries(&["Mine", "MINE"]), "mine"), None);
}

#[test]
fn chains_follow_at_most_four_hops() {
    let fixture = Fixture::new();
    fixture.user("a", "extends = \"b\"\n");
    fixture.user("b", "extends = \"c\"\n");
    fixture.user("c", "extends = \"d\"\n");
    fixture.user("d", "extends = \"e\"\n");
    fixture.user("e", "extends = \"default\"\n");

    let four = fixture.load("b").expect("four hops load");
    assert_eq!(
        four.chain.iter().count(),
        MAX_EXTENDS_HOPS.saturating_add(1)
    );

    let message = error_text(fixture.load("a"));
    assert!(
        message.contains("needs more than 4 `extends` hops"),
        "{message}"
    );
}

#[test]
fn cycles_are_errors_naming_the_chain() {
    let fixture = Fixture::new();
    fixture.user("a", "extends = \"b\"\n");
    fixture.user("b", "extends = \"a\"\n");

    let message = error_text(fixture.load("a"));

    assert!(
        message.contains("`extends` cycle: a (user) → b (user) → a (user)"),
        "{message}"
    );
}

#[test]
fn extends_is_a_theme_name_never_a_path() {
    let fixture = Fixture::new();
    fixture.user("path", "extends = \"../default\"\n");
    fixture.user("number", "extends = 3\n");

    let path = error_text(fixture.load("path"));
    assert!(path.contains("invalid theme name '../default'"), "{path}");
    let number = error_text(fixture.load("number"));
    assert!(
        number.contains("`extends` must be a theme name string, not integer"),
        "{number}"
    );
}

#[test]
fn unset_entries_must_be_dotted_key_paths() {
    let fixture = Fixture::new();
    fixture.user(
        "empty",
        "extends = \"default\"\nunset = [\"segments..x\"]\n",
    );
    fixture.user(
        "spaced",
        "extends = \"default\"\nunset = [\"ok\", \"a b\"]\n",
    );
    fixture.user("scalar", "extends = \"default\"\nunset = \"ui\"\n");

    let empty = error_text(fixture.load("empty"));
    assert!(
        empty.contains("`unset` entry 1 (\"segments..x\")"),
        "{empty}"
    );
    let spaced = error_text(fixture.load("spaced"));
    assert!(spaced.contains("`unset` entry 2 (\"a b\")"), "{spaced}");
    let scalar = error_text(fixture.load("scalar"));
    assert!(
        scalar.contains("`unset` must be an array of dotted key paths, not string"),
        "{scalar}"
    );
}

#[test]
fn a_full_theme_is_a_chain_of_one_and_ignores_unset() {
    let fixture = Fixture::new();
    fixture.user(
        "full",
        "unset = [\"ui\"]\n[ui]\nprompt_icon = \"F\"\n[segments]\n",
    );

    let loaded = fixture.load("full").expect("full theme loads");

    assert!(!loaded.chain.is_overlay());
    assert_eq!(chain_text(&loaded), "full (user)");
    assert_eq!(loaded.theme.ui.prompt_icon.as_deref(), Some("F"));
    assert!(loaded.fingerprint.is_some());
}

#[test]
fn a_missing_file_with_a_builtin_stem_is_the_embedded_builtin() {
    let fixture = Fixture::new();

    let loaded = fixture.load("default").expect("embedded fallback");

    assert_eq!(chain_text(&loaded), "default (builtin)");
    assert_eq!(loaded.fingerprint, None);
}
