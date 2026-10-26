//! Default values shared by the `Default` impls, the shell exports, and the embedded presets.

/// Parse a hardcoded default literal (a color, an icon, ...).
///
/// # Panics
/// Panics if `value` does not parse; callers pass only known-good constants,
/// and the `Default` tests construct every one of them.
#[expect(
    clippy::expect_used,
    reason = "argument is a compile-time-constant literal; parsing it is infallible in practice"
)]
pub(crate) fn literal<T>(value: &'static str) -> T
where
    T: std::str::FromStr,
    T::Err: std::fmt::Debug,
{
    value.parse().expect("hardcoded default literal must parse")
}

/// Segments enabled by default, in prompt order.
pub(crate) const DEFAULT_ENABLED_SEGMENTS: [&str; 5] =
    ["clock", "duration", "language", "directory", "git"];

/// Prompt icon color exported when the theme sets no `ui.prompt_color`.
pub const DEFAULT_UI_PROMPT_COLOR: &str = "green";

/// Root prompt icon color exported when the theme sets no `ui.root_prompt_color`.
pub const DEFAULT_UI_ROOT_PROMPT_COLOR: &str = "red";

/// Default prompt open delimiter
pub const DEFAULT_PROMPT_OPEN: &str = " ";

/// Default prompt close delimiter
pub const DEFAULT_PROMPT_CLOSE: &str = "";

/// Default segment open delimiter
pub const DEFAULT_SEGMENT_OPEN: &str = " ";

/// Default segment close delimiter
pub const DEFAULT_SEGMENT_CLOSE: &str = " ";

/// Embedded content of the default theme
pub const DEFAULT_THEME_CONTENT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../config/themes/default.toml"
));

/// Embedded content of the text theme
pub const TEXT_THEME_CONTENT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../config/themes/text.toml"
));

/// Embedded content of the Starship preset theme
pub const STARSHIP_THEME_CONTENT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../config/themes/starship.toml"
));

/// Embedded default palette TOML (identity ANSI map).
pub const DEFAULT_PALETTE_CONTENT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../config/palettes/default.toml"
));

/// Embedded `starship` palette TOML (pairs with the starship theme).
pub const STARSHIP_PALETTE_CONTENT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../config/palettes/starship.toml"
));
