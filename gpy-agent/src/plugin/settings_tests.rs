#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::missing_panics_doc,
    clippy::shadow_unrelated,
    reason = "tests: panics are the assertions, and each `err` binds a fresh case"
)]

use super::*;
use crate::plugin::ScriptSegmentManifest;

fn manifest_err(settings: &str) -> String {
    ScriptSegmentManifest::parse(&format!("command = \"a.sh\"\n{settings}"))
        .unwrap_err()
        .to_string()
}

#[test]
fn declared_settings_parse_with_types_defaults_and_constraints() {
    let manifest = ScriptSegmentManifest::parse(
        r#"
        command = "a.sh"
        [settings.show_namespace]
        type = "bool"
        default = true
        description = "Show the namespace"
        [settings.max_len]
        type = "int"
        default = 3
        min = 0
        max = 10
        [settings.style]
        type = "string"
        default = "bold"
        choices = ["bold", "dim"]
        "#,
    )
    .unwrap();
    let get = |key: &str| {
        manifest
            .settings
            .get(&SettingKey::new(key).unwrap())
            .expect("declared setting")
    };
    assert_eq!(
        get("show_namespace").default_value(),
        SettingValue::Bool(true)
    );
    assert_eq!(
        get("show_namespace").description(),
        Some("Show the namespace")
    );
    assert_eq!(get("max_len").summary(), "int 0..=10");
    assert_eq!(get("style").summary(), "string bold|dim");
    assert_eq!(
        SettingKey::new("max_len").unwrap().env_var(),
        "GPY_OPT_max_len"
    );
}

#[test]
fn declaration_errors_name_the_key() {
    let cases = [
        (
            "[settings.n]\ntype = \"int\"\ndefault = 11\nmax = 10\n",
            "settings.n: default: 11 is above max 10",
        ),
        (
            "[settings.n]\ntype = \"int\"\ndefault = 1\nmin = 5\nmax = 2\n",
            "settings.n: min 5 is above max 2",
        ),
        (
            "[settings.s]\ntype = \"string\"\ndefault = \"x\"\nchoices = [\"a\"]\n",
            "settings.s: default: 'x' is not one of a",
        ),
        (
            "[settings.s]\ntype = \"string\"\ndefault = \"a\"\nchoices = [\"a\", \"a\"]\n",
            "settings.s: choice 'a' is listed twice",
        ),
        (
            "[settings.s]\ntype = \"string\"\ndefault = \"a\\u0000\"\n",
            "settings.s: default:",
        ),
        (
            "[settings.ttl_secs]\ntype = \"int\"\ndefault = 1\n",
            "settings.ttl_secs: 'ttl_secs' is a manifest field",
        ),
    ];
    for (settings, expected) in cases {
        let err = manifest_err(settings);
        assert!(err.contains(expected), "{expected:?} not in {err}");
    }
}

#[test]
fn malformed_declarations_are_rejected() {
    for (settings, needle) in [
        ("[settings.b]\ntype = \"bool\"\ndefault = \"yes\"\n", "b"),
        ("[settings.b]\ntype = \"float\"\ndefault = 1.0\n", "float"),
        (
            "[settings.b]\ntype = \"bool\"\ndefault = true\nmin = 1\n",
            "min",
        ),
        ("[settings.Bad]\ntype = \"bool\"\ndefault = true\n", "Bad"),
        ("[settings.b]\ndefault = true\n", "type"),
    ] {
        let err = manifest_err(settings);
        assert!(err.contains(needle), "{needle:?} not in {err}");
    }
}

#[test]
fn setting_keys_are_snake_case() {
    for good in ["a", "_x", "show_ns2"] {
        assert!(SettingKey::new(good).is_ok(), "{good:?}");
    }
    for bad in ["", "A", "2x", "a-b", "a.b", &"a".repeat(65)] {
        assert!(SettingKey::new(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn values_are_checked_against_the_declaration() {
    let int = SettingDecl::Int {
        default: 1,
        min: Some(0),
        max: Some(5),
        description: None,
    };
    assert!(int.check(&SettingValue::Int(5)).is_ok());
    assert!(int.check(&SettingValue::Int(6)).is_err());
    let err = int
        .check(&SettingValue::Bool(true))
        .unwrap_err()
        .to_string();
    assert!(err.contains("expected int, got bool"), "{err}");
}

#[test]
fn cli_strings_parse_by_declared_type() {
    let boolean = SettingDecl::Bool {
        default: false,
        description: None,
    };
    assert_eq!(boolean.parse_str("on").unwrap(), SettingValue::Bool(true));
    assert!(boolean.parse_str("maybe").is_err());
    let int = SettingDecl::Int {
        default: 0,
        min: None,
        max: Some(9),
        description: None,
    };
    assert_eq!(int.parse_str("7").unwrap(), SettingValue::Int(7));
    assert!(int.parse_str("x").is_err());
    assert!(int.parse_str("10").is_err());
    let string = SettingDecl::String {
        default: "a".to_owned(),
        choices: vec!["a".to_owned(), "b".to_owned()],
        description: None,
    };
    assert_eq!(
        string.parse_str("b").unwrap(),
        SettingValue::String("b".to_owned())
    );
    assert!(string.parse_str("c").is_err());
}

#[test]
fn values_reach_the_script_as_plain_strings() {
    assert_eq!(SettingValue::Bool(false).to_string(), "false");
    assert_eq!(SettingValue::Int(-3).to_string(), "-3");
    assert_eq!(SettingValue::String("x y".to_owned()).to_string(), "x y");
}
