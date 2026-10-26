//! Maps git repository state to Starship-exact template variables
//! (`git_branch`/`git_status`) so imported Starship `format` strings work verbatim.

use crate::config::{Config, GitIconSet};
use crate::formatter::chrome::SegmentChrome;
use crate::formatter::separator::Glyphs;
use crate::formatter::{SegmentPosition, truncate_branch};
use crate::git::RepositoryStatus;
use crate::template::VariableResolver;
use crate::theme::{GitState, GitTextElement, ThemeConfig};

/// Nerd Font alternates, used for `icon_set = "nerd_font"` when the `[git.icons]` value
/// is still the Unicode default; an explicit override always wins.
const NERD_FONT_DETACHED: &str = "\u{f126}";
// Unicode defaults mirrored for that comparison.
const UNICODE_DETACHED: &str = "➦";
const NERD_FONT_IN_PROGRESS: &str = "\u{f021}";
const UNICODE_IN_PROGRESS: &str = "↻";
const NERD_FONT_STASH: &str = "\u{f187}";
const UNICODE_STASH: &str = "≡";

/// Resolves Starship-exact git variable names from repository state.
pub struct GitResolver<'a> {
    status: &'a RepositoryStatus,
    config: &'a Config,
    theme: &'a ThemeConfig,
    state: GitState,
    chrome: SegmentChrome<'a>,
}

impl<'a> GitResolver<'a> {
    /// Build a resolver for one git segment render, styled for `state`.
    #[must_use]
    pub fn new(
        status: &'a RepositoryStatus,
        state: GitState,
        config: &'a Config,
        theme: &'a ThemeConfig,
        pos: SegmentPosition,
    ) -> Self {
        let git_theme = &theme.segments.git;
        // Git formats have no `$attr` slot, so the state's attribute leads `$style`.
        let chrome = SegmentChrome::pill(
            pos,
            Glyphs::from(&config.ui),
            git_theme.get_text_color(GitTextElement::Branch, state),
            git_theme.get_bg_color(state),
        )
        .with_attrs(git_theme.get_style(state));
        Self {
            status,
            config,
            theme,
            state,
            chrome,
        }
    }

    /// Branch glyph, or the detached-HEAD indicator when detached.
    fn symbol(&self) -> Option<String> {
        if !self.config.ui.show_icons {
            return None;
        }
        if self.status.detached {
            let icon = self.icon_set_glyph(
                self.config.git.icons.detached.as_str(),
                UNICODE_DETACHED,
                NERD_FONT_DETACHED,
            );
            return Some(icon.to_owned());
        }
        // U+E0A0 (powerline branch).
        Some("\u{e0a0}".to_owned())
    }

