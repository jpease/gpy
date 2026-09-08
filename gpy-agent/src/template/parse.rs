//! Recursive-descent parser: template string -> AST.

use crate::cache::bounded::evict_to_capacity;
use crate::template::ast::Node;
use crate::template::{Result, TemplateError};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, PoisonError, RwLock};

/// Maximum supported group-nesting depth. Inputs deeper than this return
/// [`TemplateError::TooDeep`] instead of overflowing the stack.
const MAX_DEPTH: usize = 128_usize;

/// Maximum number of distinct template strings whose parsed ASTs are memoized.
///
/// The live prompt reuses a tiny working set of templates (one `format` per
/// segment plus a handful of style specs), so this cap is generous while still
/// bounding memory against a pathological config that cycles through many
/// distinct templates.
const AST_CACHE_CAPACITY: usize = 256_usize;

/// A memoized AST plus a monotonic insertion marker used for bounded eviction.
struct CachedAst {
    nodes: Arc<[Node]>,
    seq: u64,
}

/// Process-wide memoization table for [`parse_cached`], created on first use.
///
/// An `RwLock` (not a `Mutex`) is used because the hot path is overwhelmingly a
/// cache *hit*: the same segment templates render repeatedly across threads
/// (watcher, IPC blocking pool). A read lock lets those hits proceed
/// concurrently and only returns a cheap `Arc` clone; the exclusive write lock
/// is taken solely on the rare miss to insert a freshly parsed AST. Parsing
/// itself runs outside every lock (it is pure and touches no shared state), so
/// the lock is never held across the parse.
fn ast_cache() -> &'static RwLock<HashMap<String, CachedAst>> {
    static CACHE: OnceLock<RwLock<HashMap<String, CachedAst>>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Next insertion marker (least-recently-inserted is evicted first).
fn next_ast_seq() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1_u64, Ordering::Relaxed)
}

/// Parse a template string into a list of [`Node`]s.
///
/// # Errors
///
/// Returns [`TemplateError`] on unbalanced delimiters, invalid escapes, or
/// nesting that exceeds [`MAX_DEPTH`].
pub fn parse(input: &str) -> Result<Vec<Node>> {
    let chars: Vec<char> = input.chars().collect();
    let mut pos = 0_usize;
    parse_nodes(&chars, &mut pos, None, 0_usize)
}

/// Parse `input` into a shared AST, memoizing the result across calls.
///
/// Distinct strings key distinct entries, so a theme/config reload that changes
/// a template is self-invalidating (the new string is a fresh key). A parse
/// error is propagated and never cached, so a malformed template can never be
/// served as a silently-empty AST from the cache.
///
/// # Errors
///
/// Propagates any [`TemplateError`] from [`parse`].
#[expect(
    clippy::significant_drop_tightening,
    reason = "the write guard must span both the `insert` and the subsequent `evict_to_capacity` call -- releasing and re-acquiring the lock between them would let another thread's concurrent insert push the table past AST_CACHE_CAPACITY before eviction runs"
)]
pub fn parse_cached(input: &str) -> Result<Arc<[Node]>> {
    // Fast path: a shared read lock, returning a cheap `Arc` clone on a hit.
    {
        let guard = ast_cache().read().unwrap_or_else(PoisonError::into_inner);
        if let Some(entry) = guard.get(input) {
            return Ok(Arc::clone(&entry.nodes));
        }
    }

    // Miss: parse outside any lock (pure, no shared state). Propagate errors
    // before touching the cache so a parse failure is never memoized.
    let nodes: Arc<[Node]> = parse(input)?.into();

    let mut guard = ast_cache().write().unwrap_or_else(PoisonError::into_inner);
    guard.insert(
        input.to_owned(),
        CachedAst {
            nodes: Arc::clone(&nodes),
            seq: next_ast_seq(),
        },
    );
    // Bound the table: drop the least-recently-inserted entries past capacity.
    evict_to_capacity(&mut guard, AST_CACHE_CAPACITY, |entry| entry.seq);

    Ok(nodes)
}

