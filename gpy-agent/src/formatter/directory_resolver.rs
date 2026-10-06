//! Maps a directory path to Starship-exact template variable names.
//!
//! Variable names: `path`, `read_only`, `style`.
//! Compatible with Starship's `directory` module.

use crate::config::Config;
use crate::config::DirectorySettings;
use crate::config::types::DirectoryDisplay;
use crate::formatter::SegmentPosition;
use crate::formatter::separator::{Glyphs, SeparatorStyle, resolve_separator};
use crate::template::VariableResolver;
use crate::theme::ThemeConfig;
use std::path::{Path, PathBuf};

/// Resolves Starship-exact directory variable names.
///
/// All I/O (`$HOME` env read, `cwd` canonicalization, git repo-root walk) is
/// gathered once in [`DirectoryResolver::new`] and cached on the struct.
/// `resolve()` and its helpers (`display_path`, `repo_anchored_for`,
/// `contract_home`) are pure over those pre-gathered fields — this matters
/// because `resolve("path")` can be called more than once per render (see
/// `template::eval::any_var_nonempty`'s emptiness pre-check for a `($path)`
/// conditional group), and re-running the I/O on every call would waste a
/// `canonicalize` + repo-root filesystem walk per extra call.
pub struct DirectoryResolver<'a> {
    /// Lexically normalized `cwd` (see [`normalize_cwd`]); drives every display mode.
    cwd: String,
    read_only: bool,
    config: &'a Config,
    theme: &'a ThemeConfig,
    pos: SegmentPosition,
    /// `$HOME`, read once at construction.
    home: Option<String>,
    /// Canonicalized `cwd`, computed once at construction (falls back to
    /// `cwd` unchanged if canonicalization fails). Only consulted when
    /// `repo_root` is `Some`; computing it unconditionally trades the old
    /// "sometimes 0, sometimes 1 canonicalize calls" shape for "always
    /// exactly 1" — a minor, expected cost-shape change, not a behavior
    /// change (see issue #603).
    canonical_cwd: PathBuf,
    /// Enclosing git repo root, computed once at construction.
    repo_root: Option<PathBuf>,
}

impl<'a> DirectoryResolver<'a> {
    /// Build a resolver for one directory segment render.
    ///
    /// Gathers all I/O for the render up front: reads `$HOME`, canonicalizes
    /// `cwd`, and walks for the enclosing git repo root. No longer `const`
    /// now that it performs real I/O.
    #[must_use]
    pub fn new(
        cwd: &'a str,
        read_only: bool,
        config: &'a Config,
        theme: &'a ThemeConfig,
        pos: SegmentPosition,
    ) -> Self {
        let home = std::env::var("HOME").ok();
        let canonical_cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| PathBuf::from(cwd));
        let repo_root = enclosing_anchor_root(cwd, config, home.as_deref());
        Self {
            cwd: normalize_cwd(cwd),
            read_only,
            config,
            theme,
            pos,
            home,
            canonical_cwd,
            repo_root,
        }
    }

    /// Compute the display path: contract $HOME → ~, apply display mode, truncate.
    fn display_path(&self) -> String {
        const ELLIPSIS: &str = "...";
        const ELLIPSIS_LEN: usize = 3;

        // 1. Contract $HOME prefix to ~
        let home_contracted = contract_home(&self.cwd, self.home.as_deref());

        // 2. Apply display mode (repo anchoring takes precedence when enabled).
        let base = repo_anchored_for(
            &self.canonical_cwd,
            self.repo_root.as_deref(),
            &self.config.ui.directory,
        )
        .unwrap_or_else(|| match self.config.ui.directory.display {
            DirectoryDisplay::Basename => {
                if home_contracted == "/" {
                    "/".to_owned()
                } else {
                    home_contracted
                        .rsplit('/')
                        .next()
                        .filter(|s| !s.is_empty())
                        .map_or_else(|| "/".to_owned(), str::to_owned)
                }
            }
            DirectoryDisplay::Truncated => truncate_to_components(
                &home_contracted,
                self.config.ui.directory.truncation_length.get(),
                self.config.ui.directory.truncation_symbol.as_str(),
            ),
            DirectoryDisplay::Abbreviated => abbreviate_path(&home_contracted),
            DirectoryDisplay::Full => home_contracted,
        });

        // 3. Apply max_length truncation (tail-style: keep last chars, prefix "...")
        let max = self.config.ui.directory.max_length.get();
        let char_count = base.chars().count();
        if char_count <= max {
            return base;
        }

        // A cap at or below the ellipsis width (`ELLIPSIS_LEN` = 3) can't fit
        // both the "..." marker and any real path content without either
        // overflowing the cap or spending the whole budget on dots that carry
        // no path information. Below that threshold, drop the ellipsis
        // entirely and show the last `max` characters of the path verbatim —
        // see `MaxPathLength`'s doc comment and the `[ui.directory]` section
        // of docs/user/configuration-reference.md for the documented policy.
        if max <= ELLIPSIS_LEN {
            let skip = char_count.saturating_sub(max);
            return base.chars().skip(skip).collect();
        }

        let tail_len = max.saturating_sub(ELLIPSIS_LEN);
        let skip = char_count.saturating_sub(tail_len);
        let tail: String = base.chars().skip(skip).collect();
        format!("{ELLIPSIS}{tail}")
    }

    /// Style string: `fg:<text_color> bg:<bg_color>`.
    fn style(&self) -> String {
        let fg = self.theme.segments.directory.text_color.as_str();
        let bg = self.theme.segments.directory.bg_color.as_str();
        format!("fg:{fg} bg:{bg}")
    }
}