    /// Glyph for a stash/detached/in-progress icon: the configured value, or the Nerd
    /// Font alternate when still at the Unicode default under `icon_set = "nerd_font"`.
    fn icon_set_glyph<'b>(
        &self,
        configured: &'b str,
        unicode_default: &str,
        nerd_font_alt: &'static str,
    ) -> &'b str {
        if configured == unicode_default && self.config.git.icon_set == GitIconSet::NerdFont {
            nerd_font_alt
        } else {
            configured
        }
    }

    /// In-progress-operation indicator (e.g. `↻ REBASING 3/5`); `None` unless [`GitState::InProgress`].
    fn state(&self) -> Option<String> {
        if self.state != GitState::InProgress {
            return None;
        }
        let mut text = String::new();
        if self.config.ui.show_icons {
            let icon = self.icon_set_glyph(
                self.config.git.icons.in_progress.as_str(),
                UNICODE_IN_PROGRESS,
                NERD_FONT_IN_PROGRESS,
            );
            text.push_str(icon);
            text.push(' ');
        }
        text.push_str(&self.status.state.as_str().to_uppercase());
        if let Some(progress) = self.status.rebase_progress {
            text.push_str(
                &[
                    " ",
                    &progress.step.to_string(),
                    "/",
                    &progress.total.to_string(),
                ]
                .concat(),
            );
        }
        Some(text)
    }

    /// Stash indicator (e.g. `≡2`), or `None` when there are no stashed changesets.
    fn stash(&self) -> Option<String> {
        if self.status.stash_count == 0 {
            return None;
        }
        let icon = self
            .theme
            .segments
            .git
            .stash_icon
            .as_deref()
            .unwrap_or_else(|| {
                self.icon_set_glyph(
                    self.config.git.icons.stash.as_str(),
                    UNICODE_STASH,
                    NERD_FONT_STASH,
                )
            });
        let mut text = icon.to_owned();
        text.push_str(&self.status.stash_count.to_string());
        Some(text)
    }

    /// Composed dirty/staged/untracked indicators, or `None` when clean.
    /// Theme `[segments.git]` overrides win over `config.git.icons`.
    fn status_text(&self) -> Option<String> {
        let icons = &self.config.git.icons;
        let git_theme = &self.theme.segments.git;
        let show_counts = git_theme.show_counts();

        // (count, theme override, config fallback), in Starship's order.
        let categories = [
            (
                self.status.staged,
                git_theme.staged_icon.as_deref(),
                icons.staged.as_str(),
            ),
            (
                self.status.unstaged,
                git_theme.unstaged_icon.as_deref(),
                icons.unstaged.as_str(),
            ),
            (
                self.status.untracked,
                git_theme.untracked_icon.as_deref(),
                icons.untracked.as_str(),
            ),
            (
                self.status.conflicts,
                git_theme.conflicts_icon.as_deref(),
                icons.conflicts.as_str(),
            ),
        ];

        let mut text = String::new();
        for (count, theme_icon, config_icon) in categories {
            if count > 0 {
                text.push_str(theme_icon.unwrap_or(config_icon));
                if show_counts {
                    text.push_str(&count.to_string());
                }
            }
        }
        (!text.is_empty()).then_some(text)
    }

    /// Ahead/behind indicator (e.g. `↑2↓1`), or `None`.
    fn ahead_behind(&self) -> Option<String> {
        let mut text = String::new();
        let icons = &self.config.git.icons;
        for (count, capped, icon) in [
            (self.status.ahead, self.status.ahead_capped, &icons.ahead),
            (self.status.behind, self.status.behind_capped, &icons.behind),
        ] {
            if count > 0 {
                let suffix = if capped { "+" } else { "" };
                text.push_str(icon);
                text.push_str(&count.to_string());
                text.push_str(suffix);
            }
        }
        (!text.is_empty()).then_some(text)
    }
}

