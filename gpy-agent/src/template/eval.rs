//! Evaluator: AST + context -> styled spans.
//!
//! An unstyled `[text]` group renders with the default style, not the parent's.

use crate::template::Result;
use crate::template::TemplateError;
use crate::template::ast::Node;
use crate::template::parse::parse_cached;
use crate::template::style::{Color, Palette, Style, parse_style};
use std::borrow::Cow;
use std::collections::HashMap;

/// Resolves template variable names to their string values.
pub trait VariableResolver {
    /// Return the value for `name`, or `None` if unset/empty.
    fn resolve(&self, name: &str) -> Option<String>;

    /// Whether `name` resolves to a shell prompt-expansion token (e.g. clock `%D{…}`),
    /// emitted unescaped; all other values are escaped.
    fn is_prompt_token(&self, _name: &str) -> bool {
        false
    }
}

/// A simple in-memory resolver, primarily for tests and the importer.
#[derive(Debug, Default)]
pub struct MapResolver {
    values: HashMap<String, String>,
}

impl MapResolver {
    /// Build a resolver from `(name, value)` pairs. Empty values are treated as unset.
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

    /// Resolve a previous-segment color; anything unresolvable is the terminal
    /// default, so it never fails the current segment.
    fn resolve_prev(&self, stored: Option<&Color>) -> Color {
        match stored {
            Some(Color::PrevFg | Color::PrevBg) | None => Color::Named("default".to_owned()),
            Some(color) => self
                .resolve_color(color)
                .unwrap_or_else(|_| Color::Named("default".to_owned())),
        }
    }

    /// # Errors
    /// Returns [`TemplateError::UnknownColor`] for a palette name not in `self.palette`.
    fn resolve_color(&self, color: &Color) -> Result<Color> {
        match color {
            Color::Named(_) => Ok(self.palette.lookup(color).unwrap_or_else(|| color.clone())),
            // Self/cyclic references must error, not be dropped silently by an encoder.
            Color::Palette(name) => self
                .palette
                .lookup(color)
                .filter(|resolved| !matches!(resolved, Color::Palette(_)))
                .ok_or_else(|| TemplateError::UnknownColor { name: name.clone() }),
            Color::PrevFg => Ok(self.resolve_prev(self.prev_fg.as_ref())),
            Color::PrevBg => Ok(self.resolve_prev(self.prev_bg.as_ref())),
            Color::Rgb { .. } | Color::Ansi256(_) => Ok(color.clone()),
        }
    }

    /// # Errors
    /// Returns an error if any color in `style` cannot be resolved.
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

/// Whether a span's text is display data or a shell prompt-expansion token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpanKind {
    /// Literal text; encoders escape it.
    #[default]
    Text,
    /// A prompt-expansion token (e.g. `%D{%H:%M}`); encoders copy it unescaped.
    PromptToken,
}

/// A run of text sharing one resolved style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    /// The text content.
    pub text: String,
    /// The style applied to `text`.
    pub style: Style,
    /// Whether `text` is display data or a prompt-expansion token.
    pub kind: SpanKind,
}

