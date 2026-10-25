#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::missing_panics_doc,
    clippy::shadow_unrelated,
    reason = "tests: panics are the assertions, and each `err` binds a fresh case"
)]

use super::*;
use crate::config::loader;
use crate::plugin::{DiscoveredPlugin, PluginManifest, PluginSource};

const MANIFEST: &str = r#"
command = "k8s.sh"
timeout_ms = 300
[settings.style]
type = "string"
default = "bold"
choices = ["bold", "dim"]
[settings.depth]
type = "int"
default = 2
min = 0
max = 5
[settings.short]
type = "bool"
default = false
"#;

fn segment() -> SegmentName {
    SegmentName::new("k8s").unwrap()
}

fn discovery() -> PluginDiscovery {
    let manifest = PluginManifest::parse(
        r#"
        id = "k8s-tools"
        name = "K8s"
        version = "1.0.0"
        api_version = "v1"
        provided_segments = ["k8s"]
        entry_type = "script"
        "#,
    )
    .unwrap();
    PluginDiscovery {
        plugins: vec![DiscoveredPlugin {
            manifest,
            root: std::path::PathBuf::from("/plugins/k8s-tools"),
            source: PluginSource::User,
            script_segments: BTreeMap::from([(
                segment(),
                ScriptSegmentManifest::parse(MANIFEST).unwrap(),
            )]),
        }],
        diagnostics: Vec::new(),
    }
}

fn manifest() -> ScriptSegmentManifest {
    ScriptSegmentManifest::parse(MANIFEST).unwrap()
}

#[test]
fn config_table_is_typed_at_load() {
    let config = loader::parse_config_toml(
        "[plugin.K8S]\nttl_secs = 10\ntimeout_ms = 100\nstyle = \"dim\"\ndepth = 3\nshort = true\n",
    )
    .unwrap();
    let table = config
        .plugin
        .get(&segment())
        .expect("normalized segment key");
    assert_eq!(table.ttl_secs.map(TtlSecs::get), Some(10));
    assert_eq!(table.timeout_ms.map(TimeoutMs::get), Some(100));
    assert_eq!(
        table.settings.get(&SettingKey::new("depth").unwrap()),
        Some(&SettingValue::Int(3))
    );
}

#[test]
fn config_table_rejects_bad_shapes_by_name() {
    for (body, needle) in [
        ("timeout_ms = 501", "timeout_ms"),
        ("timeout_ms = 0", "timeout_ms"),
        ("ttl_secs = -1", "ttl_secs"),
        ("Bad-Key = 1", "Bad-Key"),
        ("nested = { a = 1 }", "nested"),
    ] {
        let err = loader::parse_config_toml(&format!("[plugin.k8s]\n{body}\n"))
            .unwrap_err()
            .to_string();
        assert!(err.contains(needle), "{needle:?} not in {err}");
    }
    let err = loader::parse_config_toml("[plugin.\"bad seg\"]\nttl_secs = 1\n")
        .unwrap_err()
        .to_string();
    assert!(err.contains("bad seg"), "{err}");
}

#[test]
fn resolve_applies_defaults_then_overrides() {
    let resolved = resolve(&segment(), &manifest(), None).unwrap();
    assert_eq!(resolved.ttl_secs.get(), 5);
    assert_eq!(resolved.timeout_ms.get(), 300);
    assert_eq!(resolved.settings.len(), 3);

    let overrides = PluginSegmentConfig {
        ttl_secs: Some(TtlSecs::new(0).unwrap()),
        timeout_ms: Some(TimeoutMs::new(100).unwrap()),
        settings: BTreeMap::from([(
            SettingKey::new("style").unwrap(),
            SettingValue::String("dim".to_owned()),
        )]),
    };
    let resolved = resolve(&segment(), &manifest(), Some(&overrides)).unwrap();
    assert_eq!(resolved.ttl_secs.get(), 0);
    assert_eq!(resolved.timeout_ms.get(), 100);
    assert_eq!(
        resolved.settings.get(&SettingKey::new("style").unwrap()),
        Some(&SettingValue::String("dim".to_owned()))
    );
}

