//! Property tests for the template parser and evaluator.
//!
//! Templates come from user config, so `render` must turn any string into
//! `Ok` or a `TemplateError` without panicking or overflowing the stack.

use gpy_agent::template::{EvalContext, MapResolver, SpanKind, TemplateError, render};
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;

fn config() -> ProptestConfig {
    ProptestConfig {
        // A failure prints its shrunk counterexample; no regression file is
        // written into the source tree.
        failure_persistence: Some(Box::new(FileFailurePersistence::Off)),
        ..ProptestConfig::default()
    }
}

/// Strings biased toward template syntax so the delimiters actually pair up.
fn templateish() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            4 => prop::sample::select(vec![
                "$", "${", "}", "(", ")", "[", "]", "\\", " ", "\n", "bold ", "red", "fg:#ff0000 ",
                "$path", "$git_branch", "\u{1b}", "\0",
            ]).prop_map(str::to_owned),
            1 => ".{0,4}",
        ],
        0..24,
    )
    .prop_map(|parts| parts.concat())
}

fn resolver() -> MapResolver {
    MapResolver::from_pairs([("path", "~/src"), ("git_branch", "main")])
}

proptest! {
    #![proptest_config(config())]

    /// Any text renders to `Ok` or an error; nothing panics.
    #[test]
    fn render_never_panics_on_arbitrary_text(template in ".{0,200}") {
        let resolver = resolver();
        let _outcome = render(&template, &EvalContext::new(&resolver));
    }

    /// Same, for strings built from the template's own delimiters.
    #[test]
    fn render_never_panics_on_templateish_text(template in templateish()) {
        let resolver = resolver();
        let _outcome = render(&template, &EvalContext::new(&resolver));
    }

    /// The memoized parse path agrees with itself: a repeat render of the
    /// same template yields the identical result, success or error.
    #[test]
    fn render_is_repeatable(template in templateish()) {
        let resolver = resolver();
        let ctx = EvalContext::new(&resolver);
        prop_assert_eq!(render(&template, &ctx), render(&template, &ctx));
    }

    /// Literal text never reaches the terminal with a control character other
    /// than newline: control characters either error or are stripped.
    #[test]
    fn rendered_text_has_no_stray_control_characters(template in templateish()) {
        let resolver = resolver();
        if let Ok(spans) = render(&template, &EvalContext::new(&resolver)) {
            for span in spans.iter().filter(|span| span.kind == SpanKind::Text) {
                prop_assert!(
                    !span.text.chars().any(|c| c.is_control() && c != '\n'),
                    "control character in rendered span {:?}",
                    span.text
                );
            }
        }
    }

    /// Group nesting beyond the limit is rejected, not recursed into.
    #[test]
    fn excessive_nesting_is_rejected_not_overflowed(depth in 129_usize..4000, open in prop::sample::select(vec!['(', '['])) {
        let close = if open == '(' { ')' } else { ']' };
        let template = format!(
            "{}x{}",
            open.to_string().repeat(depth),
            close.to_string().repeat(depth)
        );
        let resolver = resolver();
        let outcome = render(&template, &EvalContext::new(&resolver));
        prop_assert!(outcome.is_err(), "depth {depth} should not render");
        if let Err(error) = outcome {
            prop_assert!(
                matches!(error, TemplateError::TooDeep { .. } | TemplateError::Unbalanced { .. }),
                "unexpected error {error:?}"
            );
        }
    }
}
