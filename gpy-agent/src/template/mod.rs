//! Starship-compatible prompt template engine.
//!
//! Parses 100% of Starship's prompt-template grammar (`$variable`, `[text](style)`
//! text groups, `(...)` conditional groups, escapes, palettes, and `prev_fg`/`prev_bg`
//! color transfer) and evaluates templates into format-agnostic [`Span`]s.
//!
//! This module is pure: it performs no I/O, spawns no processes, and knows nothing
//! about segments, IPC, or prompt layout. Callers supply a [`VariableResolver`], a
//! palette, and optional previous-segment colors.
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
pub use eval::{MapResolver, RenderContext, Span, VariableResolver, render};
pub(crate) use style::parse_color;
pub use style::{Attr, Color, Palette, Style, parse_style};