/// Lexically normalize a cwd without touching the filesystem.
///
/// Drops trailing `/` and `.` components. `..` is kept (resolving it lexically
/// would change meaning across symlinks). A cwd that is only `.` is returned
/// unchanged.
fn normalize_cwd(cwd: &str) -> String {
    let normalized: PathBuf = Path::new(cwd)
        .components()
        .filter(|c| !matches!(c, std::path::Component::CurDir))
        .collect();
    if normalized.as_os_str().is_empty() {
        cwd.to_owned()
    } else {
        normalized.to_string_lossy().into_owned()
    }
}

/// Enclosing git repo root for `cwd`, minus a root that is `$HOME`.
///
/// `$HOME` and the root are canonicalized only when `truncate_to_repo` is on
/// and a root was found, keeping the #603 I/O budget.
fn enclosing_anchor_root(cwd: &str, config: &Config, home: Option<&str>) -> Option<PathBuf> {
    let found_root = crate::git::find_repo_root(Path::new(cwd));
    let canonical_home = found_root
        .as_ref()
        .filter(|_| config.ui.directory.truncate_to_repo)
        .and(home)
        .and_then(|h| std::fs::canonicalize(h).ok());
    anchor_root(found_root, canonical_home.as_deref())
}

/// Drop `repo_root` when it is the home directory (dotfiles repo): Starship
/// does not anchor there, so the normal display mode (`~/…`) applies.
///
/// `canonical_home` must already be canonical; `repo_root` is canonicalized
/// here (only reached when a root was found and anchoring is on).
fn anchor_root(repo_root: Option<PathBuf>, canonical_home: Option<&Path>) -> Option<PathBuf> {
    let root = repo_root?;
    let is_home = canonical_home.is_some_and(|home| {
        root == home || std::fs::canonicalize(&root).is_ok_and(|canonical| canonical == home)
    });
    (!is_home).then_some(root)
}

/// Contract a `$HOME` prefix on `cwd` to `~`.
///
/// Pure: takes `home` as a parameter instead of reading `std::env::var("HOME")`
/// internally, so it's testable with an explicit/injected home value without
/// touching the real environment. `None` (or an empty string) leaves `cwd`
/// unchanged, matching the previous inline `std::env::var("HOME")` no-op path.
fn contract_home(cwd: &str, home: Option<&str>) -> String {
    match home {
        Some(home_value) if !home_value.is_empty() && cwd.starts_with(home_value) => {
            if cwd.len() == home_value.len() {
                "~".to_owned()
            } else if cwd.as_bytes().get(home_value.len()) == Some(&b'/') {
                format!("~{}", &cwd[home_value.len()..])
            } else {
                cwd.to_owned()
            }
        }
        _ => cwd.to_owned(),
    }
}

/// Repo-anchored display path when `truncate_to_repo` is on and `repo_root`
/// (pre-gathered by `DirectoryResolver::new`) is `Some`; `None` otherwise
/// (caller falls back to the normal path).
///
/// Pure: does no I/O itself — `canonical_cwd` and `repo_root` are gathered
/// once at construction (see `DirectoryResolver`'s doc comment) rather than
/// being re-derived on every call.
fn repo_anchored_for(
    canonical_cwd: &Path,
    repo_root: Option<&Path>,
    dir: &DirectorySettings,
) -> Option<String> {
    if !dir.truncate_to_repo {
        return None;
    }
    let display = dir.display;
    if !matches!(
        display,
        DirectoryDisplay::Truncated | DirectoryDisplay::Full
    ) {
        return None;
    }
    let root = repo_root?;
    repo_anchored_path(
        canonical_cwd,
        root,
        display,
        dir.truncation_length.get(),
        dir.truncation_symbol.as_str(),
    )
}

/// Truncate a home-contracted path to its last `keep` components.
///
/// A component is a non-empty `/`-separated segment; a leading `~` home marker
/// is not counted. When the component count is `<= keep` the path is returned
/// unchanged with no symbol. Otherwise the last `keep` components are joined by
/// `/` and prefixed with `symbol` (the `~`/root prefix is dropped). See the
/// directory-truncation design doc for the full semantics table.
fn truncate_to_components(path: &str, keep: usize, symbol: &str) -> String {
    let mut segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.first() == Some(&"~") {
        segments.remove(0);
    }

    if segments.len() <= keep {
        return path.to_owned();
    }

    let skip = segments.len().saturating_sub(keep);
    let tail = segments
        .into_iter()
        .skip(skip)
        .collect::<Vec<_>>()
        .join("/");
    format!("{symbol}{tail}")
}