/// Return `Ok(())` when `depth` is within the allowed limit, or
/// [`TemplateError::TooDeep`] if the caller is about to exceed it.
///
/// # Errors
///
/// Returns [`TemplateError::TooDeep`] when `depth >= MAX_DEPTH`.
const fn check_depth(depth: usize) -> Result<()> {
    if depth >= MAX_DEPTH {
        return Err(TemplateError::TooDeep { limit: MAX_DEPTH });
    }
    Ok(())
}

/// Parse nodes until `terminator` (or end of input when `None`).
///
/// `depth` tracks how many group levels deep this call is; the public
/// [`parse`] entry-point passes `0`.
///
/// # Errors
///
/// Returns [`TemplateError`] on unbalanced delimiters, invalid escapes, or
/// nesting that exceeds [`MAX_DEPTH`].
fn parse_nodes(
    chars: &[char],
    pos: &mut usize,
    terminator: Option<char>,
    depth: usize,
) -> Result<Vec<Node>> {
    let mut nodes: Vec<Node> = Vec::new();
    let mut literal = String::new();

    while let Some(&ch) = chars.get(*pos) {
        if Some(ch) == terminator {
            break;
        }
        match ch {
            '\\' => {
                let escaped = chars
                    .get(pos.saturating_add(1_usize))
                    .ok_or(TemplateError::BadEscape { position: *pos })?;
                if matches!(escaped, '$' | '[' | ']' | '(' | ')' | '\\') {
                    literal.push(*escaped);
                    *pos = pos.saturating_add(2_usize);
                } else {
                    return Err(TemplateError::BadEscape { position: *pos });
                }
            }
            '$' => {
                flush_literal(&mut nodes, &mut literal);
                nodes.push(parse_variable(chars, pos)?);
            }
            '[' => {
                flush_literal(&mut nodes, &mut literal);
                check_depth(depth)?;
                nodes.push(parse_styled_group(
                    chars,
                    pos,
                    depth.saturating_add(1_usize),
                )?);
            }
            '(' => {
                flush_literal(&mut nodes, &mut literal);
                check_depth(depth)?;
                *pos = pos.saturating_add(1_usize); // consume '('
                let children = parse_nodes(chars, pos, Some(')'), depth.saturating_add(1_usize))?;
                if chars.get(*pos) != Some(&')') {
                    return Err(TemplateError::Unbalanced {
                        delimiter: '(',
                        position: *pos,
                    });
                }
                *pos = pos.saturating_add(1_usize); // consume ')'
                nodes.push(Node::Conditional(children));
            }
            _ => {
                literal.push(ch);
                *pos = pos.saturating_add(1_usize);
            }
        }
    }
    flush_literal(&mut nodes, &mut literal);
    Ok(nodes)
}

/// Parse a `[children](style)` styled group. `*pos` points at the `[`.
///
/// `depth` is the nesting level of the children (already incremented by the
/// caller before descending here).
///
/// # Errors
///
/// Returns [`TemplateError`] if the closing `]` is missing or the style spec is malformed.
fn parse_styled_group(chars: &[char], pos: &mut usize, depth: usize) -> Result<Node> {
    *pos = pos.saturating_add(1_usize); // consume '['
    let children = parse_nodes(chars, pos, Some(']'), depth)?;
    if chars.get(*pos) != Some(&']') {
        return Err(TemplateError::Unbalanced {
            delimiter: '[',
            position: *pos,
        });
    }
    *pos = pos.saturating_add(1_usize); // consume ']'
    // Style spec is optional: `[text]` with no `(style)` uses empty style.
    let style_spec = if chars.get(*pos) == Some(&'(') {
        *pos = pos.saturating_add(1_usize); // consume '('
        parse_style_spec(chars, pos)?
    } else {
        String::new()
    };
    Ok(Node::Styled {
        style_spec,
        children,
    })
}

/// Read the raw style text between style parens. `*pos` points just past `(`.
///
/// # Errors
///
/// Returns [`TemplateError::Unbalanced`] if the closing `)` is not found.
fn parse_style_spec(chars: &[char], pos: &mut usize) -> Result<String> {
    let open = *pos;
    let mut spec = String::new();
    while let Some(&ch) = chars.get(*pos) {
        match ch {
            ')' => {
                *pos = pos.saturating_add(1_usize);
                return Ok(spec);
            }
            '\\' => {
                let escaped = chars
                    .get(pos.saturating_add(1_usize))
                    .ok_or(TemplateError::BadEscape { position: *pos })?;
                spec.push(*escaped);
                *pos = pos.saturating_add(2_usize);
            }
            _ => {
                spec.push(ch);
                *pos = pos.saturating_add(1_usize);
            }
        }
    }
    Err(TemplateError::Unbalanced {
        delimiter: '(',
        position: open,
    })
}