/// Parse and evaluate `template` against `ctx`.
///
/// # Errors
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
/// Returns an error if style resolution fails.
fn eval_nodes(
    nodes: &[Node],
    inherited: &Style,
    ctx: &RenderContext<'_>,
    out: &mut Vec<Span>,
) -> Result<()> {
    // First pass buffers each child so a whitespace-only literal can collapse
    // when both neighboring groups are empty.
    let mut per_node: Vec<Vec<Span>> = Vec::with_capacity(nodes.len());
    for node in nodes {
        let mut buf: Vec<Span> = Vec::new();
        match node {
            Node::Literal(text) => {
                push_text(&mut buf, &sanitize_literal(text), inherited, SpanKind::Text);
            }
            Node::Var(name) => {
                if let Some(value) = ctx.resolver.resolve(name) {
                    let kind = if ctx.resolver.is_prompt_token(name) {
                        SpanKind::PromptToken
                    } else {
                        SpanKind::Text
                    };
                    push_text(&mut buf, &sanitize_control_chars(&value), inherited, kind);
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

    // Only empty group nodes (`Styled`/`Conditional`) count as neighbors.
    let is_empty_group = |node: &Node, buf: &[Span]| {
        matches!(node, Node::Styled { .. } | Node::Conditional(_)) && buf.is_empty()
    };

    let mut previous_empty_group = false;
    let mut pending = nodes.iter().zip(per_node).peekable();
    while let Some((node, buf)) = pending.next() {
        let is_whitespace_literal = matches!(
            node,
            Node::Literal(text) if !text.is_empty() && text.chars().all(char::is_whitespace)
        );
        let sandwiched = is_whitespace_literal
            && previous_empty_group
            && pending
                .peek()
                .is_some_and(|(next, next_buf)| is_empty_group(next, next_buf));
        previous_empty_group = is_empty_group(node, &buf);
        if sandwiched {
            continue;
        }
        out.extend(buf);
    }
    Ok(())
}

/// Strip every C0/C1 control character from a data-derived variable value.
///
/// Security: directory/branch names may contain ESC/OSC sequences that would
/// inject terminal escapes into the prompt. Stripped (not escaped) so prompt
/// width stays aligned. Style ANSI is produced by the encoders and never passes through here.
/// The CLI uses it for plugin manifest text it prints, for the same reason.
pub(crate) fn sanitize_control_chars(value: &str) -> Cow<'_, str> {
    strip_controls(value, |_| false)
}

/// Strip terminal controls from theme literal text, keeping layout characters.
///
/// Validation rejects these themes; this keeps a theme that bypassed it (hot
/// reload, direct render) from emitting ESC/BEL/OSC payloads.
fn sanitize_literal(text: &str) -> Cow<'_, str> {
    strip_controls(text, is_template_layout_char)
}

/// The only control character theme template text may carry: newline, for multi-line layouts.
const fn is_template_layout_char(ch: char) -> bool {
    ch == '\n'
}

fn strip_controls(text: &str, keep: fn(char) -> bool) -> Cow<'_, str> {
    let forbidden = |ch: char| ch.is_control() && !keep(ch);
    if text.chars().any(forbidden) {
        Cow::Owned(text.chars().filter(|&ch| !forbidden(ch)).collect())
    } else {
        Cow::Borrowed(text)
    }
}

/// Reject terminal control characters (anything but newline) in a template.
///
/// # Errors
/// Returns [`TemplateError::ControlCharacter`] for the first offending character.
pub fn check_template_controls(template: &str) -> Result<()> {
    match template
        .chars()
        .enumerate()
        .find(|&(_, ch)| ch.is_control() && !is_template_layout_char(ch))
    {
        Some((position, ch)) => Err(TemplateError::ControlCharacter {
            code: u32::from(ch),
            position,
        }),
        None => Ok(()),
    }
}

fn push_text(out: &mut Vec<Span>, text: &str, style: &Style, kind: SpanKind) {
    if text.is_empty() {
        return;
    }
    out.push(Span {
        text: text.to_owned(),
        style: style.clone(),
        kind,
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
                // Style specs contain only literals and $vars.
            }
        }
    }
    result
}

