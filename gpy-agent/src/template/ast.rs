//! Abstract syntax tree for parsed templates.

/// A parsed template node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// Literal text emitted verbatim.
    Literal(String),
    /// A `$name` / `${name}` variable reference.
    Var(String),
    /// A `[children](style_spec)` styled text group.
    Styled {
        /// Raw style text between the parens (variables not yet substituted).
        style_spec: String,
        /// Inner nodes the style applies to.
        children: Vec<Self>,
    },
    /// A `(children)` conditional group: rendered only if it contains at least
    /// one non-empty variable.
    Conditional(Vec<Self>),
}
