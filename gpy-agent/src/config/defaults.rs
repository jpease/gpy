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

/// Built-in git status glyphs for one [`GitIconSet`](crate::config::GitIconSet).
pub(crate) struct GitGlyphs {
    /// Ahead indicator.
    pub(crate) ahead: &'static str,
    /// Behind indicator.
    pub(crate) behind: &'static str,
    /// Staged files indicator.
    pub(crate) staged: &'static str,
    /// Unstaged files indicator.
    pub(crate) unstaged: &'static str,
    /// Untracked files indicator.
    pub(crate) untracked: &'static str,
    /// Conflicts indicator.
    pub(crate) conflicts: &'static str,
    /// Stash indicator.
    pub(crate) stash: &'static str,
    /// Detached-HEAD indicator.
    pub(crate) detached: &'static str,
    /// In-progress-operation indicator.
    pub(crate) in_progress: &'static str,
}

/// Git glyphs for `git.icon_set = "unicode"`.
pub(crate) const GIT_GLYPHS_UNICODE: GitGlyphs = GitGlyphs {
    ahead: "↑",
    behind: "↓",
    staged: "✚",
    unstaged: "✱",
    untracked: "?",
    conflicts: "✖",
    stash: "≡",
    detached: "➦",
    in_progress: "↻",
};

/// Git glyphs for `git.icon_set = "nerd_font"`: the Unicode set with a Nerd
/// Font stash, detached-HEAD and in-progress glyph.
pub(crate) const GIT_GLYPHS_NERD_FONT: GitGlyphs = GitGlyphs {
    stash: "\u{f187}",
    detached: "\u{f126}",
    in_progress: "\u{f021}",
    ..GIT_GLYPHS_UNICODE
};

/// Branch glyph drawn when `ui.show_icons` is on and HEAD is on a branch.
pub(crate) const BRANCH_GLYPH: &str = "\u{e0a0}";

/// Built-in Nerd Font language icons by canonical name; they need a patched font.
pub const DEFAULT_LANGUAGE_ICONS: &[(&str, &str)] = &[
    ("rust", "\u{f1617}"),
    ("python", "\u{f0320}"),
    ("node", "\u{f0399}"),
    ("go", "\u{f07d3}"),
    ("java", "\u{f0b37}"),
    ("ruby", "\u{f0d2d}"),
    ("swift", "\u{f06e5}"),
    ("elixir", "\u{e7cd}"),
    ("php", "\u{f031f}"),
    ("csharp", "\u{f031b}"),
    ("cpp", "\u{f0672}"),
    ("c", "\u{f0671}"),
    ("erlang", "\u{e7b1}"),
    ("fish", "\u{f023a}"),
];

#[cfg(test)]
mod tests {
    use super::DEFAULT_ENABLED_SEGMENTS;
    use std::path::Path;

    /// Each shell keeps a fallback list for when no theme export has loaded;
    /// `tests/fixtures/default_segments.txt` pins them all to the agent's default.
    #[test]
    fn default_segments_fixture_matches() {
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/default_segments.txt");
        let text = std::fs::read_to_string(&fixture)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", fixture.display()));
        let lines: Vec<&str> = text
            .lines()
            .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
            .collect();
        assert_eq!(lines, [DEFAULT_ENABLED_SEGMENTS.join(" ")]);
    }
}
