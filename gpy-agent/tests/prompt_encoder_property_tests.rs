//! Property test for the prompt encoders (#752).
//!
//! Every [`PromptDialect`] must render an arbitrary span sequence so that each
//! visible character carries exactly the style of its span: nothing leaks from
//! an earlier span, the visible text is the spans' text verbatim (whatever
//! prompt-hostile characters it holds), and the output ends in the default
//! style. The oracles are independent of the encoder's own SGR code:
//!
//! - [`PromptDialect::Ansi`]: the SGR state machine in `common/sgr.rs`
//!   (`interpret`).
//! - [`PromptDialect::BashPrompt`]: real bash expands the encoding as `PS1`
//!   (`${PS1@P}`, `promptvars` on, bash >= 4.4), and the result must equal the
//!   Ansi encoding of the same spans byte for byte once bash's `\001`/`\002`
//!   zero-width markers are stripped. Byte equality is the right bar here:
//!   bash copies SGR bytes and decoded text through untouched, so any
//!   difference is an escaping or marker bug.
//! - [`PromptDialect::ZshPrompt`]: real zsh expands the encoding with
//!   `print -rP` (`prompt_subst`, `prompt_percent`, `no_prompt_bang`). zsh owns
//!   its output bytes (it drops `%{ %}` and is free to normalize), so the
//!   result is judged by the same per-character SGR interpretation, not by raw
//!   bytes.
//!
//! Each shell test hands one shell process a whole batch of encodings
//! (NUL-separated, through a file) and reads the expansions back the same
//! way, so the run stays fast and nothing is interpolated into shell code.
//! A missing or too-old shell skips visibly (a failure under `CI`).

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::default_numeric_fallback)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::many_single_char_names)]
// Counterexample reports print the spans, encodings and expansions with `{:?}`.
#![allow(clippy::use_debug)]

use gpy_agent::formatter::PromptDialect;
use gpy_agent::template::{Attr, Color, Span, SpanKind, Style};
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use std::fmt::Write as _;
use std::process::Command;

#[path = "common/sgr.rs"]
mod sgr;
#[path = "common/skip.rs"]
mod skip;

use sgr::{BLINK, BOLD, DIM, HIDDEN, INVERSE, ITALIC, Look, STRIKE, Shade, UNDERLINE, interpret};

/// The look a `Style` asks for, from the specification of the style (not the
/// encoder).
fn look_of(style: &Style) -> Look {
    let attrs = style.attrs.iter().fold(0_u8, |bits, attr| {
        bits | match attr {
            Attr::Bold => BOLD,
            Attr::Dimmed => DIM,
            Attr::Italic => ITALIC,
            Attr::Underline => UNDERLINE,
            Attr::Blink => BLINK,
            Attr::Inverted => INVERSE,
            Attr::Hidden => HIDDEN,
            Attr::Strikethrough => STRIKE,
        }
    });
    Look {
        attrs,
        fg: style.fg.as_ref().and_then(shade_of),
        bg: style.bg.as_ref().and_then(shade_of),
    }
}

/// Names the engine accepts, in basic-color index order.
const BASIC_NAMES: [&str; 8] = [
    "black", "red", "green", "yellow", "blue", "purple", "cyan", "white",
];

fn shade_of(color: &Color) -> Option<Shade> {
    match color {
        Color::Named(name) => {
            let (bright, base) = name
                .strip_prefix("bright-")
                .map_or((false, name.as_str()), |rest| (true, rest));
            let index = BASIC_NAMES.iter().position(|known| *known == base)?;
            Some(Shade::Basic {
                index: u8::try_from(index).ok()?,
                bright,
            })
        }
        Color::Ansi256(index) => Some(Shade::Indexed(u16::from(*index))),
        Color::Rgb { r, g, b } => Some(Shade::Rgb(u16::from(*r), u16::from(*g), u16::from(*b))),
        Color::Palette(_) | Color::PrevFg | Color::PrevBg => None,
    }
}

