//! Evaluator: AST + context -> styled spans.
//!
//! ## Style inheritance note
//!
//! An unstyled `[text]` group (no `(style)` suffix) renders its children with
//! the **default** style — it does **not** inherit the surrounding style. This
//! is a deliberate consequence of the permissive grammar: every styled group
//! introduces its own style scope.

use crate::template::Result;
use crate::template::TemplateError;
use crate::template::ast::Node;
use crate::template::parse::parse_cached;
use crate::template::style::{Color, Palette, Style, parse_style};
use std::collections::HashMap;

/// Resolves template variable names to their string values.
pub trait VariableResolver {
    /// Return the value for `name`, or `None` if unset/empty.
    fn resolve(&self, name: &str) -> Option<String>;
}

/// A simple in-memory resolver, primarily for tests and the importer.
#[derive(Debug, Default)]
pub struct MapResolver {
    values: HashMap<String, String>,
}

impl MapResolver {
    /// Build a resolver from `(name, value)` pairs. Empty values are treated as unset.
    ///
    /// Accepts any key/value types that convert to [`String`], so both
    /// `&'static str` literals and runtime-owned [`String`]s work.
    #[must_use]
    pub fn from_pairs<I, K, V>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let mut values = HashMap::new();
        for (key, val) in pairs {
            let value = val.into();
            if !value.is_empty() {
                values.insert(key.into(), value);
            }
        }
        Self { values }
    }
}

impl VariableResolver for MapResolver {
    fn resolve(&self, name: &str) -> Option<String> {
        self.values.get(name).cloned()
    }
}

/// Evaluation context: variable resolver, palette, and previous-segment colors.
pub struct RenderContext<'a> {
    resolver: &'a dyn VariableResolver,
    palette: Palette,
    prev_fg: Option<Color>,
    prev_bg: Option<Color>,
}

impl<'a> RenderContext<'a> {
    /// Create a context backed by `resolver`.
    #[must_use]
    pub fn new(resolver: &'a dyn VariableResolver) -> Self {
        Self {
            resolver,
            palette: Palette::default(),
            prev_fg: None,
            prev_bg: None,
        }
    }

    /// Attach a palette for `Color::Palette` resolution.
    #[must_use]
    pub fn with_palette(mut self, palette: Palette) -> Self {
        self.palette = palette;
        self
    }

    /// Attach previous-segment colors for `prev_fg`/`prev_bg`.
    #[must_use]
    pub fn with_prev_colors(mut self, fg: Option<Color>, bg: Option<Color>) -> Self {
        self.prev_fg = fg;
        self.prev_bg = bg;
        self
    }

    /// # Errors
    ///
    /// Returns [`TemplateError::UnknownColor`] when `color` is a palette name not in `self.palette`.
    fn resolve_color(&self, color: &Color) -> Result<Color> {
        match color {
            // Starship semantics: a palette entry shadows the standard ANSI name.
            // Palette miss → keep the standard named color unchanged.
            Color::Named(name) => Ok(self.palette.get(name).unwrap_or_else(|| color.clone())),
            Color::Palette(name) => self
                .palette
                .get(name)
                .ok_or_else(|| TemplateError::UnknownColor { name: name.clone() }),
            Color::PrevFg => Ok(self
                .prev_fg
                .clone()
                .unwrap_or_else(|| Color::Named("default".to_owned()))),
            Color::PrevBg => Ok(self
                .prev_bg
                .clone()
                .unwrap_or_else(|| Color::Named("default".to_owned()))),
            Color::Rgb { .. } | Color::Ansi256(_) => Ok(color.clone()),
        }
    }

    /// # Errors
    ///
    /// Returns an error if any color in `style` cannot be resolved via [`Self::resolve_color`].
    fn resolve_style(&self, style: &Style) -> Result<Style> {
        let fg = match &style.fg {
            Some(color) => Some(self.resolve_color(color)?),
            None => None,
        };
        let bg = match &style.bg {
            Some(color) => Some(self.resolve_color(color)?),
            None => None,
        };
        Ok(Style {
            fg,
            bg,
            attrs: style.attrs.clone(),
        })
    }
}

/// A run of text sharing one resolved style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    /// The text content.
    pub text: String,
    /// The style applied to `text`.
    pub style: Style,
}

