//! Directory segment variables (`path`, `read_only`, `style`), Starship-compatible.

use crate::config::Config;
use crate::config::DirectorySettings;
use crate::config::types::DirectoryDisplay;
use crate::formatter::SegmentPosition;
use crate::formatter::separator::{Glyphs, SeparatorStyle, separator_var};
use crate::template::VariableResolver;
use crate::theme::ThemeConfig;
use std::path::{Path, PathBuf};

/// Resolves Starship-exact directory variable names. All I/O happens once in
/// [`DirectoryResolver::new`]; `resolve("path")` may run several times per render.
pub struct DirectoryResolver<'a> {
    /// Lexically normalized `cwd` (see [`normalize_cwd`]); drives every display mode.
    cwd: String,
    read_only: bool,
    config: &'a Config,
    theme: &'a ThemeConfig,
    pos: SegmentPosition,
    /// `$HOME`, read once at construction.
    home: Option<String>,
    /// Canonicalized `cwd` (or `cwd` if that fails); used only with `repo_root`.
    canonical_cwd: PathBuf,
    /// Enclosing git repo root, computed once at construction.
    repo_root: Option<PathBuf>,
}

impl<'a> DirectoryResolver<'a> {
    /// Build a resolver; reads `$HOME`, canonicalizes `cwd`, finds the repo root.
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

        let home_contracted = contract_home(&self.cwd, self.home.as_deref());

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

        // Tail-style max_length truncation.
        let max = self.config.ui.directory.max_length.get();
        let char_count = base.chars().count();
        if char_count <= max {
            return base;
        }

        // A cap ≤ the ellipsis width can't fit "..." plus content: show the raw tail.
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

/// Lexically normalize a cwd: drop trailing/repeated `/` and `.`; keep `..`
/// (symlinks). String-based so separators stay `/` on Windows.
fn normalize_cwd(cwd: &str) -> String {
    let joined = cwd
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>()
        .join("/");
    if cwd.starts_with('/') {
        format!("/{joined}")
    } else if joined.is_empty() {
        cwd.to_owned()
    } else {
        joined
    }
}

/// Enclosing git repo root for `cwd`, minus a root that is `$HOME`. Canonicalizes
/// only when `truncate_to_repo` is on and a root was found.
fn enclosing_anchor_root(cwd: &str, config: &Config, home: Option<&str>) -> Option<PathBuf> {
    let found_root = crate::git::find_repo_root(Path::new(cwd));
    let canonical_home = found_root
        .as_ref()
        .filter(|_| config.ui.directory.truncate_to_repo)
        .and(home)
        .and_then(|h| std::fs::canonicalize(h).ok());
    anchor_root(found_root, canonical_home.as_deref())
}

/// Drop `repo_root` when it is `$HOME` (dotfiles repo); Starship doesn't anchor there.
/// `canonical_home` must already be canonical.
fn anchor_root(repo_root: Option<PathBuf>, canonical_home: Option<&Path>) -> Option<PathBuf> {
    let root = repo_root?;
    let is_home = canonical_home.is_some_and(|home| {
        root == home || std::fs::canonicalize(&root).is_ok_and(|canonical| canonical == home)
    });
    (!is_home).then_some(root)
}

/// Contract a `$HOME` prefix on `cwd` to `~`; `None`/empty `home` is a no-op.
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

/// Repo-anchored display path when `truncate_to_repo` is on and a root exists.
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

/// Truncate to the last `keep` components (leading `~` not counted), prefixed
/// with `symbol`; unchanged when already short enough.
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

/// Render `cwd` anchored at the repo root's basename; `Truncated` caps the
/// trailing components at `keep`. `None` when not applicable (caller falls back).
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

/// Shorten each non-final component to its first character (hidden: `.` plus one),
/// e.g. `~/.config/fish` → `~/.c/fish`. The shells rely on this implementation.
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

/// First character cluster of a component (two for hidden names); `.`/`..` whole.
/// Clusters approximated with `char` rules: flags, combining marks, ZWJ sequences.
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
            "sep_gap" | "sep_open" | "sep_close" => {
                separator_var(name, self.pos, SeparatorStyle::Chained, glyphs)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
