//! Golden fidelity tests: GPY engine output vs. Starship default module formats.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]

use gpy_agent::template::{MapResolver, RenderContext, Span, render};

fn text_of(spans: &[Span]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect()
}

#[test]
fn git_branch_default_format() {
    // Starship: format = "on [$symbol$branch(:$remote_branch)]($style) "
    // $style is unset; as a style-spec variable it does not participate in any_var_nonempty — only content variables affect Conditional collapse.
    let resolver = MapResolver::from_pairs([("symbol", " "), ("branch", "main")]);
    let ctx = RenderContext::new(&resolver);
    let spans = render("on [$symbol$branch(:$remote_branch)]($style) ", &ctx).unwrap();
    assert_eq!(text_of(&spans), "on  main ");
}

#[test]
fn git_branch_with_remote() {
    let resolver = MapResolver::from_pairs([
        ("symbol", " "),
        ("branch", "main"),
        ("remote_branch", "origin/main"),
    ]);
    let ctx = RenderContext::new(&resolver);
    let spans = render("on [$symbol$branch(:$remote_branch)]($style) ", &ctx).unwrap();
    assert_eq!(text_of(&spans), "on  main:origin/main ");
}

#[test]
fn git_status_hidden_when_clean() {
    // Starship: format = '([\[$all_status$ahead_behind\]]($style) )'
    let resolver = MapResolver::default();
    let ctx = RenderContext::new(&resolver);
    let spans = render(r"([\[$all_status$ahead_behind\]]($style) )", &ctx).unwrap();
    assert_eq!(text_of(&spans), "");
}

#[test]
fn git_status_shown_when_dirty() {
    let resolver = MapResolver::from_pairs([("all_status", "!?")]);
    let ctx = RenderContext::new(&resolver);
    let spans = render(r"([\[$all_status$ahead_behind\]]($style) )", &ctx).unwrap();
    assert_eq!(text_of(&spans), "[!?] ");
}

#[test]
fn character_success_symbol_is_green_bold() {
    use gpy_agent::template::{Attr, Color};
    // Starship success_symbol = "[❯](bold green)"
    let resolver = MapResolver::default();
    let ctx = RenderContext::new(&resolver);
    let spans = render("[❯](bold green)", &ctx).unwrap();
    assert_eq!(spans.len(), 1_usize); // guard against trailing empty spans
    let span = spans.first().unwrap();
    assert_eq!(span.text, "❯");
    assert_eq!(span.style.fg, Some(Color::Named("green".to_owned())));
    assert!(span.style.attrs.contains(&Attr::Bold));
}

#[test]
fn language_version_shown_and_hidden() {
    // Starship language modules: format = "via [$symbol($version )]($style)"
    let fmt = "via [$symbol($version )]($style)";

    let with_version = MapResolver::from_pairs([("symbol", "🦀 "), ("version", "v1.75.0")]);
    let ctx = RenderContext::new(&with_version);
    let spans = render(fmt, &ctx).unwrap();
    assert_eq!(text_of(&spans), "via 🦀 v1.75.0 ");

    // No version: the inner `($version )` group collapses, leaving just the symbol.
    let no_version = MapResolver::from_pairs([("symbol", "🦀 ")]);
    let ctx_no_version = RenderContext::new(&no_version);
    let spans_no_version = render(fmt, &ctx_no_version).unwrap();
    assert_eq!(text_of(&spans_no_version), "via 🦀 ");
}

#[test]
fn cmd_duration_default_format() {
    // Starship: format = "took [$duration]($style) "
    let resolver = MapResolver::from_pairs([("duration", "2s")]);
    let ctx = RenderContext::new(&resolver);
    let spans = render("took [$duration]($style) ", &ctx).unwrap();
    assert_eq!(text_of(&spans), "took 2s ");
}