/// Flush accumulated literal text into the node list, then clear the buffer.
fn flush_literal(nodes: &mut Vec<Node>, literal: &mut String) {
    if !literal.is_empty() {
        nodes.push(Node::Literal(std::mem::take(literal)));
    }
}

/// Parse a `$name` or `${name}` variable. `*pos` points at the `$`.
///
/// # Errors
///
/// Returns [`TemplateError::Unbalanced`] if a `${...}` is not closed.
fn parse_variable(chars: &[char], pos: &mut usize) -> Result<Node> {
    *pos = pos.saturating_add(1_usize); // consume '$'
    let mut name = String::new();
    if chars.get(*pos) == Some(&'{') {
        *pos = pos.saturating_add(1_usize);
        while let Some(&ch) = chars.get(*pos) {
            if ch == '}' {
                *pos = pos.saturating_add(1_usize);
                return Ok(Node::Var(name));
            }
            name.push(ch);
            *pos = pos.saturating_add(1_usize);
        }
        return Err(TemplateError::Unbalanced {
            delimiter: '{',
            position: *pos,
        });
    }
    while let Some(&ch) = chars.get(*pos) {
        if ch.is_alphanumeric() || ch == '_' || ch == '.' {
            name.push(ch);
            *pos = pos.saturating_add(1_usize);
        } else {
            break;
        }
    }
    Ok(Node::Var(name))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]

    use super::parse;
    use crate::template::TemplateError;
    use crate::template::ast::Node;

    #[test]
    fn parses_plain_literal() {
        let nodes = parse("hello world").unwrap();
        assert_eq!(nodes, vec![Node::Literal("hello world".to_owned())]);
    }

    #[test]
    fn parses_simple_variable() {
        let nodes = parse("$branch").unwrap();
        assert_eq!(nodes, vec![Node::Var("branch".to_owned())]);
    }

    #[test]
    fn parses_braced_variable() {
        let nodes = parse("${git_branch}").unwrap();
        assert_eq!(nodes, vec![Node::Var("git_branch".to_owned())]);
    }

    #[test]
    fn parses_literal_then_variable() {
        let nodes = parse("on $branch").unwrap();
        assert_eq!(
            nodes,
            vec![
                Node::Literal("on ".to_owned()),
                Node::Var("branch".to_owned()),
            ]
        );
    }

    #[test]
    fn unescapes_special_characters() {
        let nodes = parse(r"\$ \[ \] \( \) \\").unwrap();
        assert_eq!(nodes, vec![Node::Literal(r"$ [ ] ( ) \".to_owned())]);
    }

    #[test]
    fn rejects_dangling_escape() {
        let err = parse(r"abc\").unwrap_err();
        assert_eq!(err, TemplateError::BadEscape { position: 3 });
    }

    #[test]
    fn rejects_invalid_escape() {
        let err = parse(r"\x").unwrap_err();
        assert_eq!(err, TemplateError::BadEscape { position: 0 });
    }

    #[test]
    fn rejects_unterminated_brace() {
        let err = parse("${unclosed").unwrap_err();
        assert_eq!(
            err,
            TemplateError::Unbalanced {
                delimiter: '{',
                position: 10,
            }
        );
    }

    #[test]
    fn parses_styled_group() {
        let nodes = parse("[main](bold green)").unwrap();
        assert_eq!(
            nodes,
            vec![Node::Styled {
                style_spec: "bold green".to_owned(),
                children: vec![Node::Literal("main".to_owned())],
            }]
        );
    }

    #[test]
    fn parses_styled_group_with_variable_child() {
        let nodes = parse("[$branch]($style)").unwrap();
        assert_eq!(
            nodes,
            vec![Node::Styled {
                style_spec: "$style".to_owned(),
                children: vec![Node::Var("branch".to_owned())],
            }]
        );
    }

    #[test]
    fn parses_conditional_group() {
        let nodes = parse("($branch)").unwrap();
        assert_eq!(
            nodes,
            vec![Node::Conditional(vec![Node::Var("branch".to_owned())])]
        );
    }

    #[test]
    fn parses_nested_groups() {
        let nodes = parse("([$a](red))").unwrap();
        assert_eq!(
            nodes,
            vec![Node::Conditional(vec![Node::Styled {
                style_spec: "red".to_owned(),
                children: vec![Node::Var("a".to_owned())],
            }])]
        );
    }

    #[test]
    fn rejects_unclosed_bracket() {
        let err = parse("[main").unwrap_err();
        assert_eq!(
            err,
            TemplateError::Unbalanced {
                delimiter: '[',
                position: 5
            }
        );
    }

    #[test]
    fn deeply_nested_input_errors_without_panic() {
        // 100 000 unclosed `[` would overflow the stack without the depth cap.
        // With MAX_DEPTH = 128 the parser returns Err instead of aborting.
        let deep = "[".repeat(100_000_usize);
        assert!(matches!(parse(&deep), Err(TemplateError::TooDeep { .. })));
    }

    #[test]
    fn nesting_within_limit_ok() {
        // 10 levels of nesting is well within the 128-level cap.
        let template = "[[[[[[[[[[text](red)](red)](red)](red)](red)](red)](red)](red)](red)](red)";
        assert!(parse(template).is_ok());
    }

    #[test]
    fn nesting_at_127_brackets_ok() {
        // 127 brackets reach check_depth at depths 0..126 — all < MAX_DEPTH 128.
        let template = format!("{}x{}", "[".repeat(127_usize), "]".repeat(127_usize));
        assert!(parse(&template).is_ok());
    }

    #[test]
    fn nesting_at_128_brackets_exact_boundary_ok() {
        // 128 brackets reach check_depth at depths 0..127 — the last check is
        // depth 127 < 128, so this is still within the allowed limit.
        let template = format!("{}x{}", "[".repeat(128_usize), "]".repeat(128_usize));
        assert!(parse(&template).is_ok());
    }

    #[test]
    fn nesting_at_129_brackets_too_deep() {
        // 129 brackets trigger check_depth at depth 128 >= MAX_DEPTH 128 → TooDeep.
        let template = "[".repeat(129_usize);
        let err = parse(&template).unwrap_err();
        assert_eq!(err, TemplateError::TooDeep { limit: 128_usize });
    }

    #[test]
    fn conditional_nesting_too_deep() {
        // The `(` conditional path shares check_depth; verify it fires there too.
        let template = "(".repeat(129_usize);
        let err = parse(&template).unwrap_err();
        assert_eq!(err, TemplateError::TooDeep { limit: 128_usize });
    }

    #[test]
    fn rejects_unbalanced_style_paren() {
        // `[x](red` — the `(` opening the style spec is never closed.
        let err = parse("[x](red").unwrap_err();
        assert_eq!(
            err,
            TemplateError::Unbalanced {
                delimiter: '(',
                position: 4_usize,
            }
        );
    }

    #[test]
    fn rejects_unbalanced_conditional() {
        // `(abc` — the `(` opening the conditional is never closed.
        let err = parse("(abc").unwrap_err();
        assert_eq!(
            err,
            TemplateError::Unbalanced {
                delimiter: '(',
                position: 4_usize,
            }
        );
    }

    #[test]
    fn escaped_paren_in_style_spec_is_accepted() {
        // `[x](a\)b)` — the `\)` inside the style spec is an escaped `)`, not
        // the closing delimiter; the spec is "a)b".
        let nodes = parse(r"[x](a\)b)").unwrap();
        assert_eq!(
            nodes,
            vec![Node::Styled {
                style_spec: "a)b".to_owned(),
                children: vec![Node::Literal("x".to_owned())],
            }]
        );
    }

    #[test]
    fn lone_dollar_sign_produces_empty_var() {
        // A bare `$` with no following name or `{` yields `Var("")`.
        let nodes = parse("$").unwrap();
        assert_eq!(nodes, vec![Node::Var(String::new())]);
    }

    #[test]
    fn empty_braced_var_produces_empty_var() {
        // `${}` — braced form with an empty name yields `Var("")`.
        let nodes = parse("${}").unwrap();
        assert_eq!(nodes, vec![Node::Var(String::new())]);
    }
}