impl VariableResolver for GitResolver<'_> {
    fn resolve(&self, name: &str) -> Option<String> {
        match name {
            "symbol" => self.symbol(),
            "branch" => {
                if !self.theme.segments.git.show_branch() {
                    return None;
                }
                let max = self.config.git.max_branch_length.get();
                let display = truncate_branch(&self.status.branch, max);
                (!display.is_empty()).then_some(display)
            }
            "status" => self.status_text(),
            "ahead_behind" => self
                .theme
                .segments
                .git
                .show_ahead_behind()
                .then(|| self.ahead_behind())
                .flatten(),
            "state" => self.state(),
            "stash" => self
                .theme
                .segments
                .git
                .show_stash()
                .then(|| self.stash())
                .flatten(),
            _ => self.chrome.resolve(name),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    #![allow(clippy::missing_panics_doc)]

    use super::GitResolver;
    use crate::config::Config;
    use crate::formatter::{IsFirst, IsLast, SegmentPosition};
    use crate::git::{RepositoryState, RepositoryStatus};
    use crate::template::VariableResolver;
    use crate::theme::{GitState, ThemeConfig};

    fn status() -> RepositoryStatus {
        RepositoryStatus {
            branch: "main".to_owned(),
            ahead: 2,
            behind: 0,
            ahead_capped: false,
            behind_capped: false,
            staged: 1,
            unstaged: 0,
            untracked: 3,
            conflicts: 0,
            state: RepositoryState::Clean,
            stash_count: 0,
            detached: false,
            rebase_progress: None,
        }
    }

    #[test]
    fn exposes_branch_and_ahead_behind() {
        let (config, theme, st) = (Config::default(), ThemeConfig::default(), status());
        let resolver = GitResolver::new(
            &st,
            GitState::AheadBehind,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("branch"), Some("main".to_owned()));
        assert!(resolver.resolve("ahead_behind").is_some());
        assert!(resolver.resolve("status").is_some());
        assert!(resolver.resolve("style").is_some());
    }

    #[test]
    fn ahead_behind_marks_capped_counts_with_plus() {
        let mut st = status();
        st.ahead = 2;
        st.ahead_capped = true;
        st.behind = 100;
        st.behind_capped = true;
        let (config, theme) = (Config::default(), ThemeConfig::default());
        let resolver = GitResolver::new(
            &st,
            GitState::AheadBehind,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(
            resolver.resolve("ahead_behind"),
            Some(format!(
                "{}2+{}100+",
                config.git.icons.ahead, config.git.icons.behind
            ))
        );
    }

    #[test]
    fn ahead_behind_uncapped_has_no_plus() {
        let mut st = status();
        st.ahead = 2;
        st.ahead_capped = false;
        st.behind = 1;
        let (config, theme) = (Config::default(), ThemeConfig::default());
        let resolver = GitResolver::new(
            &st,
            GitState::AheadBehind,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        let text = resolver.resolve("ahead_behind").unwrap_or_default();
        assert!(!text.is_empty() && !text.contains('+'), "got {text:?}");
    }

    #[test]
    fn clean_repo_has_empty_status_and_ahead_behind() {
        let mut st = status();
        st.ahead = 0;
        st.staged = 0;
        st.untracked = 0;
        let (config, theme) = (Config::default(), ThemeConfig::default());
        let resolver = GitResolver::new(
            &st,
            GitState::Clean,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("status"), None);
        assert_eq!(resolver.resolve("ahead_behind"), None);
    }

    #[test]
    fn unknown_variable_is_none() {
        let (config, theme, st) = (Config::default(), ThemeConfig::default(), status());
        let resolver = GitResolver::new(
            &st,
            GitState::Clean,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("nonsuch"), None);
    }

    #[test]
    fn sep_close_is_half_circle_when_last() {
        let (config, theme, st) = (Config::default(), ThemeConfig::default(), status());
        let resolver = GitResolver::new(
            &st,
            GitState::Clean,
            &config,
            &theme,
            SegmentPosition::new(IsLast::Yes, IsFirst::No),
        );
        assert_eq!(resolver.resolve("sep_close"), Some("\u{e0b4}".to_owned()));
    }

    #[test]
    fn sep_close_is_triangle_when_not_last() {
        let (config, theme, st) = (Config::default(), ThemeConfig::default(), status());
        let resolver = GitResolver::new(
            &st,
            GitState::Clean,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("sep_close"), Some("\u{e0bc}".to_owned()));
    }

    #[test]
    fn sep_gap_is_space_when_not_last() {
        let (config, theme, st) = (Config::default(), ThemeConfig::default(), status());
        let resolver = GitResolver::new(
            &st,
            GitState::Clean,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("sep_gap"), Some(" ".to_owned()));
    }

    #[test]
    fn sep_gap_is_none_when_last() {
        let (config, theme, st) = (Config::default(), ThemeConfig::default(), status());
        let resolver = GitResolver::new(
            &st,
            GitState::Clean,
            &config,
            &theme,
            SegmentPosition::new(IsLast::Yes, IsFirst::No),
        );
        assert_eq!(resolver.resolve("sep_gap"), None);
    }

    #[test]
    fn bg_is_some_for_clean_state() {
        let (config, theme, st) = (Config::default(), ThemeConfig::default(), status());
        let resolver = GitResolver::new(
            &st,
            GitState::Clean,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert!(
            resolver.resolve("bg").is_some(),
            "bg should be non-None for git segment"
        );
    }

    #[test]
    fn sep_open_is_none_when_first() {
        let (config, theme, st) = (Config::default(), ThemeConfig::default(), status());
        let resolver = GitResolver::new(
            &st,
            GitState::Clean,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::Yes),
        );
        assert_eq!(resolver.resolve("sep_open"), None);
    }

    #[test]
    fn sep_open_is_glyph_when_not_first() {
        let (config, theme, st) = (Config::default(), ThemeConfig::default(), status());
        let resolver = GitResolver::new(
            &st,
            GitState::Clean,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("sep_open"), Some("\u{e0ba}".to_owned()));
    }

    #[test]
    fn default_status_uses_config_icons_with_counts() {
        // Default icons: ✚ staged, ? untracked.
        let (config, theme, st) = (Config::default(), ThemeConfig::default(), status());
        let resolver = GitResolver::new(
            &st,
            GitState::AheadBehind,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("status"), Some("✚1?3".to_owned()));
    }

    #[test]
    fn theme_status_icons_override_and_show_counts_false_suppresses_counts() {
        // Theme overrides icons and drops counts: symbols-only like `+?`.
        let config = Config::default();
        let mut theme = ThemeConfig::default();
        theme.segments.git.show_counts = Some(false);
        theme.segments.git.staged_icon = Some("+".to_owned());
        theme.segments.git.untracked_icon = Some("?".to_owned());
        let st = status();
        let resolver = GitResolver::new(
            &st,
            GitState::AheadBehind,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("status"), Some("+?".to_owned()));
    }

    #[test]
    fn state_empty_when_not_in_progress() {
        let (config, theme, st) = (Config::default(), ThemeConfig::default(), status());
        for state in [
            GitState::Clean,
            GitState::AheadBehind,
            GitState::UntrackedOnly,
            GitState::Modified,
            GitState::Conflicts,
        ] {
            let resolver = GitResolver::new(
                &st,
                state,
                &config,
                &theme,
                SegmentPosition::new(IsLast::No, IsFirst::No),
            );
            assert_eq!(
                resolver.resolve("state"),
                None,
                "state should be empty for {state:?}"
            );
        }
    }

    #[test]
    fn state_renders_label_for_rebasing_with_progress() {
        let (config, theme) = (Config::default(), ThemeConfig::default());
        let mut st = status();
        st.state = RepositoryState::Rebasing;
        st.rebase_progress = Some(crate::git::RebaseProgress { step: 3, total: 5 });
        let resolver = GitResolver::new(
            &st,
            GitState::InProgress,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("state"), Some("↻ REBASING 3/5".to_owned()));
    }

    #[test]
    fn state_renders_label_without_progress_for_non_interactive_rebase() {
        let (config, theme) = (Config::default(), ThemeConfig::default());
        let mut st = status();
        st.state = RepositoryState::Rebasing;
        st.rebase_progress = None;
        let resolver = GitResolver::new(
            &st,
            GitState::InProgress,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("state"), Some("↻ REBASING".to_owned()));
    }

    #[test]
    fn state_renders_label_for_applying() {
        let (config, theme) = (Config::default(), ThemeConfig::default());
        let mut st = status();
        st.state = RepositoryState::Applying;
        st.rebase_progress = None;
        let resolver = GitResolver::new(
            &st,
            GitState::InProgress,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("state"), Some("↻ APPLYING".to_owned()));
    }

    #[test]
    fn stash_empty_when_zero() {
        let (config, theme, st) = (Config::default(), ThemeConfig::default(), status());
        assert_eq!(st.stash_count, 0);
        let resolver = GitResolver::new(
            &st,
            GitState::Clean,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("stash"), None);
    }

    #[test]
    fn stash_renders_icon_and_count() {
        let (config, theme) = (Config::default(), ThemeConfig::default());
        let mut st = status();
        st.stash_count = 2;
        let resolver = GitResolver::new(
            &st,
            GitState::Clean,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("stash"), Some("≡2".to_owned()));
    }

    #[test]
    fn stash_theme_icon_overrides_config_default() {
        let config = Config::default();
        let mut theme = ThemeConfig::default();
        theme.segments.git.stash_icon = Some("$".to_owned());
        let mut st = status();
        st.stash_count = 1;
        let resolver = GitResolver::new(
            &st,
            GitState::Clean,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("stash"), Some("$1".to_owned()));
    }

    #[test]
    fn show_branch_false_suppresses_branch() {
        let config = Config::default();
        let mut theme = ThemeConfig::default();
        theme.segments.git.show_branch = Some(false);
        let st = status();
        let resolver = GitResolver::new(
            &st,
            GitState::AheadBehind,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("branch"), None);
        assert!(resolver.resolve("ahead_behind").is_some());
    }

    #[test]
    fn show_ahead_behind_false_suppresses_arrows() {
        let config = Config::default();
        let mut theme = ThemeConfig::default();
        theme.segments.git.show_ahead_behind = Some(false);
        let st = status();
        let resolver = GitResolver::new(
            &st,
            GitState::AheadBehind,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("ahead_behind"), None);
        assert_eq!(resolver.resolve("branch"), Some("main".to_owned()));
    }

    #[test]
    fn show_stash_false_suppresses_stash() {
        let config = Config::default();
        let mut theme = ThemeConfig::default();
        theme.segments.git.show_stash = Some(false);
        let mut st = status();
        st.stash_count = 2;
        let resolver = GitResolver::new(
            &st,
            GitState::Clean,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("stash"), None);
    }

    #[test]
    fn content_visibility_flags_default_to_shown_when_unset() {
        // Default `GitTheme` leaves these `None`, i.e. always on.
        let (config, theme) = (Config::default(), ThemeConfig::default());
        assert_eq!(theme.segments.git.show_branch, None);
        assert_eq!(theme.segments.git.show_ahead_behind, None);
        assert_eq!(theme.segments.git.show_stash, None);
        let mut st = status();
        st.stash_count = 1;
        let resolver = GitResolver::new(
            &st,
            GitState::AheadBehind,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("branch"), Some("main".to_owned()));
        assert!(resolver.resolve("ahead_behind").is_some());
        assert!(resolver.resolve("stash").is_some());
    }

    #[test]
    fn symbol_uses_detached_glyph_when_detached() {
        let (config, theme) = (Config::default(), ThemeConfig::default());
        let mut st = status();
        st.detached = true;
        let resolver = GitResolver::new(
            &st,
            GitState::Clean,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("symbol"), Some("➦".to_owned()));
    }

    #[test]
    fn symbol_uses_nerd_font_detached_glyph_when_icon_set_is_nerd_font() {
        let mut config = Config::default();
        config.git.icon_set = crate::config::GitIconSet::NerdFont;
        let theme = ThemeConfig::default();
        let mut st = status();
        st.detached = true;
        let resolver = GitResolver::new(
            &st,
            GitState::Clean,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("symbol"), Some("\u{f126}".to_owned()));
    }

    #[test]
    fn branch_renders_bare_sha_when_detached_no_head_prefix() {
        let (config, theme) = (Config::default(), ThemeConfig::default());
        let mut st = status();
        st.detached = true;
        st.branch = "a1b2c3d".to_owned();
        let resolver = GitResolver::new(
            &st,
            GitState::Clean,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        let branch = resolver.resolve("branch").expect("branch should resolve");
        assert!(
            !branch.contains("HEAD@"),
            "branch must not carry the legacy HEAD@ prefix, got {branch:?}"
        );
        assert_eq!(branch, "a1b2c3d");
    }

    #[test]
    fn style_has_no_attr_prefix_when_git_style_unset() {
        let (config, theme, st) = (Config::default(), ThemeConfig::default(), status());
        let resolver = GitResolver::new(
            &st,
            GitState::Conflicts,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        let style = resolver.resolve("style").expect("style should resolve");
        assert!(
            style.starts_with("fg:"),
            "expected no attr prefix, got {style:?}"
        );
    }

    #[test]
    fn style_includes_git_style_attr_when_set() {
        let config = Config::default();
        let mut theme = ThemeConfig::default();
        theme
            .segments
            .git
            .git_style
            .insert("conflicts".to_owned(), "bold".to_owned());
        let st = status();
        let resolver = GitResolver::new(
            &st,
            GitState::Conflicts,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        let style = resolver.resolve("style").expect("style should resolve");
        assert!(
            style.starts_with("bold fg:"),
            "expected bold attr prefix, got {style:?}"
        );
    }

    #[test]
    fn symbol_uses_default_branch_glyph_when_not_detached() {
        let (config, theme, st) = (Config::default(), ThemeConfig::default(), status());
        assert!(!st.detached);
        let resolver = GitResolver::new(
            &st,
            GitState::Clean,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(resolver.resolve("symbol"), Some("\u{e0a0}".to_owned()));
    }

    #[test]
    fn symbol_none_when_icons_disabled() {
        let mut config = Config::default();
        config.ui.show_icons = false;
        let theme = ThemeConfig::default();
        let mut st = status();
        st.detached = true;
        let resolver = GitResolver::new(
            &st,
            GitState::Clean,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(
            resolver.resolve("symbol"),
            None,
            "symbol must be suppressed when icons are disabled, detached or not"
        );
    }

    #[test]
    fn branch_unaffected_by_detached_flag_for_normal_branch_name() {
        let (config, theme) = (Config::default(), ThemeConfig::default());
        let mut st = status();
        st.detached = true;
        st.branch = "main".to_owned();
        let resolver = GitResolver::new(
            &st,
            GitState::Clean,
            &config,
            &theme,
            SegmentPosition::new(IsLast::No, IsFirst::No),
        );
        assert_eq!(
            resolver.resolve("branch"),
            Some("main".to_owned()),
            "branch should render the underlying name regardless of detached flag"
        );
    }
}