/// Parse and evaluate `template` against `ctx`.
///
/// # Errors
///
/// Propagates parse and style errors.
pub fn render(template: &str, ctx: &RenderContext<'_>) -> Result<Vec<Span>> {
    let ast = parse_cached(template)?;
    let mut spans: Vec<Span> = Vec::new();
    eval_nodes(&ast, &Style::default(), ctx, &mut spans)?;
    Ok(spans)
}

/// Recursively evaluate `nodes` into styled spans.
///
/// # Errors
///
/// Returns an error if style resolution or a nested [`eval_nodes`] call fails.
fn eval_nodes(
    nodes: &[Node],
    inherited: &Style,
    ctx: &RenderContext<'_>,
    out: &mut Vec<Span>,
) -> Result<()> {
    // First pass: evaluate every child into its own buffer so we know, for
    // each index, whether that child produced any spans. This lookahead is
    // what lets a whitespace-only literal collapse when both of its immediate
    // group neighbors turned out empty (issue #351).
    let mut per_node: Vec<Vec<Span>> = Vec::with_capacity(nodes.len());
    for node in nodes {
        let mut buf: Vec<Span> = Vec::new();
        match node {
            Node::Literal(text) => push_text(&mut buf, text, inherited),
            Node::Var(name) => {
                if let Some(value) = ctx.resolver.resolve(name) {
                    push_text(&mut buf, &sanitize_control_chars(&value), inherited);
                }
            }
            Node::Styled {
                style_spec,
                children,
            } => {
                let resolved_spec = substitute_vars(style_spec, ctx);
                let parsed = parse_style(&resolved_spec)?;
                let style = ctx.resolve_style(&parsed)?;
                eval_nodes(children, &style, ctx, &mut buf)?;
            }
            Node::Conditional(children) => {
                if any_var_nonempty(children, ctx) {
                    eval_nodes(children, inherited, ctx, &mut buf)?;
                }
            }
        }
        per_node.push(buf);
    }

    // A neighbor "counts" for the sandwich rule only if it is a group node
    // (`Styled`/`Conditional`) that evaluated to zero spans. Bare `Var`/plain
    // `Literal` neighbors never qualify.
    let is_empty_group_at = |index: usize| -> bool {
        matches!(
            nodes.get(index),
            Some(Node::Styled { .. } | Node::Conditional(_))
        ) && per_node.get(index).is_some_and(Vec::is_empty)
    };

    // Second pass: flatten the buffers in original order, dropping only a
    // whitespace-only literal that is sandwiched between two empty groups.
    for (index, (node, buf)) in nodes.iter().zip(per_node.iter()).enumerate() {
        let is_whitespace_literal = matches!(
            node,
            Node::Literal(text) if !text.is_empty() && text.chars().all(char::is_whitespace)
        );
        let sandwiched = is_whitespace_literal
            && index.checked_sub(1).is_some_and(is_empty_group_at)
            && is_empty_group_at(index.saturating_add(1_usize));
        if sandwiched {
            continue;
        }
        out.extend(buf.iter().cloned());
    }
    Ok(())
}

/// Strip C0/C1 control characters from a **data-derived** variable value before
/// it becomes a rendered span.
///
/// Variable values come from unconstrained sources — most dangerously directory
/// names, which on Unix may contain any byte but `/` and NUL, including ESC
/// (`0x1b`) and complete OSC sequences. Forwarding them verbatim lets a
/// directory such as `$'\x1b]0;pwned\x07evil'` inject terminal escapes into the
/// victim's prompt the moment they `cd` into it (title spoofing, cursor and
/// line manipulation, hidden/overwritten text). This is the single choke point
/// for the default `fish-ansi` render path: every `$var` substitution passes
/// through here, so directory, branch, language, and version values are all
/// covered. We strip rather than visibly-escape so removed bytes cost no prompt
/// columns and cannot desync powerline separator alignment.
///
/// `char::is_control()` matches exactly U+0000–U+001F, U+007F (DEL), and the C1
/// range U+0080–U+009F. Theme-authored template literals and the ANSI the style
/// encoder emits are *not* routed through here, so segment styling is preserved.
/// See #425.
fn sanitize_control_chars(value: &str) -> String {
    if value.chars().any(char::is_control) {
        value.chars().filter(|c| !c.is_control()).collect()
    } else {
        value.to_owned()
    }
}

fn push_text(out: &mut Vec<Span>, text: &str, style: &Style) {
    if text.is_empty() {
        return;
    }
    out.push(Span {
        text: text.to_owned(),
        style: style.clone(),
    });
}

