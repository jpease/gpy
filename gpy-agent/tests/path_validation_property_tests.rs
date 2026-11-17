//! Property tests for `security::validate_path`.
//!
//! The policy under test: control characters and over-long paths are
//! rejected, `..` is rejected where it cannot be resolved, and nothing under
//! `/etc`, `/proc`, `/sys` or `/dev` is ever accepted, however it is spelled.

use gpy_agent::security::validate_path;
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use std::path::{Component, Path};

fn config() -> ProptestConfig {
    ProptestConfig {
        // A failure prints its shrunk counterexample; no regression file is
        // written into the source tree.
        failure_persistence: Some(Box::new(FileFailurePersistence::Off)),
        ..ProptestConfig::default()
    }
}

const DENIED: [&str; 4] = ["/etc", "/proc", "/sys", "/dev"];

fn segment() -> impl Strategy<Value = String> {
    "[a-z0-9_]{1,8}"
}

/// `/` or `//` or `/./` between segments: spellings of the same path.
fn separator() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec!["/", "//", "/./"])
}

proptest! {
    #![proptest_config(config())]

    /// A control character anywhere rejects the path.
    #[test]
    fn control_characters_are_rejected(
        head in "[a-z/]{0,12}",
        control in prop::sample::select(vec!['\0', '\n', '\r', '\t', '\u{1b}', '\u{7f}', '\u{85}']),
        tail in "[a-z/]{0,12}",
    ) {
        let path = format!("{head}{control}{tail}");
        prop_assert!(validate_path(&path).is_err(), "{path:?} was accepted");
    }

    /// A path past the length cap is rejected.
    #[test]
    fn overlong_paths_are_rejected(len in 4097_usize..8000) {
        let path = format!("/{}", "a".repeat(len));
        prop_assert!(validate_path(&path).is_err());
    }

    /// Nothing under a denied system directory is accepted, whatever the
    /// separators between segments.
    #[test]
    fn denied_directories_are_rejected(
        root in prop::sample::select(DENIED.to_vec()),
        rest in prop::collection::vec((separator(), segment()), 0..4),
    ) {
        let mut path = root.to_owned();
        for (separator, segment) in rest {
            path.push_str(separator);
            path.push_str(&segment);
        }
        prop_assert!(validate_path(&path).is_err(), "{path:?} was accepted");
    }

    /// A `..` that cannot be resolved against the filesystem is rejected, so
    /// it cannot step past the denylist lexically.
    #[test]
    fn unresolvable_parent_components_are_rejected(
        marker in segment(),
        before in prop::collection::vec(segment(), 0..3),
        after in prop::collection::vec(segment(), 0..3),
    ) {
        let path = format!(
            "/gpy-no-such-root-{marker}/{}/../{}",
            before.join("/"),
            after.join("/")
        );
        prop_assert!(validate_path(&path).is_err(), "{path:?} was accepted");
    }

    /// Whatever is accepted is free of `..` and outside every denied directory.
    #[test]
    fn accepted_paths_uphold_the_invariants(path in "[a-z./ ]{0,30}") {
        if let Ok(accepted) = validate_path(&path) {
            prop_assert!(
                !accepted.components().any(|part| matches!(part, Component::ParentDir)),
                "{path:?} -> {accepted:?} keeps a parent component"
            );
            for denied in DENIED {
                prop_assert!(
                    !accepted.starts_with(Path::new(denied)),
                    "{path:?} -> {accepted:?} is under {denied}"
                );
            }
        }
    }
}