/// Every character of `spans` with the look its own span asks for.
fn expected_drawing(spans: &[Span]) -> Vec<(char, Look)> {
    spans
        .iter()
        .flat_map(|span| {
            let look = look_of(&span.style);
            span.text.chars().map(move |ch| (ch, look))
        })
        .collect()
}

/// `Ok` when `output` draws exactly `spans` and ends in the default look.
fn check_drawing(output: &str, spans: &[Span]) -> Result<(), String> {
    let (drawn, last) = interpret(output)?;
    let expected = expected_drawing(spans);
    if drawn != expected {
        let first_difference = drawn
            .iter()
            .zip(&expected)
            .position(|(got, want)| got != want)
            .map_or_else(
                || {
                    format!(
                        "lengths differ: drew {}, wanted {}",
                        drawn.len(),
                        expected.len()
                    )
                },
                |position| {
                    format!(
                        "character {position}: drew {:?}, wanted {:?}",
                        drawn.get(position),
                        expected.get(position)
                    )
                },
            );
        return Err(format!("drawing differs ({first_difference})"));
    }
    if last != Look::default() {
        return Err(format!("output ends with {last:?} still active"));
    }
    Ok(())
}

// ============================================================================
// Generators
// ============================================================================

const COLOR_NAMES: [&str; 18] = [
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "purple",
    "cyan",
    "white",
    "default",
    "bright-black",
    "bright-red",
    "bright-green",
    "bright-yellow",
    "bright-blue",
    "bright-purple",
    "bright-cyan",
    "bright-white",
    "bright-white",
];

const ATTRS: [Attr; 8] = [
    Attr::Bold,
    Attr::Dimmed,
    Attr::Italic,
    Attr::Underline,
    Attr::Blink,
    Attr::Inverted,
    Attr::Hidden,
    Attr::Strikethrough,
];

/// Characters bash, zsh or a terminal treat specially in prompt text.
const HOSTILE: [char; 36] = [
    '$', '`', '\\', '%', '!', '{', '}', '\'', '"', '\n', '\t', ' ', '(', ')', '[', ']', 'a', 'b',
    'x', 'u', 'h', 'w', 'é', '日', '☃', '~', '#', ';', '&', '|', '<', '>', '*', '?', '^', '@',
];

fn color_strategy() -> impl Strategy<Value = Color> {
    prop_oneof![
        4 => prop::sample::select(COLOR_NAMES.to_vec()).prop_map(|name| Color::Named(name.to_owned())),
        1 => any::<u8>().prop_map(Color::Ansi256),
        1 => (any::<u8>(), any::<u8>(), any::<u8>()).prop_map(|(r, g, b)| Color::Rgb { r, g, b }),
    ]
}

fn style_strategy() -> impl Strategy<Value = Style> {
    (
        prop::option::of(color_strategy()),
        prop::option::of(color_strategy()),
        prop::collection::vec(prop::sample::select(ATTRS.to_vec()), 0..4),
    )
        .prop_map(|(fg, bg, attrs)| Style { fg, bg, attrs })
}

fn text_strategy() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            6 => prop::sample::select(HOSTILE.to_vec()),
            1 => any::<char>().prop_filter("printable", |ch| !ch.is_control()),
        ],
        0..6,
    )
    .prop_map(String::from_iter)
}

fn span_strategy() -> impl Strategy<Value = Span> {
    (text_strategy(), style_strategy()).prop_map(|(text, style)| Span {
        text,
        style,
        kind: SpanKind::Text,
    })
}

/// A batch of independent span sequences (one shell process expands them all).
fn batch_strategy() -> impl Strategy<Value = Vec<Vec<Span>>> {
    prop::collection::vec(prop::collection::vec(span_strategy(), 0..6), 1..=48)
}

fn config(cases: u32) -> ProptestConfig {
    ProptestConfig {
        cases,
        // A failure prints its shrunk counterexample; no regression file is
        // written into the source tree.
        failure_persistence: Some(Box::new(FileFailurePersistence::Off)),
        max_shrink_iters: 4096,
        ..ProptestConfig::default()
    }
}

