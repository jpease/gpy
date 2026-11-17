//! Property tests for the base16 and Starship importers.
//!
//! Both read third-party files, so any text must come back as `Ok` or a typed
//! error; and a valid base16 scheme must survive a parse unchanged.

#![allow(
    clippy::unwrap_used,
    reason = "tests: a failed unwrap is a test failure"
)]
#![allow(
    clippy::expect_used,
    reason = "tests: a failed expect is a test failure"
)]

use gpy_agent::import::base16::{Hex, SchemeSystem, parse_scheme};
use gpy_agent::import::starship;
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use std::fmt::Write as _;

fn config() -> ProptestConfig {
    ProptestConfig {
        // A failure prints its shrunk counterexample; no regression file is
        // written into the source tree.
        failure_persistence: Some(Box::new(FileFailurePersistence::Off)),
        ..ProptestConfig::default()
    }
}

/// A format string from Starship's own syntax, so references, escapes and
/// style groups are exercised, not just noise.
fn format_string() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            3 => prop::sample::select(vec![
                "$", "${", "}", "\\", "[", "]", "(", ")", "$directory", "${git_branch}",
                "$character", "$time", "$custom", " ", "\n", "$all", "$fill",
            ]).prop_map(str::to_owned),
            1 => ".{0,4}",
        ],
        0..20,
    )
    .prop_map(|parts| parts.concat())
}

proptest! {
    #![proptest_config(config())]

    /// Arbitrary text never panics the scheme parser.
    #[test]
    fn scheme_parser_never_panics(input in "(.|\n){0,400}") {
        let _outcome = parse_scheme(&input);
    }

    /// A scheme written from a model parses back to that model, in the nested
    /// `palette:` layout and the legacy flat one, with or without `#`.
    #[test]
    fn valid_scheme_round_trips(
        name in "[A-Za-z][A-Za-z0-9 ]{0,15}",
        base24 in any::<bool>(),
        nested in any::<bool>(),
        hash in any::<bool>(),
        upper in any::<bool>(),
        colors in prop::collection::vec(any::<[u8; 3]>(), 24),
    ) {
        let slot_count = if base24 { 24 } else { 16 };
        let mut text = String::new();
        writeln!(text, "system: \"{}\"", if base24 { "base24" } else { "base16" }).unwrap();
        writeln!(text, "name: \"{name}\"").unwrap();
        if nested {
            text.push_str("palette:\n");
        }
        for (slot, [r, g, b]) in colors.iter().take(slot_count).enumerate() {
            let indent = if nested { "  " } else { "" };
            let mark = if hash { "#" } else { "" };
            let mut value = format!("{r:02x}{g:02x}{b:02x}");
            if upper {
                value = value.to_ascii_uppercase();
            }
            writeln!(text, "{indent}base{slot:02X}: \"{mark}{value}\"").unwrap();
        }

        let scheme = parse_scheme(&text).unwrap();
        prop_assert_eq!(scheme.system, if base24 { SchemeSystem::Base24 } else { SchemeSystem::Base16 });
        prop_assert_eq!(scheme.name, name.trim());
        prop_assert_eq!(scheme.slots.len(), slot_count);
        for (slot, [r, g, b]) in colors.iter().take(slot_count).enumerate() {
            let index = u8::try_from(slot).unwrap();
            prop_assert_eq!(
                scheme.slots.get(&index).map(Hex::as_css),
                Some(format!("#{r:02x}{g:02x}{b:02x}"))
            );
        }
    }

    /// Arbitrary text never panics the Starship TOML reader.
    #[test]
    fn starship_parse_never_panics(input in "(.|\n){0,400}") {
        let _outcome = starship::parse(&input);
    }

    /// Any format string in the root and in a module table builds to an
    /// outcome, and a successful theme is itself valid TOML.
    #[test]
    fn starship_build_survives_any_format(root in format_string(), module in format_string()) {
        let mut config = toml::Table::new();
        config.insert("format".to_owned(), toml::Value::String(root));
        let mut directory = toml::Table::new();
        directory.insert("format".to_owned(), toml::Value::String(module));
        config.insert("directory".to_owned(), toml::Value::Table(directory));
        let source = toml::to_string(&config).unwrap();

        let model = starship::parse(&source).unwrap();
        if let Ok(artifacts) = starship::build(&model, "prop")
            && let Ok(theme) = artifacts.theme_toml()
        {
            prop_assert!(theme.parse::<toml::Table>().is_ok(), "theme is not TOML:\n{theme}");
        }
    }
}
