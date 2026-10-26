//! Starship-compatible prompt template engine.
//!
//! Parses Starship's full prompt-template grammar and evaluates it into
//! format-agnostic [`Span`]s. Pure: no I/O.
//!
//! # Example
//!
//! ```
//! use gpy_agent::template::{render, MapResolver, RenderContext};
//!
//! let resolver = MapResolver::from_pairs([("branch", "main")]);
//! let ctx = RenderContext::new(&resolver);
//! let spans = render("on [$branch](bold purple)", &ctx).expect("valid template");
//! assert_eq!(spans.iter().map(|s| s.text.as_str()).collect::<String>(), "on main");
//! ```

pub(crate) mod ast;
mod error;
mod eval;
pub(crate) mod parse;
mod style;

pub use error::{Result, TemplateError};
pub(crate) use eval::sanitize_control_chars;
pub use eval::{
    MapResolver, RenderContext, Span, SpanKind, VariableResolver, check_template_controls, render,
};
pub use style::canonical_ansi_name;
#[cfg(test)]
pub(crate) use style::{ANSI_BASE_WORDS, BRIGHT_PREFIXES, is_named_color};
pub use style::{Attr, Color, Palette, Style, parse_style};
pub(crate) use style::{alias_owner, config_color_name, parse_color};