// ============================================================================
// Shell oracles
// ============================================================================

/// A real shell that expands prompt source, fed a NUL-separated batch.
struct ShellOracle {
    program: &'static str,
    args: &'static [&'static str],
}

/// Expand each `PS1` in `$1` (NUL-separated) with bash's prompt decoding.
const BASH_SCRIPT: &str = "shopt -s promptvars\n\
    while IFS= read -r -d '' PS1; do printf '%s\\0' \"${PS1@P}\"; done < \"$1\"";

/// Expand each `PROMPT` in `$1` (NUL-separated) with zsh's `print -P`.
const ZSH_SCRIPT: &str = "setopt prompt_subst prompt_percent no_prompt_bang\n\
    while IFS= read -r -d '' line; do print -rnP -- \"$line\"; printf '\\0'; done < \"$1\"";

/// The shell that judges `dialect`, or `None` when the model of
/// [`interpret`] judges the encoding directly. Exhaustive on purpose: a new
/// dialect must choose an oracle here.
const fn oracle_for(dialect: PromptDialect) -> Option<ShellOracle> {
    match dialect {
        PromptDialect::Ansi => None,
        PromptDialect::BashPrompt => Some(ShellOracle {
            program: "bash",
            args: &["--norc", "--noprofile", "-c", BASH_SCRIPT, "bash"],
        }),
        PromptDialect::ZshPrompt => Some(ShellOracle {
            program: "zsh",
            args: &["-f", "-c", ZSH_SCRIPT, "zsh"],
        }),
    }
}

impl ShellOracle {
    /// Whether this shell can act as the oracle, or why not.
    fn availability(&self) -> Result<(), String> {
        let probe = if self.program == "bash" {
            // `${var@P}` needs bash 4.4; macOS ships 3.2 as /bin/bash.
            "[ \"${BASH_VERSINFO[0]}\" -gt 4 ] || { [ \"${BASH_VERSINFO[0]}\" -eq 4 ] && [ \"${BASH_VERSINFO[1]}\" -ge 4 ]; }"
        } else {
            ":"
        };
        match Command::new(self.program).args(["-c", probe]).output() {
            Ok(output) if output.status.success() => Ok(()),
            Ok(_) => Err(format!(
                "{} is older than the required version (bash >= 4.4)",
                self.program
            )),
            Err(err) => Err(format!("{} not found in PATH ({err})", self.program)),
        }
    }