/// Render `cwd` anchored at its enclosing git repo root.
///
/// `repo_root`'s basename `R` becomes the leading component and the path above
/// it (and any `~`/root prefix) is dropped. For `Full` the whole in-repo path is
/// returned uncapped; for `Truncated` the trailing components are capped at
/// `keep` while `R` stays pinned (see the truncate-to-repo design doc). Returns
/// `None` when `repo_root` has no basename, `cwd` is not under `repo_root`, or
/// `display` is not `Truncated`/`Full` — the caller then falls back.
fn repo_anchored_path(
    cwd: &Path,
    repo_root: &Path,
    display: DirectoryDisplay,
    keep: usize,
    symbol: &str,
) -> Option<String> {
    if !matches!(
        display,
        DirectoryDisplay::Truncated | DirectoryDisplay::Full
    ) {
        return None;
    }
    let root_name = repo_root.file_name()?.to_str()?;
    let rel = cwd.strip_prefix(repo_root).ok()?;
    let sub: Vec<&str> = rel
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .filter(|s| !s.is_empty())
        .collect();

    // Full: whole in-repo path, no cap, no symbol.
    if matches!(display, DirectoryDisplay::Full) {
        return Some(join_anchored(root_name, &sub));
    }

    // Truncated: m = sub.len() + 1 (including the repo root component).
    let m = sub.len().saturating_add(1);
    if m <= keep {
        return Some(join_anchored(root_name, &sub));
    }
    let tail_count = keep.saturating_sub(1);
    if tail_count == 0 {
        return Some(root_name.to_owned());
    }
    let tail_start = sub.len().saturating_sub(tail_count);
    let tail = sub.get(tail_start..).unwrap_or(&[]).join("/");
    Some(format!("{root_name}/{symbol}{tail}"))
}

/// Shorten each non-final path component to its first character (hidden
/// components keep the leading `.` plus one more).
///
/// The shells no longer abbreviate paths themselves (the Fish helper that
/// mirrored this was dead code and removed in #644); this is the one
/// implementation, asserted end to end by
/// `tests/fish/e2e_prompt_content.test.fish`.
///
/// - `~/alpha/beta/project` → `~/a/b/project`
/// - `~/.config/fish` → `~/.c/fish`
/// - `/usr/local/bin` → `/u/l/bin`
/// - `~` / `/` → unchanged
fn abbreviate_path(home_contracted: &str) -> String {
    if home_contracted == "/" || home_contracted == "~" {
        return home_contracted.to_owned();
    }

    // Split off a leading `~/` or `/`, keeping the prefix to re-prepend later.
    let (prefix, rest) = home_contracted.strip_prefix("~/").map_or_else(
        || {
            home_contracted
                .strip_prefix('/')
                .map_or(("", home_contracted), |tail| ("/", tail))
        },
        |tail| ("~/", tail),
    );

    let parts: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return home_contracted.to_owned();
    }

    let last = parts.len().saturating_sub(1);
    let abbreviated: Vec<String> = parts
        .iter()
        .enumerate()
        .map(|(i, part)| {
            if i == last {
                (*part).to_owned()
            } else {
                abbreviate_component(part).to_owned()
            }
        })
        .collect();

    format!("{prefix}{}", abbreviated.join("/"))
}

/// Shorten one path component: its first character cluster, plus the next
/// cluster when the component is a hidden name (`.config` → `.c`).
/// `.` and `..` are returned whole.
///
/// Clusters are approximated with `char` rules (no segmentation dependency):
/// a regional-indicator pair (flag), trailing combining marks / variation
/// selectors / emoji modifiers / tag characters, and ZWJ-joined sequences stay
/// together.
fn abbreviate_component(part: &str) -> &str {
    let first_end = cluster_end(part, 0);
    let end = if part.starts_with('.') {
        cluster_end(part, first_end)
    } else {
        first_end
    };
    part.get(..end).unwrap_or(part)
}

/// Byte offset just past the character cluster starting at byte `start`.
fn cluster_end(text: &str, start: usize) -> usize {
    let mut chars = text.get(start..).unwrap_or("").chars().peekable();
    let Some(first) = chars.next() else {
        return start;
    };
    let mut len = first.len_utf8();
    if is_regional_indicator(first) && chars.peek().copied().is_some_and(is_regional_indicator) {
        len = len.saturating_add(chars.next().map_or(0, char::len_utf8));
    }
    while let Some(next) = chars.peek().copied() {
        if next == '\u{200D}' {
            // Zero-width joiner glues the following character on.
            chars.next();
            len = len.saturating_add(next.len_utf8());
            len = len.saturating_add(chars.next().map_or(0, char::len_utf8));
        } else if is_cluster_extender(next) {
            chars.next();
            len = len.saturating_add(next.len_utf8());
        } else {
            break;
        }
    }
    start.saturating_add(len)
}

const fn is_regional_indicator(c: char) -> bool {
    matches!(c, '\u{1F1E6}'..='\u{1F1FF}')
}

/// Combining marks, variation selectors, emoji skin-tone modifiers, tag chars.
const fn is_cluster_extender(c: char) -> bool {
    matches!(
        c,
        '\u{0300}'..='\u{036F}'
            | '\u{1AB0}'..='\u{1AFF}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20FF}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FE20}'..='\u{FE2F}'
            | '\u{1F3FB}'..='\u{1F3FF}'
            | '\u{E0020}'..='\u{E007F}'
    )
}

/// Join the repo-root basename with its in-repo sub-components by `/`.
fn join_anchored(root_name: &str, sub: &[&str]) -> String {
    if sub.is_empty() {
        root_name.to_owned()
    } else {
        format!("{root_name}/{}", sub.join("/"))
    }
}