#[test]
fn resolve_errors_name_the_dotted_key() {
    let with = |key: &str, value: SettingValue| PluginSegmentConfig {
        settings: BTreeMap::from([(SettingKey::new(key).unwrap(), value)]),
        ..PluginSegmentConfig::default()
    };
    for (overrides, expected) in [
        (
            with("nope", SettingValue::Int(1)),
            "plugin.k8s.nope: not a declared setting",
        ),
        (
            with("depth", SettingValue::Int(9)),
            "plugin.k8s.depth: 9 is above max 5",
        ),
        (
            with("short", SettingValue::String("x".to_owned())),
            "plugin.k8s.short: expected bool",
        ),
        (
            PluginSegmentConfig {
                timeout_ms: Some(TimeoutMs::new(400).unwrap()),
                ..PluginSegmentConfig::default()
            },
            "plugin.k8s.timeout_ms: 400 is above the segment's timeout_ms 300",
        ),
    ] {
        let err = resolve(&segment(), &manifest(), Some(&overrides))
            .unwrap_err()
            .to_string();
        assert!(err.contains(expected), "{expected:?} not in {err}");
    }
}

#[test]
fn set_and_get_reach_plugin_keys() {
    let discovery = discovery();
    let mut config = Config::default();
    assert_eq!(
        get(&config, &discovery, "plugin.k8s.ttl_secs").unwrap(),
        "5"
    );
    assert_eq!(
        get(&config, &discovery, "plugin.k8s.style").unwrap(),
        "bold"
    );

    set(&mut config, &discovery, "plugin.K8s.ttl_secs", "30").unwrap();
    set(&mut config, &discovery, "plugin.k8s.timeout_ms", "250").unwrap();
    set(&mut config, &discovery, "plugin.k8s.style", "dim").unwrap();
    set(&mut config, &discovery, "plugin.k8s.short", "yes").unwrap();
    assert_eq!(
        get(&config, &discovery, "plugin.k8s.ttl_secs").unwrap(),
        "30"
    );
    assert_eq!(
        get(&config, &discovery, "plugin.k8s.timeout_ms").unwrap(),
        "250"
    );
    assert_eq!(get(&config, &discovery, "plugin.k8s.style").unwrap(), "dim");
    assert_eq!(
        get(&config, &discovery, "plugin.k8s.short").unwrap(),
        "true"
    );
    assert_eq!(
        canonical_key("plugin.K8s.ttl_secs").unwrap(),
        "plugin.k8s.ttl_secs"
    );
}

#[test]
fn set_rejects_invalid_values_naming_the_key() {
    let discovery = discovery();
    let mut config = Config::default();
    for (key, value, expected) in [
        (
            "plugin.k8s.style",
            "loud",
            "plugin.k8s.style: 'loud' is not one of",
        ),
        (
            "plugin.k8s.depth",
            "six",
            "plugin.k8s.depth: 'six' is not an integer",
        ),
        ("plugin.k8s.timeout_ms", "400", "may only lower it"),
        (
            "plugin.k8s.timeout_ms",
            "0",
            "plugin.k8s.timeout_ms: timeout_ms must be 1..=500",
        ),
        (
            "plugin.k8s.ttl_secs",
            "soon",
            "plugin.k8s.ttl_secs: 'soon' is not a whole number",
        ),
        (
            "plugin.k8s.unknown",
            "1",
            "plugin.k8s.unknown: not a declared setting",
        ),
        (
            "plugin.other.ttl_secs",
            "1",
            "no installed script plugin provides segment 'other'",
        ),
        ("plugin.k8s", "1", "plugin keys have the form"),
        ("plugin.k8s.a.b", "1", "plugin keys have the form"),
    ] {
        let err = set(&mut config, &discovery, key, value)
            .unwrap_err()
            .to_string();
        assert!(err.contains(expected), "{expected:?} not in {err}");
    }
}

#[test]
fn list_keys_covers_every_overridable_key() {
    let keys: Vec<String> = list_keys(&discovery())
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    assert_eq!(
        keys,
        [
            "plugin.k8s.depth",
            "plugin.k8s.short",
            "plugin.k8s.style",
            "plugin.k8s.timeout_ms",
            "plugin.k8s.ttl_secs",
        ]
    );
}

#[test]
fn overrides_round_trip_through_a_saved_file() {
    let temp = tempfile::TempDir::new().unwrap();
    let path = temp.path().join("config.toml");
    let path_str = path.to_str().unwrap();
    std::fs::write(&path, "# mine\n[ui]\nshow_icons = false\n").unwrap();

    let original = loader::load_config_from_file(path_str).unwrap();
    let mut updated = original.clone();
    set(&mut updated, &discovery(), "plugin.k8s.depth", "4").unwrap();
    set(&mut updated, &discovery(), "plugin.k8s.ttl_secs", "60").unwrap();
    loader::save_config(path_str, &original, &updated, &["plugin.k8s.depth"]).unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("# mine") && text.contains("[plugin.k8s]"),
        "{text}"
    );
    let reloaded = loader::load_config_from_file(path_str).unwrap();
    assert_eq!(reloaded.plugin, updated.plugin);
}