    /// Expand every encoding in one shell process.
    fn expand(&self, encodings: &[String]) -> Result<Vec<String>, String> {
        let dir = tempfile::tempdir().map_err(|err| err.to_string())?;
        let input = dir.path().join("encodings");
        let mut bytes = Vec::new();
        for encoding in encodings {
            bytes.extend_from_slice(encoding.as_bytes());
            bytes.push(0);
        }
        std::fs::write(&input, bytes).map_err(|err| err.to_string())?;
        // Run inside the scratch directory so a broken escape that lets a
        // redirection through cannot write anywhere that matters.
        let output = Command::new(self.program)
            .args(self.args)
            .arg(&input)
            .current_dir(dir.path())
            .env_remove("BASH_ENV")
            .env_remove("ENV")
            .output()
            .map_err(|err| format!("spawning {}: {err}", self.program))?;
        if !output.status.success() {
            return Err(format!(
                "{} exited {:?}: {}",
                self.program,
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        let text = String::from_utf8(output.stdout).map_err(|err| err.to_string())?;
        let mut expansions: Vec<String> = text.split('\0').map(str::to_owned).collect();
        // The output ends in a terminator, which leaves one empty tail piece.
        if expansions.pop().as_deref() != Some("") {
            return Err("shell output was not NUL-terminated".to_owned());
        }
        if expansions.len() != encodings.len() {
            return Err(format!(
                "sent {} encodings, got {} expansions back",
                encodings.len(),
                expansions.len()
            ));
        }
        Ok(expansions)
    }
}

/// Encode every sequence in `dialect`, have its oracle expand them, and check
/// each result. `Err` describes the first failing sequence.
fn check_batch(dialect: PromptDialect, batch: &[Vec<Span>]) -> Result<(), String> {
    let encodings: Vec<String> = batch.iter().map(|spans| dialect.encode(spans)).collect();
    let shown = match oracle_for(dialect) {
        None => encodings.clone(),
        Some(shell) => shell.expand(&encodings)?,
    };
    for ((spans, encoding), expansion) in batch.iter().zip(&encodings).zip(&shown) {
        let mut judged = expansion.clone();
        if dialect == PromptDialect::BashPrompt {
            // Zero-width markers readline consumes; bash prints them as \001/\002.
            judged.retain(|ch| ch != '\u{1}' && ch != '\u{2}');
            let ansi = PromptDialect::Ansi.encode(spans);
            if judged != ansi {
                return Err(format!(
                    "{dialect:?}: bash expansion differs from the Ansi encoding\n  spans: {spans:?}\n  encoded: {encoding:?}\n  bash: {judged:?}\n  ansi: {ansi:?}"
                ));
            }
        }
        if let Err(reason) = check_drawing(&judged, spans) {
            let mut report = String::new();
            let _ = write!(
                report,
                "{dialect:?}: {reason}\n  spans: {spans:?}\n  encoded: {encoding:?}\n  shown: {expansion:?}"
            );
            return Err(report);
        }
    }
    Ok(())
}

/// Run the property for one dialect, skipping visibly if its shell is absent.
fn run_dialect(dialect: PromptDialect, cases: u32) {
    if let Some(shell) = oracle_for(dialect)
        && let Err(reason) = shell.availability()
    {
        skip::skip_test(&reason);
        return;
    }
    let mut runner = proptest::test_runner::TestRunner::new(config(cases));
    let outcome = runner.run(&batch_strategy(), |batch| {
        check_batch(dialect, &batch).map_err(TestCaseError::fail)
    });
    assert!(outcome.is_ok(), "{}", outcome.unwrap_err());
}

// ============================================================================
// Tests
// ============================================================================

#[test]
fn ansi_every_character_carries_exactly_its_spans_style() {
    run_dialect(PromptDialect::Ansi, 256);
}

#[test]
fn bash_prompt_expands_to_the_ansi_encoding_of_the_same_spans() {
    run_dialect(PromptDialect::BashPrompt, 16);
}

#[test]
fn zsh_prompt_draws_every_character_with_exactly_its_spans_style() {
    run_dialect(PromptDialect::ZshPrompt, 16);
}

/// Every dialect is covered by a test above (the match in `oracle_for` is
/// exhaustive; this pins the list the instant cache iterates).
#[test]
fn every_prompt_dialect_has_an_oracle_test() {
    assert_eq!(
        PromptDialect::ALL,
        [
            PromptDialect::Ansi,
            PromptDialect::BashPrompt,
            PromptDialect::ZshPrompt
        ]
    );
}

fn plain(text: &str, style: Style) -> Span {
    Span {
        text: text.to_owned(),
        style,
        kind: SpanKind::Text,
    }
}

/// The oracle itself must see a leak: the pre-fix bytes for
/// `[bold green "a"][blue "b"]` are rejected, the post-fix bytes accepted.
#[test]
fn oracle_distinguishes_a_leak_from_a_clean_encoding() {
    let spans = [
        plain(
            "a",
            Style {
                fg: Some(Color::Named("green".to_owned())),
                bg: None,
                attrs: vec![Attr::Bold],
            },
        ),
        plain(
            "b",
            Style {
                fg: Some(Color::Named("blue".to_owned())),
                bg: None,
                attrs: vec![],
            },
        ),
    ];
    assert!(check_drawing("\x1b[1;32ma\x1b[34mb\x1b[0m", &spans).is_err());
    assert!(check_drawing("\x1b[1;32ma\x1b[0;34mb\x1b[0m", &spans).is_ok());
    assert!(check_drawing("\x1b[1;32ma\x1b[0;34mb", &spans).is_err());
}