impl VariableResolver for DirectoryResolver<'_> {
    fn resolve(&self, name: &str) -> Option<String> {
        let glyphs = Glyphs::from(&self.config.ui);
        match name {
            "path" => {
                let p = self.display_path();
                (!p.is_empty()).then_some(p)
            }
            "read_only" => self.read_only.then(|| "\u{1f512}".to_owned()), // 🔒
            "style" => Some(self.style()),
            "bg" => Some(self.theme.segments.directory.bg_color.as_str().to_owned()),
            "sep_gap" => resolve_separator(self.pos, SeparatorStyle::Chained, glyphs)
                .gap
                .map(str::to_owned),
            "sep_close" => resolve_separator(self.pos, SeparatorStyle::Chained, glyphs)
                .close
                .map(str::to_owned),
            "sep_open" => resolve_separator(self.pos, SeparatorStyle::Chained, glyphs)
                .open
                .map(str::to_owned),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(clippy::missing_errors_doc)]

    use super::DirectoryResolver;
    use crate::config::Config;
    use crate::formatter::{IsFirst, IsLast, SegmentPosition};
    use crate::template::VariableResolver;
    use crate::theme::ThemeConfig;

    #[test]
    fn path_is_present_for_nonempty_cwd() {
        let (config, theme) = (Config::default(), ThemeConfig::default());
        let resolver = DirectoryResolver::new(
            "/home/user/project",
            false,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert!(resolver.resolve("path").is_some());
    }

    #[test]
    fn read_only_none_when_false() {
        let (config, theme) = (Config::default(), ThemeConfig::default());
        let resolver = DirectoryResolver::new(
            "/tmp",
            false,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("read_only"), None);
    }

    #[test]
    fn read_only_some_when_true() {
        let (config, theme) = (Config::default(), ThemeConfig::default());
        let resolver = DirectoryResolver::new(
            "/tmp",
            true,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("read_only"), Some("\u{1f512}".to_owned()));
    }

    #[test]
    fn style_contains_bg_color() {
        let (config, theme) = (Config::default(), ThemeConfig::default());
        let resolver = DirectoryResolver::new(
            "/tmp",
            false,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        let style = resolver.resolve("style").expect("style present");
        // Default DirectoryTheme bg_color is "blue"
        assert!(style.contains("bg:"), "got {style}");
    }

    #[test]
    fn unknown_variable_is_none() {
        let (config, theme) = (Config::default(), ThemeConfig::default());
        let resolver = DirectoryResolver::new(
            "/tmp",
            false,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("nonsuch"), None);
    }

    /// Boundary table for `max_length` values 1 through 6 against an ASCII basename `abcdefgh`
    /// (8 chars).
    ///
    /// Caps at/below `ELLIPSIS_LEN` (3) drop the ellipsis and show the raw tail; caps above it
    /// keep the existing `"..."` + tail behavior byte-for-byte.
    #[test]
    fn max_length_boundary_ascii_1_through_6() {
        let cases: [(usize, &str); 6] = [
            (1, "h"),
            (2, "gh"),
            (3, "fgh"),
            (4, "...h"),
            (5, "...gh"),
            (6, "...fgh"),
        ];
        for (max, expected) in cases {
            let mut config = Config::default();
            config.ui.directory.max_length =
                crate::config::types::MaxPathLength::new(max).expect("valid max_length");
            let theme = ThemeConfig::default();
            let resolver = DirectoryResolver::new(
                "/abcdefgh",
                false,
                &config,
                &theme,
                SegmentPosition::new(IsLast::No, IsFirst::No),
            );
            let path = resolver.resolve("path").expect("path present");
            assert_eq!(path, expected, "max_length={max} got {path:?}");
            assert!(
                path.chars().count() <= max,
                "max_length={max} exceeded cap: {path:?}"
            );
        }
    }

    /// Same boundary table as `max_length_boundary_ascii_1_through_6` but with
    /// a multibyte (CJK) basename, to confirm the small-cap policy counts
    /// `char`s (Unicode scalar values), not bytes.
    #[test]
    fn max_length_boundary_unicode_1_through_6() {
        let cases: [(usize, &str); 6] = [
            (1, "九"),
            (2, "八九"),
            (3, "七八九"),
            (4, "...九"),
            (5, "...八九"),
            (6, "...七八九"),
        ];
        for (max, expected) in cases {
            let mut config = Config::default();
            config.ui.directory.max_length =
                crate::config::types::MaxPathLength::new(max).expect("valid max_length");
            let theme = ThemeConfig::default();
            let resolver = DirectoryResolver::new(
                "/一二三四五六七八九",
                false,
                &config,
                &theme,
                SegmentPosition::new(IsLast::No, IsFirst::No),
            );
            let path = resolver.resolve("path").expect("path present");
            assert_eq!(path, expected, "max_length={max} got {path:?}");
            assert!(
                path.chars().count() <= max,
                "max_length={max} exceeded cap: {path:?}"
            );
        }
    }

    #[test]
    fn unicode_cjk_path_truncated_correctly() {
        let mut config = Config::default();
        // Force truncation at 6 chars; 3 ellipsis + 3 tail
        // Override max_length to 6
        config.ui.directory.max_length =
            crate::config::types::MaxPathLength::new(6).expect("6 is valid");
        let theme = ThemeConfig::default();
        let cjk = "/一二三四五六七八九";
        let resolver = DirectoryResolver::new(
            cjk,
            false,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        // Display mode is Basename by default → last component after "/"
        // basename of "/一二三四五六七八九" = "一二三四五六七八九" (9 chars)
        // truncated to 6 chars = "...七八九" (3 ellipsis + 3 tail)
        let path = resolver.resolve("path").expect("path present");
        assert_eq!(path.chars().count(), 6, "got {path:?}");
        assert!(path.starts_with("..."), "got {path:?}");
    }

    #[test]
    fn sep_close_is_half_circle_when_last() {
        let (config, theme) = (Config::default(), ThemeConfig::default());
        let resolver = DirectoryResolver::new(
            "/tmp",
            false,
            &config,
            &theme,
            SegmentPosition::new(IsLast::Yes, IsFirst::No),
        );
        assert_eq!(resolver.resolve("sep_close"), Some("\u{e0b4}".to_owned()));
    }

    #[test]
    fn sep_close_is_triangle_when_not_last() {
        let (config, theme) = (Config::default(), ThemeConfig::default());
        let resolver = DirectoryResolver::new(
            "/tmp",
            false,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("sep_close"), Some("\u{e0bc}".to_owned()));
    }

    #[test]
    fn sep_gap_is_space_when_not_last() {
        let (config, theme) = (Config::default(), ThemeConfig::default());
        let resolver = DirectoryResolver::new(
            "/tmp",
            false,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("sep_gap"), Some(" ".to_owned()));
    }

    #[test]
    fn sep_gap_is_none_when_last() {
        let (config, theme) = (Config::default(), ThemeConfig::default());
        let resolver = DirectoryResolver::new(
            "/tmp",
            false,
            &config,
            &theme,
            SegmentPosition::new(IsLast::Yes, IsFirst::No),
        );
        assert_eq!(resolver.resolve("sep_gap"), None);
    }

    #[test]
    fn bg_is_some_with_directory_bg_color() {
        let (config, theme) = (Config::default(), ThemeConfig::default());
        let resolver = DirectoryResolver::new(
            "/tmp",
            false,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert!(
            resolver.resolve("bg").is_some(),
            "bg should be non-None for directory segment"
        );
    }

    #[test]
    fn sep_open_is_none_when_first() {
        let (config, theme) = (Config::default(), ThemeConfig::default());
        let resolver = DirectoryResolver::new(
            "/tmp",
            false,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::Yes),
        );
        assert_eq!(resolver.resolve("sep_open"), None);
    }

    #[test]
    fn sep_open_is_glyph_when_not_first() {
        let (config, theme) = (Config::default(), ThemeConfig::default());
        let resolver = DirectoryResolver::new(
            "/tmp",
            false,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("sep_open"), Some("\u{e0ba}".to_owned()));
    }

    use super::abbreviate_path;
    use super::anchor_root;
    use super::contract_home;
    use super::enclosing_anchor_root;
    use super::normalize_cwd;
    use super::repo_anchored_path;
    use super::truncate_to_components;
    use crate::config::types::DirectoryDisplay;
    use std::path::Path;

    // ── contract_home unit tests ────────────────────────────────────────────
    // `contract_home` takes `home` as an explicit parameter rather than
    // reading `std::env::var("HOME")` internally, so these exercise the
    // actual $HOME-contraction logic with a controlled home value — the
    // existing display-mode tests below deliberately use paths NOT under the
    // real environment's $HOME (a documented no-op) or start from an
    // already-contracted "~/..." string, so none of them cover this path.

    #[test]
    fn contract_home_exact_match_becomes_tilde() {
        assert_eq!(contract_home("/home/user", Some("/home/user")), "~");
    }

    #[test]
    fn contract_home_prefix_on_slash_boundary_contracts() {
        assert_eq!(
            contract_home("/home/user/project", Some("/home/user")),
            "~/project"
        );
    }

    #[test]
    fn contract_home_prefix_match_without_slash_boundary_is_unchanged() {
        // "/home/user_name" starts with "/home/user" as a raw string
        // prefix, but the byte right after the prefix isn't '/' — this must
        // NOT contract, or "user_name" would wrongly look like it's under
        // user's home.
        assert_eq!(
            contract_home("/home/user_name/project", Some("/home/user")),
            "/home/user_name/project"
        );
    }

    #[test]
    fn contract_home_no_prefix_match_is_unchanged() {
        assert_eq!(
            contract_home("/var/other", Some("/home/user")),
            "/var/other"
        );
    }

    #[test]
    fn contract_home_none_is_unchanged() {
        assert_eq!(contract_home("/anything", None), "/anything");
    }

    #[test]
    fn contract_home_empty_string_is_unchanged() {
        // Mirrors the old `std::env::var("HOME")` behavior when HOME is set
        // but empty: treated the same as "no home", not as "everything
        // matches an empty prefix".
        assert_eq!(contract_home("/anything", Some("")), "/anything");
    }

    #[test]
    fn anchor_full_shows_whole_in_repo_path() {
        let root = Path::new("/home/u/dev/x/myrepo");
        let cwd = Path::new("/home/u/dev/x/myrepo/a/b/c/d");
        assert_eq!(
            repo_anchored_path(cwd, root, DirectoryDisplay::Full, 3, "…/"),
            Some("myrepo/a/b/c/d".to_owned())
        );
    }

    #[test]
    fn anchor_full_at_repo_root_is_basename() {
        let root = Path::new("/home/u/dev/x/myrepo");
        assert_eq!(
            repo_anchored_path(root, root, DirectoryDisplay::Full, 3, "…/"),
            Some("myrepo".to_owned())
        );
    }

    #[test]
    fn anchor_truncated_under_or_equal_keep_has_no_symbol() {
        let root = Path::new("/home/u/dev/x/myrepo");
        // m == N (R, src, sub -> 3) -> unchanged, no symbol
        assert_eq!(
            repo_anchored_path(
                Path::new("/home/u/dev/x/myrepo/src/sub"),
                root,
                DirectoryDisplay::Truncated,
                3,
                "…/"
            ),
            Some("myrepo/src/sub".to_owned())
        );
        // at repo root
        assert_eq!(
            repo_anchored_path(root, root, DirectoryDisplay::Truncated, 3, "…/"),
            Some("myrepo".to_owned())
        );
    }

    #[test]
    fn anchor_truncated_deeper_than_keep_pins_repo_and_prepends_symbol() {
        let root = Path::new("/home/u/dev/x/myrepo");
        // m == 5 (R,a,b,c,d) > N=3 -> R + sym + last (N-1)=2 of [a,b,c,d] = c,d
        assert_eq!(
            repo_anchored_path(
                Path::new("/home/u/dev/x/myrepo/a/b/c/d"),
                root,
                DirectoryDisplay::Truncated,
                3,
                "…/"
            ),
            Some("myrepo/…/c/d".to_owned())
        );
        // empty symbol
        assert_eq!(
            repo_anchored_path(
                Path::new("/home/u/dev/x/myrepo/a/b/c/d"),
                root,
                DirectoryDisplay::Truncated,
                3,
                ""
            ),
            Some("myrepo/c/d".to_owned())
        );
    }

    #[test]
    fn anchor_truncated_keep_one_collapses_to_repo_name() {
        let root = Path::new("/home/u/dev/x/myrepo");
        assert_eq!(
            repo_anchored_path(
                Path::new("/home/u/dev/x/myrepo/a/b/c/d"),
                root,
                DirectoryDisplay::Truncated,
                1,
                "…/"
            ),
            Some("myrepo".to_owned())
        );
    }

    // ── abbreviate_path unit tests ─────────────────────────────────────────
    // The same cases are asserted through a real agent and shell by
    // tests/fish/e2e_prompt_content.test.fish (#644).

    #[test]
    fn abbreviate_root_is_unchanged() {
        assert_eq!(abbreviate_path("/"), "/");
    }

    #[test]
    fn abbreviate_home_only_is_unchanged() {
        assert_eq!(abbreviate_path("~"), "~");
    }

    #[test]
    fn abbreviate_single_component_under_home_is_unchanged() {
        assert_eq!(abbreviate_path("~/project"), "~/project");
    }

    #[test]
    fn abbreviate_home_path_shortens_non_final_components() {
        // Mirrors shell test: HOME/alpha/beta/project → ~/a/b/project
        assert_eq!(abbreviate_path("~/alpha/beta/project"), "~/a/b/project");
    }

    #[test]
    fn abbreviate_absolute_path_shortens_non_final_components() {
        assert_eq!(abbreviate_path("/usr/local/bin"), "/u/l/bin");
    }

    #[test]
    fn abbreviate_two_levels_under_home() {
        assert_eq!(abbreviate_path("~/src/main"), "~/s/main");
    }

    #[test]
    fn abbreviate_keeps_dot_plus_first_char_for_hidden_dirs() {
        assert_eq!(abbreviate_path("~/.config/fish"), "~/.c/fish");
        assert_eq!(abbreviate_path("/.hidden/a/leaf"), "/.h/a/leaf");
        assert_eq!(abbreviate_path("~/./a/leaf"), "~/./a/leaf");
        assert_eq!(abbreviate_path("~/../a/leaf"), "~/../a/leaf");
    }

    #[test]
    fn abbreviate_keeps_whole_leading_grapheme() {
        assert_eq!(abbreviate_path("~/🇺🇸flags/x"), "~/🇺🇸/x");
        assert_eq!(abbreviate_path("~/e\u{301}tc/x"), "~/e\u{301}/x");
        assert_eq!(
            abbreviate_path("~/👨\u{200D}👩\u{200D}👧home/x"),
            "~/👨\u{200D}👩\u{200D}👧/x"
        );
        assert_eq!(abbreviate_path("~/.🇺🇸flags/x"), "~/.🇺🇸/x");
    }

    #[test]
    fn abbreviate_display_mode_renders_via_resolver() {
        let mut config = Config::default();
        config.ui.directory.display = crate::config::types::DirectoryDisplay::Abbreviated;
        let theme = ThemeConfig::default();
        // Absolute path so home-contraction is a no-op; final component preserved.
        let resolver = DirectoryResolver::new(
            "/usr/local/bin/foo",
            false,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(
            resolver.resolve("path").expect("path present"),
            "/u/l/b/foo"
        );
    }

    #[test]
    fn trailing_slash_cwd_renders_real_basename() {
        let (config, theme) = (Config::default(), ThemeConfig::default());
        for cwd in ["/usr/local/proj/", "/usr/local/proj/."] {
            let resolver = DirectoryResolver::new(
                cwd,
                false,
                &config,
                &theme,
                SegmentPosition::new(IsLast::No, IsFirst::No),
            );
            assert_eq!(resolver.resolve("path").as_deref(), Some("proj"), "{cwd}");
        }
        let resolver = DirectoryResolver::new(
            "/",
            false,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("path").as_deref(), Some("/"));
        assert_eq!(contract_home(&normalize_cwd("/h/u/"), Some("/h/u")), "~");
    }

    #[test]
    fn anchor_returns_none_for_basename_and_outside_repo() {
        let root = Path::new("/home/u/dev/x/myrepo");
        // Basename not anchored
        assert_eq!(
            repo_anchored_path(root, root, DirectoryDisplay::Basename, 3, "…/"),
            None
        );
        // Abbreviated also not anchored (only Truncated/Full are)
        assert_eq!(
            repo_anchored_path(root, root, DirectoryDisplay::Abbreviated, 3, "…/"),
            None
        );
        // cwd not under repo_root
        assert_eq!(
            repo_anchored_path(
                Path::new("/somewhere/else"),
                root,
                DirectoryDisplay::Full,
                3,
                "…/"
            ),
            None
        );
    }

    #[test]
    fn truncate_leaves_home_and_root_markers_alone() {
        assert_eq!(truncate_to_components("~", 3, "…/"), "~");
        assert_eq!(truncate_to_components("/", 3, "…/"), "/");
    }

    #[test]
    fn truncate_keeps_paths_at_or_under_length_unchanged() {
        // 1 component < N
        assert_eq!(truncate_to_components("~/dev", 3, "…/"), "~/dev");
        // exactly N components — unchanged, no symbol
        assert_eq!(truncate_to_components("~/a/b/c", 3, "…/"), "~/a/b/c");
    }

    #[test]
    fn truncate_drops_prefix_and_prepends_symbol_when_deeper() {
        assert_eq!(truncate_to_components("~/a/b/c/d", 3, "…/"), "…/b/c/d");
        assert_eq!(truncate_to_components("~/a/b/c/d", 3, ""), "b/c/d");
        assert_eq!(
            truncate_to_components("/usr/local/bin/foo", 3, "…/"),
            "…/local/bin/foo"
        );
    }

    #[test]
    fn truncate_handles_length_one_boundary() {
        assert_eq!(truncate_to_components("/a/b/c", 1, "…/"), "…/c");
        assert_eq!(truncate_to_components("~/only", 1, "…/"), "~/only");
    }

    #[test]
    fn truncated_display_mode_renders_via_resolver() {
        let mut config = Config::default();
        config.ui.directory.display = crate::config::types::DirectoryDisplay::Truncated;
        config.ui.directory.truncation_length =
            crate::config::types::DirectoryTruncationLength::new(2).expect("2 valid");
        config.ui.directory.truncation_symbol =
            crate::config::types::DirectoryTruncationSymbol::new("…/".to_owned()).expect("valid");
        let theme = ThemeConfig::default();
        // Absolute path not under $HOME so home-contraction is a no-op here.
        let resolver = DirectoryResolver::new(
            "/usr/local/bin/foo",
            false,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("path").expect("path present"), "…/bin/foo");
    }

    use std::fs;
    use tempfile::tempdir;

    fn config_with_truncate_to_repo(display: crate::config::types::DirectoryDisplay) -> Config {
        let mut config = Config::default();
        config.ui.directory.truncate_to_repo = true;
        config.ui.directory.display = display;
        config.ui.directory.truncation_length =
            crate::config::types::DirectoryTruncationLength::new(3).expect("3 valid");
        config.ui.directory.truncation_symbol =
            crate::config::types::DirectoryTruncationSymbol::new("…/".to_owned()).expect("valid");
        config
    }

    #[test]
    fn truncate_to_repo_anchors_truncated_via_resolver() {
        let tmp = tempdir().expect("temp dir");
        let root = fs::canonicalize(tmp.path()).expect("canonicalize root");
        fs::create_dir_all(root.join(".git")).expect("mk .git");
        let deep = root.join("a/b/c/d");
        fs::create_dir_all(&deep).expect("mk deep");
        let root_name = root
            .file_name()
            .and_then(|s| s.to_str())
            .expect("root basename");

        let config =
            config_with_truncate_to_repo(crate::config::types::DirectoryDisplay::Truncated);
        let theme = ThemeConfig::default();
        let cwd = deep.to_str().expect("utf8 cwd");
        let resolver = DirectoryResolver::new(
            cwd,
            false,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        // m=5 (R,a,b,c,d) > N=3 -> R/…/c/d
        assert_eq!(
            resolver.resolve("path").expect("path present"),
            format!("{root_name}/…/c/d")
        );
    }

    #[test]
    fn truncate_to_repo_full_shows_whole_in_repo_path() {
        let tmp = tempdir().expect("temp dir");
        let root = fs::canonicalize(tmp.path()).expect("canonicalize root");
        fs::create_dir_all(root.join(".git")).expect("mk .git");
        let deep = root.join("a/b/c/d");
        fs::create_dir_all(&deep).expect("mk deep");
        let root_name = root
            .file_name()
            .and_then(|s| s.to_str())
            .expect("root basename");

        let config = config_with_truncate_to_repo(crate::config::types::DirectoryDisplay::Full);
        let theme = ThemeConfig::default();
        let cwd = deep.to_str().expect("utf8 cwd");
        let resolver = DirectoryResolver::new(
            cwd,
            false,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(
            resolver.resolve("path").expect("path present"),
            format!("{root_name}/a/b/c/d")
        );
    }

    #[test]
    fn truncate_to_repo_falls_back_when_not_in_repo() {
        let tmp = tempdir().expect("temp dir");
        // No .git anywhere under tmp.
        let dir = fs::canonicalize(tmp.path()).expect("canonicalize");
        let nested = dir.join("alpha/beta");
        fs::create_dir_all(&nested).expect("mk nested");

        let config = config_with_truncate_to_repo(crate::config::types::DirectoryDisplay::Full);
        let theme = ThemeConfig::default();
        let cwd = nested.to_str().expect("utf8 cwd");
        let resolver = DirectoryResolver::new(
            cwd,
            false,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        // Full + not in repo -> home-contracted absolute path unchanged.
        assert_eq!(resolver.resolve("path").expect("path present"), cwd);
    }

    #[test]
    fn anchor_root_ignores_repo_rooted_at_home() {
        let tmp = tempdir().expect("temp dir");
        let home = fs::canonicalize(tmp.path()).expect("canonicalize");
        fs::create_dir_all(home.join(".git")).expect("mk .git");
        let proj = home.join("src/proj");
        fs::create_dir_all(proj.join(".git")).expect("mk proj");

        assert_eq!(anchor_root(Some(home.clone()), Some(&home)), None);
        assert_eq!(anchor_root(Some(proj.clone()), Some(&home)), Some(proj));
        assert_eq!(anchor_root(None, Some(&home)), None);
        assert_eq!(anchor_root(Some(home.clone()), None), Some(home));
    }

    /// Resolver as `new` builds it, but with an injected `$HOME` (no env mutation).
    fn resolver_with_home<'a>(
        cwd: &'a str,
        config: &'a Config,
        theme: &'a ThemeConfig,
        home: &str,
    ) -> DirectoryResolver<'a> {
        let mut resolver = DirectoryResolver::new(
            cwd,
            false,
            config,
            theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        resolver.home = Some(home.to_owned());
        resolver.repo_root = enclosing_anchor_root(cwd, config, Some(home));
        resolver
    }

    #[cfg(unix)]
    #[test]
    fn anchor_root_compares_canonical_forms_through_symlink() {
        let tmp = tempdir().expect("temp dir");
        let real = fs::canonicalize(tmp.path()).expect("canonicalize");
        let home = real.join("home");
        fs::create_dir_all(home.join(".git")).expect("mk home");
        let link = real.join("link");
        std::os::unix::fs::symlink(&home, &link).expect("symlink");

        // Repo root reported via a symlink, home canonical.
        assert_eq!(anchor_root(Some(link), Some(&home)), None);
    }

    #[test]
    fn truncate_to_repo_ignores_repo_rooted_at_home_via_resolver() {
        let tmp = tempdir().expect("temp dir");
        let home = fs::canonicalize(tmp.path()).expect("canonicalize");
        fs::create_dir_all(home.join(".git")).expect("mk .git");
        let cwd_path = home.join("src/x");
        fs::create_dir_all(&cwd_path).expect("mk cwd");
        let cwd = cwd_path.to_str().expect("utf8 cwd");
        let home_str = home.to_str().expect("utf8 home").to_owned();

        let config = config_with_truncate_to_repo(crate::config::types::DirectoryDisplay::Full);
        let theme = ThemeConfig::default();
        let resolver = resolver_with_home(cwd, &config, &theme, &home_str);
        assert_eq!(resolver.resolve("path").expect("path present"), "~/src/x");

        let at_home = resolver_with_home(&home_str, &config, &theme, &home_str);
        assert_eq!(at_home.resolve("path").expect("path present"), "~");
    }

    #[test]
    fn truncate_to_repo_still_anchors_repo_below_home_via_resolver() {
        let tmp = tempdir().expect("temp dir");
        let home = fs::canonicalize(tmp.path()).expect("canonicalize");
        fs::create_dir_all(home.join(".git")).expect("mk home .git");
        let proj = home.join("src/proj");
        fs::create_dir_all(proj.join(".git")).expect("mk proj .git");
        let cwd_path = proj.join("lib");
        fs::create_dir_all(&cwd_path).expect("mk cwd");
        let cwd = cwd_path.to_str().expect("utf8 cwd");
        let home_str = home.to_str().expect("utf8 home").to_owned();

        let config = config_with_truncate_to_repo(crate::config::types::DirectoryDisplay::Full);
        let theme = ThemeConfig::default();
        let resolver = resolver_with_home(cwd, &config, &theme, &home_str);
        assert_eq!(resolver.resolve("path").expect("path present"), "proj/lib");
    }
}