/// Replace `$name`/`${name}` inside a raw style spec with resolved values.
fn substitute_vars(spec: &str, ctx: &RenderContext<'_>) -> String {
    let Ok(nodes) = parse_cached(spec) else {
        return spec.to_owned();
    };
    let mut result = String::new();
    for node in nodes.iter() {
        match node {
            Node::Literal(text) => result.push_str(text),
            Node::Var(name) => {
                if let Some(value) = ctx.resolver.resolve(name) {
                    result.push_str(&value);
                }
            }
            _ => {
                // Style specs only contain literals and $vars;
                // Styled/Conditional nodes cannot appear in a valid style spec.
            }
        }
    }
    result
}

/// True if any variable reachable in `nodes` resolves to a non-empty value.
///
/// A resolver returning `Some("")` is treated as absent: an empty string
/// carries no text contribution and must not activate a conditional group.
fn any_var_nonempty(nodes: &[Node], ctx: &RenderContext<'_>) -> bool {
    nodes.iter().any(|node| match node {
        // Match render-time semantics: a value that is *only* control
        // characters is stripped to empty by [`sanitize_control_chars`], so it
        // must not activate a conditional group either (#425).
        Node::Var(name) => ctx
            .resolver
            .resolve(name)
            .is_some_and(|value| !sanitize_control_chars(&value).is_empty()),
        Node::Styled { children, .. } | Node::Conditional(children) => {
            any_var_nonempty(children, ctx)
        }
        Node::Literal(_) => false,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]

    use super::{MapResolver, RenderContext, Span, VariableResolver, render};
    use crate::template::style::{Attr, Color, Style};

    /// Test resolver that returns `Some("")` for every variable name.
    struct EmptyResolver;

    impl VariableResolver for EmptyResolver {
        fn resolve(&self, _name: &str) -> Option<String> {
            Some(String::new())
        }
    }

    fn text_of(spans: &[Span]) -> String {
        spans.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn substitutes_variables() {
        let resolver = MapResolver::from_pairs([("branch", "main")]);
        let ctx = RenderContext::new(&resolver);
        let spans = render("on $branch", &ctx).unwrap();
        assert_eq!(text_of(&spans), "on main");
    }

    #[test]
    fn missing_variable_renders_empty() {
        let resolver = MapResolver::default();
        let ctx = RenderContext::new(&resolver);
        let spans = render("on $branch", &ctx).unwrap();
        assert_eq!(text_of(&spans), "on ");
    }

    #[test]
    fn styled_group_applies_style() {
        let resolver = MapResolver::from_pairs([("branch", "main")]);
        let ctx = RenderContext::new(&resolver);
        let spans = render("[$branch](bold green)", &ctx).unwrap();
        assert_eq!(spans.len(), 1);
        let span = spans.first().unwrap();
        assert_eq!(span.text, "main");
        assert_eq!(
            span.style,
            Style {
                fg: Some(Color::Named("green".to_owned())),
                bg: None,
                attrs: vec![Attr::Bold]
            }
        );
    }

    #[test]
    fn conditional_hidden_when_all_empty() {
        let resolver = MapResolver::default();
        let ctx = RenderContext::new(&resolver);
        let spans = render("on ($branch) end", &ctx).unwrap();
        assert_eq!(text_of(&spans), "on  end");
    }

    #[test]
    fn conditional_shown_when_any_nonempty() {
        let resolver = MapResolver::from_pairs([("branch", "main")]);
        let ctx = RenderContext::new(&resolver);
        let spans = render("on ([$branch]) end", &ctx).unwrap();
        assert_eq!(text_of(&spans), "on main end");
    }

    #[test]
    fn whitespace_between_two_empty_groups_collapses() {
        // Issue #351 repro: the middle whitespace literal is a sibling of two
        // groups that both evaluate to zero spans, so it must collapse too.
        // The `(*)` all-literal conditional already collapses; only `branch`
        // is set, so the trailing cyan-styled space must not appear.
        let resolver = MapResolver::from_pairs([("branch", "main")]);
        let ctx = RenderContext::new(&resolver);
        let spans = render(
            "[$branch](bright-black) [[(*)](218) ($ahead_behind)](cyan)",
            &ctx,
        )
        .unwrap();
        assert_eq!(text_of(&spans), "main ");
        assert!(
            spans
                .iter()
                .all(|s| s.style.fg != Some(Color::Named("cyan".to_owned()))),
            "no span should retain the cyan style: {spans:?}"
        );
        assert!(
            spans
                .iter()
                .all(|s| s.style.fg != Some(Color::Ansi256(218))),
            "no span should retain the 218 style: {spans:?}"
        );
    }

    #[test]
    fn whitespace_not_collapsed_when_only_one_neighbor_is_empty_group() {
        // Control: the whitespace literal sits between an empty Conditional
        // (left) and a bare Var (right, not a group). Only one side is an
        // empty group, so the rule must NOT fire and the space must render.
        // `x` is unset (empty Conditional); `branch` is set so the right Var
        // produces text, isolating the "one side only" case.
        let resolver = MapResolver::from_pairs([("branch", "main")]);
        let ctx = RenderContext::new(&resolver);
        let spans = render("([$x]) $branch", &ctx).unwrap();
        assert_eq!(text_of(&spans), " main");
    }

    #[test]
    fn non_whitespace_literal_between_two_empty_groups_not_collapsed() {
        // Control: a non-whitespace literal ("-") sandwiched between two empty
        // groups is NEVER suppressed by the rule. Both `a` and `b` are unset,
        // so both Conditionals produce nothing, but the "-" must still render.
        let resolver = MapResolver::default();
        let ctx = RenderContext::new(&resolver);
        let spans = render("([$a])-([$b])", &ctx).unwrap();
        assert_eq!(text_of(&spans), "-");
    }

    #[test]
    fn style_spec_variable_indirection() {
        let resolver = MapResolver::from_pairs([("branch", "main"), ("style", "bold red")]);
        let ctx = RenderContext::new(&resolver);
        let spans = render("[$branch]($style)", &ctx).unwrap();
        let span = spans.first().unwrap();
        assert_eq!(span.style.fg, Some(Color::Named("red".to_owned())));
        assert_eq!(span.style.attrs, vec![Attr::Bold]);
    }

    #[test]
    fn unstyled_group_uses_default_style() {
        let resolver = MapResolver::from_pairs([("x", "hello")]);
        let ctx = RenderContext::new(&resolver);
        let spans = render("[$x]", &ctx).unwrap();
        assert_eq!(spans.first().unwrap().text, "hello");
        assert_eq!(spans.first().unwrap().style, Style::default());
    }

    #[test]
    fn resolves_palette_color() {
        use crate::template::style::Color;
        use std::collections::HashMap;

        let mut map = HashMap::new();
        map.insert("accent".to_owned(), Color::Rgb { r: 1, g: 2, b: 3 });
        let resolver = MapResolver::from_pairs([("x", "hi")]);
        let ctx = RenderContext::new(&resolver).with_palette(crate::template::Palette::new(map));
        let spans = render("[$x](fg:accent)", &ctx).unwrap();
        assert_eq!(
            spans.first().unwrap().style.fg,
            Some(Color::Rgb { r: 1, g: 2, b: 3 })
        );
    }

    #[test]
    fn resolves_prev_bg() {
        use crate::template::style::Color;
        let resolver = MapResolver::from_pairs([("x", "hi")]);
        let ctx = RenderContext::new(&resolver)
            .with_prev_colors(None, Some(Color::Named("blue".to_owned())));
        let spans = render("[$x](fg:prev_bg)", &ctx).unwrap();
        assert_eq!(
            spans.first().unwrap().style.fg,
            Some(Color::Named("blue".to_owned()))
        );
    }

    #[test]
    fn unknown_palette_name_errors() {
        let resolver = MapResolver::from_pairs([("x", "hi")]);
        let ctx = RenderContext::new(&resolver);
        let err = render("[$x](fg:nonsuch)", &ctx).unwrap_err();
        assert!(matches!(
            err,
            crate::template::TemplateError::UnknownColor { .. }
        ));
    }

    #[test]
    fn empty_string_variable_hides_conditional() {
        // A resolver that returns Some("") for every name must NOT activate
        // a conditional group — empty string is treated the same as absent.
        let resolver = EmptyResolver;
        let ctx = RenderContext::new(&resolver);
        let spans = render("on ([$x]) end", &ctx).unwrap();
        assert_eq!(text_of(&spans), "on  end");
    }

    #[test]
    fn map_resolver_accepts_owned_strings() {
        // from_pairs must work with runtime-owned Strings, not only &'static str.
        let resolver = MapResolver::from_pairs([(String::from("key"), String::from("val"))]);
        assert_eq!(resolver.resolve("key"), Some("val".to_owned()));
    }

    #[test]
    fn palette_overrides_standard_named_color() {
        use crate::template::style::Color;
        use std::collections::HashMap;

        // A palette that redefines `green` must win over the standard ANSI green.
        let mut map = HashMap::new();
        map.insert(
            "green".to_owned(),
            Color::Rgb {
                r: 163,
                g: 190,
                b: 140,
            },
        );
        let resolver = MapResolver::from_pairs([("x", "hi")]);
        let ctx = RenderContext::new(&resolver).with_palette(crate::template::Palette::new(map));
        let spans = render("[$x](fg:green)", &ctx).unwrap();
        assert_eq!(
            spans.first().unwrap().style.fg,
            Some(Color::Rgb {
                r: 163,
                g: 190,
                b: 140
            })
        );
    }

    #[test]
    fn palette_miss_falls_back_to_standard_named_color() {
        use crate::template::style::Color;
        use std::collections::HashMap;

        // Palette defines `accent` but not `green`; `green` keeps its standard meaning.
        let mut map = HashMap::new();
        map.insert("accent".to_owned(), Color::Rgb { r: 1, g: 2, b: 3 });
        let resolver = MapResolver::from_pairs([("x", "hi")]);
        let ctx = RenderContext::new(&resolver).with_palette(crate::template::Palette::new(map));
        let spans = render("[$x](fg:green)", &ctx).unwrap();
        assert_eq!(
            spans.first().unwrap().style.fg,
            Some(Color::Named("green".to_owned()))
        );
    }

    #[test]
    fn explicit_palette_color_still_errors_when_absent() {
        // `Color::Palette` (a bare unknown word) must still hard-error when unresolved,
        // even with palette precedence added for Color::Named.
        let resolver = MapResolver::from_pairs([("x", "hi")]);
        let ctx = RenderContext::new(&resolver);
        let err = render("[$x](fg:nonsuch)", &ctx).unwrap_err();
        assert!(matches!(
            err,
            crate::template::TemplateError::UnknownColor { .. }
        ));
    }

    #[test]
    fn control_chars_in_variable_value_are_stripped() {
        // A directory (or branch) name carrying an ESC/OSC sequence must not
        // reach the rendered span verbatim — that is terminal escape injection
        // (#425). ESC (0x1b), BEL (0x07), and DEL (0x7f) are all stripped;
        // surrounding printable data survives.
        let evil = "\u{1b}]0;pwned\u{7}evil\u{7f}dir";
        let resolver = MapResolver::from_pairs([("path", evil)]);
        let ctx = RenderContext::new(&resolver);
        let spans = render("$path", &ctx).unwrap();
        let rendered = text_of(&spans);
        assert!(
            !rendered.chars().any(char::is_control),
            "rendered span still contains control chars: {rendered:?}"
        );
        assert_eq!(rendered, "]0;pwnedevildir");
    }

    #[test]
    fn control_only_variable_value_does_not_activate_conditional() {
        // `($var)` renders only when the var is non-empty. A value that is
        // entirely control characters strips to empty, so the group stays empty
        // rather than emitting a bare styled separator (#425).
        let resolver = MapResolver::from_pairs([("path", "\u{1b}\u{1b}\u{7f}")]);
        let ctx = RenderContext::new(&resolver);
        let spans = render("($path)", &ctx).unwrap();
        assert!(spans.is_empty(), "expected no spans, got {spans:?}");
    }

    #[test]
    fn unstyled_group_inside_styled_does_not_inherit_style() {
        // [[$x]](bold red): the inner `[$x]` has no style spec → Style::default(),
        // regardless of the outer `(bold red)`. Each Styled node introduces its own
        // scope; children of an unstyled `[..]` receive the default, not the parent's.
        let resolver = MapResolver::from_pairs([("x", "hi")]);
        let ctx = RenderContext::new(&resolver);
        let spans = render("[[$x]](bold red)", &ctx).unwrap();
        assert_eq!(spans.len(), 1_usize);
        assert_eq!(spans.first().unwrap().text, "hi");
        assert_eq!(spans.first().unwrap().style, Style::default());
    }
}