/// True if any variable reachable in `nodes` resolves to a non-empty value.
///
/// `Some("")` counts as absent.
fn any_var_nonempty(nodes: &[Node], ctx: &RenderContext<'_>) -> bool {
    nodes.iter().any(|node| match node {
        // Control-char-only values strip to empty at render time, so match that.
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

    use super::{
        MapResolver, RenderContext, Span, VariableResolver, check_template_controls, render,
    };
    use crate::template::style::{Attr, Color, Style};

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
        // Whitespace between two empty groups collapses.
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
        // Only one neighbor is an empty group, so the space renders.
        let resolver = MapResolver::from_pairs([("branch", "main")]);
        let ctx = RenderContext::new(&resolver);
        let spans = render("([$x]) $branch", &ctx).unwrap();
        assert_eq!(text_of(&spans), " main");
    }

    #[test]
    fn non_whitespace_literal_between_two_empty_groups_not_collapsed() {
        // Non-whitespace literals are never suppressed.
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
    fn prev_bg_resolves_through_palette() {
        use crate::template::style::Color;
        use std::collections::HashMap;
        let palette = crate::template::Palette::new(HashMap::from([(
            "black".to_owned(),
            Color::Rgb {
                r: 46,
                g: 52,
                b: 64,
            },
        )]));
        let resolver = MapResolver::from_pairs([("x", "hi")]);
        let ctx = RenderContext::new(&resolver)
            .with_palette(palette)
            .with_prev_colors(None, Some(Color::Named("black".to_owned())));
        let spans = render("[X](fg:prev_bg)", &ctx).unwrap();
        assert_eq!(
            spans.first().unwrap().style.fg,
            Some(Color::Rgb {
                r: 46,
                g: 52,
                b: 64
            })
        );
    }

    #[test]
    fn prev_fg_resolves_through_palette() {
        use crate::template::style::Color;
        use std::collections::HashMap;
        let palette = crate::template::Palette::new(HashMap::from([(
            "orange".to_owned(),
            Color::Ansi256(208),
        )]));
        let resolver = MapResolver::from_pairs([("x", "hi")]);
        let ctx = RenderContext::new(&resolver)
            .with_palette(palette)
            .with_prev_colors(Some(Color::Palette("orange".to_owned())), None);
        let spans = render("[X](fg:prev_fg)", &ctx).unwrap();
        assert_eq!(spans.first().unwrap().style.fg, Some(Color::Ansi256(208)));
    }

    #[test]
    fn prev_bg_palette_only_role_resolves() {
        use crate::template::style::Color;
        use std::collections::HashMap;
        let palette = crate::template::Palette::new(HashMap::from([(
            "orange".to_owned(),
            Color::Ansi256(208),
        )]));
        let resolver = MapResolver::from_pairs([("x", "hi")]);
        let ctx = RenderContext::new(&resolver)
            .with_palette(palette)
            .with_prev_colors(None, Some(Color::Palette("orange".to_owned())));
        let spans = render("[X](fg:prev_bg)", &ctx).unwrap();
        assert_eq!(spans.first().unwrap().style.fg, Some(Color::Ansi256(208)));
    }

    #[test]
    fn prev_bg_unresolvable_falls_back_to_default() {
        use crate::template::style::Color;
        let resolver = MapResolver::from_pairs([("x", "hi")]);
        let ctx = RenderContext::new(&resolver)
            .with_prev_colors(None, Some(Color::Palette("missing".to_owned())));
        let spans = render("[X](fg:prev_bg)", &ctx).unwrap();
        assert_eq!(
            spans.first().unwrap().style.fg,
            Some(Color::Named("default".to_owned()))
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
        let resolver = EmptyResolver;
        let ctx = RenderContext::new(&resolver);
        let spans = render("on ([$x]) end", &ctx).unwrap();
        assert_eq!(text_of(&spans), "on  end");
    }

    #[test]
    fn map_resolver_accepts_owned_strings() {
        let resolver = MapResolver::from_pairs([(String::from("key"), String::from("val"))]);
        assert_eq!(resolver.resolve("key"), Some("val".to_owned()));
    }

    #[test]
    fn palette_overrides_standard_named_color() {
        use crate::template::style::Color;
        use std::collections::HashMap;

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
        // ESC/OSC in data must not reach the span (escape injection).
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
        // Control-char-only value strips to empty, so the group stays empty.
        let resolver = MapResolver::from_pairs([("path", "\u{1b}\u{1b}\u{7f}")]);
        let ctx = RenderContext::new(&resolver);
        let spans = render("($path)", &ctx).unwrap();
        assert!(spans.is_empty(), "expected no spans, got {spans:?}");
    }

    #[test]
    fn control_chars_in_literal_text_are_stripped_but_newline_kept() {
        let resolver = MapResolver::from_pairs([("path", "~")]);
        let ctx = RenderContext::new(&resolver);
        let spans = render("\u{1b}]0;title\u{7}[$path\r\n](bold red)>", &ctx).unwrap();
        assert_eq!(text_of(&spans), "]0;title~\n>");
        let [_, styled, ..] = spans.as_slice() else {
            panic!("expected at least two spans, got {spans:?}");
        };
        assert_eq!(styled.style.fg, Some(Color::Named("red".to_owned())));
        assert_eq!(styled.style.attrs, vec![Attr::Bold]);
    }

    #[test]
    fn check_template_controls_allows_newline_and_reports_first_control() {
        assert_eq!(check_template_controls("$path\n$branch"), Ok(()));
        assert_eq!(
            check_template_controls("ab\u{1b}]0;x\u{7}$path"),
            Err(crate::template::TemplateError::ControlCharacter {
                code: 0x1b,
                position: 2,
            })
        );
        assert!(check_template_controls("[x](fg:red\u{9b})").is_err());
    }

    #[test]
    fn unstyled_group_inside_styled_does_not_inherit_style() {
        // Inner unstyled `[$x]` gets the default style, not the outer one.
        let resolver = MapResolver::from_pairs([("x", "hi")]);
        let ctx = RenderContext::new(&resolver);
        let spans = render("[[$x]](bold red)", &ctx).unwrap();
        assert_eq!(spans.len(), 1_usize);
        assert_eq!(spans.first().unwrap().text, "hi");
        assert_eq!(spans.first().unwrap().style, Style::default());
    }
}
