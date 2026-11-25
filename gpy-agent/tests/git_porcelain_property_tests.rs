//! Property tests for the `git status --porcelain=v2 -z` parser.
//!
//! The oracle is the documented v2 format (git-status(1)), not the parser:
//! records are generated from a model, serialized, parsed back and compared.

#![allow(
    clippy::unwrap_used,
    reason = "tests: a failed unwrap is a test failure"
)]
#![allow(
    clippy::expect_used,
    reason = "tests: a failed expect is a test failure"
)]

use gpy_agent::git::FileStatus;
use gpy_agent::git::native::parser::{
    BranchHead, parse_v2_ahead_behind, parse_v2_changed_line, parse_v2_files, parse_v2_records,
};
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use std::collections::HashMap;
use std::path::PathBuf;

fn config() -> ProptestConfig {
    ProptestConfig {
        // A failure prints its shrunk counterexample; no regression file is
        // written into the source tree.
        failure_persistence: Some(Box::new(FileFailurePersistence::Off)),
        ..ProptestConfig::default()
    }
}

/// A filename git could emit under `-z`: any bytes but NUL, kept as UTF-8 here.
fn path_strategy() -> impl Strategy<Value = String> {
    "[^\0]{1,24}"
}

#[derive(Debug, Clone)]
enum Entry {
    Ordinary {
        xy: String,
        path: String,
    },
    Renamed {
        xy: String,
        path: String,
        orig: String,
    },
    Unmerged {
        xy: String,
        path: String,
    },
    Untracked {
        path: String,
    },
}

impl Entry {
    fn path(&self) -> &str {
        match self {
            Self::Ordinary { path, .. }
            | Self::Renamed { path, .. }
            | Self::Unmerged { path, .. }
            | Self::Untracked { path } => path,
        }
    }

    /// The `-z` records git prints for this entry.
    fn records(&self) -> Vec<String> {
        match self {
            Self::Ordinary { xy, path } => {
                vec![format!("1 {xy} N... 100644 100644 100644 aaaa bbbb {path}")]
            }
            Self::Renamed { xy, path, orig } => vec![
                format!("2 {xy} N... 100644 100644 100644 aaaa bbbb R100 {path}"),
                orig.clone(),
            ],
            Self::Unmerged { xy, path } => {
                vec![format!(
                    "u {xy} N... 100644 100644 100644 100644 a b c {path}"
                )]
            }
            Self::Untracked { path } => vec![format!("? {path}")],
        }
    }

    /// The status git-status(1) documents for this entry.
    fn expected(&self) -> FileStatus {
        match self {
            Self::Untracked { .. } => FileStatus {
                untracked: true,
                ..FileStatus::default()
            },
            Self::Ordinary { xy, .. } | Self::Renamed { xy, .. } | Self::Unmerged { xy, .. } => {
                let mut sides = xy.chars();
                let x = sides.next().unwrap();
                let y = sides.next().unwrap();
                let conflicted =
                    x == 'U' || y == 'U' || (x == 'A' && y == 'A') || (x == 'D' && y == 'D');
                if conflicted {
                    FileStatus {
                        conflicted: true,
                        ..FileStatus::default()
                    }
                } else {
                    FileStatus {
                        staged: x != '.',
                        unstaged: y != '.',
                        ..FileStatus::default()
                    }
                }
            }
        }
    }
}

fn xy_strategy() -> impl Strategy<Value = String> {
    let side = prop::sample::select(vec!['.', 'M', 'T', 'A', 'D', 'R', 'C', 'U']);
    (side.clone(), side).prop_map(|(x, y)| format!("{x}{y}"))
}

fn entry_strategy() -> impl Strategy<Value = Entry> {
    prop_oneof![
        (xy_strategy(), path_strategy()).prop_map(|(xy, path)| Entry::Ordinary { xy, path }),
        (xy_strategy(), path_strategy(), path_strategy())
            .prop_map(|(xy, path, orig)| Entry::Renamed { xy, path, orig }),
        (xy_strategy(), path_strategy()).prop_map(|(xy, path)| Entry::Unmerged { xy, path }),
        path_strategy().prop_map(|path| Entry::Untracked { path }),
    ]
}

proptest! {
    #![proptest_config(config())]

    /// Arbitrary text never panics any entry point.
    #[test]
    fn parsers_never_panic_on_arbitrary_text(output in "(.|\0){0,300}", max in 0_usize..1000) {
        let _state = parse_v2_records(&output, max);
        let _files = parse_v2_files(&output);
        let _line = parse_v2_changed_line(&output);
        let _ab = parse_v2_ahead_behind(&output, max);
    }

    /// Every generated entry is parsed back with the documented status, spaces
    /// and tabs in paths survive, and a rename's original-path record is
    /// consumed instead of becoming an entry of its own.
    #[test]
    fn generated_records_round_trip(generated in prop::collection::vec(entry_strategy(), 0..12)) {
        let mut seen = std::collections::HashSet::new();
        let entries: Vec<Entry> = generated
            .into_iter()
            .filter(|entry| seen.insert(entry.path().to_owned()))
            .collect();
        let mut output = String::new();
        for record in entries.iter().flat_map(Entry::records) {
            output.push_str(&record);
            output.push('\0');
        }

        let expected: HashMap<PathBuf, FileStatus> = entries
            .iter()
            .map(|entry| (PathBuf::from(entry.path()), entry.expected()))
            .collect();
        prop_assert_eq!(parse_v2_files(&output), expected);
    }

    /// Ahead/behind: a non-zero ceiling clamps and flags exactly the counts
    /// above it; zero is unlimited.
    #[test]
    fn ahead_behind_respects_the_ceiling(ahead in any::<u32>(), behind in any::<u32>(), max in 0_usize..100_000) {
        let (got_ahead, got_behind, ahead_capped, behind_capped) =
            parse_v2_ahead_behind(&format!("+{ahead} -{behind}"), max);
        if max == 0 {
            prop_assert_eq!((got_ahead, got_behind, ahead_capped, behind_capped), (ahead, behind, false, false));
        } else {
            let ceiling = u32::try_from(max).unwrap();
            prop_assert_eq!(got_ahead, ahead.min(ceiling));
            prop_assert_eq!(got_behind, behind.min(ceiling));
            prop_assert_eq!(ahead_capped, ahead > ceiling);
            prop_assert_eq!(behind_capped, behind > ceiling);
        }
    }

    /// The `# branch.head` header is taken verbatim, `(detached)` aside.
    #[test]
    fn branch_head_is_verbatim(name in "[^\0]{1,40}") {
        let state = parse_v2_records(format!("# branch.head {name}\0"), 0);
        let expected = if name == "(detached)" {
            BranchHead::Detached
        } else {
            BranchHead::Named(name)
        };
        prop_assert_eq!(state.branch, Some(expected));
    }
}
