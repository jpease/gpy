//! Pure wizard selection/navigation state; no terminal I/O.

use crate::config::Config;
use crate::config::types::{DirectoryDisplay, LanguageDisplay};
use crate::palette::manager::PaletteManager;
use crate::theme::{ThemeConfig, ThemeManager};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Theme,
    Palette,
    Segments,
}

impl Section {
    #[must_use]
    pub(crate) const fn next(self) -> Self {
        match self {
            Self::Theme => Self::Palette,
            Self::Palette => Self::Segments,
            Self::Segments => Self::Theme,
        }
    }

    #[must_use]
    pub(crate) const fn prev(self) -> Self {
        match self {
            Self::Theme => Self::Segments,
            Self::Palette => Self::Theme,
            Self::Segments => Self::Palette,
        }
    }
}

pub struct WizardState {
    /// Startup config, never mutated: save base and `dirty()` baseline.
    initial_config: Config,

    available_themes: Vec<String>,
    available_palettes: Vec<String>,
    available_segments: Vec<String>,

    selected_theme: String,
    selected_palette: String,
    selected_segments: BTreeSet<String>,
    /// `ui.directory.display`; cycled with `[`/`]` on the "directory" segment.
    selected_directory_display: DirectoryDisplay,
    /// `language.display`; cycled with `[`/`]` on the "language" segment.
    selected_language_display: LanguageDisplay,
    /// `language.show_versions`; toggled with `v` on the "language" segment.
    selected_language_show_versions: bool,

    focus: Section,

    /// Highlight cursor into `available_themes`; distinct from the selection
    /// until activated.
    theme_cursor: usize,
    palette_cursor: usize,
    segment_cursor: usize,

    /// One filter query shared by all sections; persists across Tab.
    filter_query: String,

    /// Pending edits to the active theme's file (not `config.toml`); dropped
    /// when the theme changes.
    pending_theme: Option<ThemeConfig>,
}

impl WizardState {
    /// Build state, discovering themes/palettes/segments like the list commands.
    pub(crate) fn new(config: Config) -> Self {
        let available_themes: Vec<String> = ThemeManager::discover_available_themes()
            .into_iter()
            .map(|t| t.name)
            .collect();
        let available_palettes: Vec<String> = PaletteManager::discover_available_palettes()
            .into_iter()
            .map(|p| p.name)
            .collect();
        let available_segments = crate::commands::segments::available_segments();

        Self::from_parts(
            config,
            available_themes,
            available_palettes,
            available_segments,
        )
    }

    /// Build state from pre-discovered lists; no I/O (used by tests).
    pub(crate) fn from_parts(
        config: Config,
        available_themes: Vec<String>,
        available_palettes: Vec<String>,
        available_segments: Vec<String>,
    ) -> Self {
        let selected_theme = config.ui.theme.as_str().to_owned();
        let selected_palette = config.ui.palette.to_string();
        let selected_segments = initial_enabled_segments(&config, &available_segments);
        let selected_directory_display = config.ui.directory.display;
        let selected_language_display = config.language.display;
        let selected_language_show_versions = config.language.show_versions;

        let theme_cursor = available_themes
            .iter()
            .position(|t| t == &selected_theme)
            .unwrap_or(0);
        let palette_cursor = available_palettes
            .iter()
            .position(|p| p == &selected_palette)
            .unwrap_or(0);

        Self {
            initial_config: config,
            available_themes,
            available_palettes,
            available_segments,
            selected_theme,
            selected_palette,
            selected_segments,
            selected_directory_display,
            selected_language_display,
            selected_language_show_versions,
            focus: Section::Theme,
            theme_cursor,
            palette_cursor,
            segment_cursor: 0,
            filter_query: String::new(),
            pending_theme: None,
        }
    }

    pub(crate) fn available_themes(&self) -> &[String] {
        &self.available_themes
    }

    pub(crate) fn available_palettes(&self) -> &[String] {
        &self.available_palettes
    }

    pub(crate) fn available_segments(&self) -> &[String] {
        &self.available_segments
    }

    pub(crate) fn selected_theme(&self) -> &str {
        &self.selected_theme
    }

    pub(crate) fn selected_palette(&self) -> &str {
        &self.selected_palette
    }

    /// Startup theme, the baseline for "changed" notes.
    pub(crate) fn initial_theme(&self) -> &str {
        self.initial_config.ui.theme.as_str()
    }

    pub(crate) fn initial_palette(&self) -> &str {
        self.initial_config.ui.palette.as_str()
    }

    pub(crate) fn is_segment_enabled(&self, segment: &str) -> bool {
        self.selected_segments.contains(segment)
    }

    pub(crate) const fn directory_display(&self) -> DirectoryDisplay {
        self.selected_directory_display
    }

    pub(crate) const fn initial_directory_display(&self) -> DirectoryDisplay {
        self.initial_config.ui.directory.display
    }

    /// Builtin segment under the Segments cursor, if Segments has focus.
    /// Every per-segment picker guards on this.
    pub(crate) fn focused_builtin(&self) -> Option<crate::plugin::BuiltinSegment> {
        if self.focus != Section::Segments {
            return None;
        }
        self.available_segments
            .get(self.segment_cursor)
            .and_then(|name| crate::plugin::BuiltinSegment::try_from(name.as_str()).ok())
    }

    pub(crate) const fn language_display(&self) -> LanguageDisplay {
        self.selected_language_display
    }

    pub(crate) const fn initial_language_display(&self) -> LanguageDisplay {
        self.initial_config.language.display
    }

    pub(crate) const fn language_show_versions(&self) -> bool {
        self.selected_language_show_versions
    }

    pub(crate) const fn initial_language_show_versions(&self) -> bool {
        self.initial_config.language.show_versions
    }

    /// Pending clock time format, else `base`'s; unset means `"12"`.
    pub(crate) fn clock_time_format<'a>(&'a self, base: &'a ThemeConfig) -> &'a str {
        self.pending_theme
            .as_ref()
            .unwrap_or(base)
            .segments
            .clock
            .time_format
            .as_deref()
            .unwrap_or("12")
    }

    /// Pending show-seconds, else `base`'s; unset means `false`.
    pub(crate) fn clock_show_seconds(&self, base: &ThemeConfig) -> bool {
        self.pending_theme
            .as_ref()
            .unwrap_or(base)
            .segments
            .clock
            .show_seconds
            .unwrap_or(false)
    }

    /// Pending show-milliseconds, else `base`'s.
    pub(crate) fn duration_show_milliseconds(&self, base: &ThemeConfig) -> bool {
        self.pending_theme
            .as_ref()
            .unwrap_or(base)
            .segments
            .duration
            .show_milliseconds
    }

    /// Pending git show-branch, else `base`'s; unset means `true`.
    pub(crate) fn git_show_branch(&self, base: &ThemeConfig) -> bool {
        self.pending_theme
            .as_ref()
            .unwrap_or(base)
            .segments
            .git
            .show_branch
            .unwrap_or(true)
    }

    pub(crate) fn git_show_ahead_behind(&self, base: &ThemeConfig) -> bool {
        self.pending_theme
            .as_ref()
            .unwrap_or(base)
            .segments
            .git
            .show_ahead_behind
            .unwrap_or(true)
    }

    pub(crate) fn git_show_stash(&self, base: &ThemeConfig) -> bool {
        self.pending_theme
            .as_ref()
            .unwrap_or(base)
            .segments
            .git
            .show_stash
            .unwrap_or(true)
    }

    pub(crate) const fn focus(&self) -> Section {
        self.focus
    }

    pub(crate) const fn theme_cursor(&self) -> usize {
        self.theme_cursor
    }

    pub(crate) const fn palette_cursor(&self) -> usize {
        self.palette_cursor
    }

    pub(crate) const fn segment_cursor(&self) -> usize {
        self.segment_cursor
    }

    /// Indices of `section`'s items matching the shared filter, best first.
    pub(crate) fn filtered_indices(&self, section: Section) -> Vec<usize> {
        let items = match section {
            Section::Theme => &self.available_themes,
            Section::Palette => &self.available_palettes,
            Section::Segments => &self.available_segments,
        };
        super::fuzzy::filter_and_sort(&self.filter_query, items)
    }

    fn focused_filtered_indices(&self) -> Vec<usize> {
        self.filtered_indices(self.focus)
    }

    pub(crate) fn filter_query(&self) -> &str {
        &self.filter_query
    }

    pub(crate) fn select_theme(&mut self, name: &str) {
        if self.available_themes.iter().any(|t| t == name) {
            if name != self.selected_theme {
                // Pending edits belong to the old theme's file.
                self.pending_theme = None;
            }
            name.clone_into(&mut self.selected_theme);
        }
    }

    /// Accumulate an edit to the active theme; the first edit clones `base`.
    pub(crate) fn patch_active_theme(
        &mut self,
        base: &ThemeConfig,
        patch: impl FnOnce(&mut ThemeConfig),
    ) {
        let mut theme = self.pending_theme.take().unwrap_or_else(|| base.clone());
        patch(&mut theme);
        self.pending_theme = Some(theme);
    }

    /// Active theme with pending edits, or `None` if unedited.
    pub(crate) const fn pending_theme(&self) -> Option<&ThemeConfig> {
        self.pending_theme.as_ref()
    }

    pub(crate) fn select_palette(&mut self, name: &str) {
        if self.available_palettes.iter().any(|p| p == name) {
            name.clone_into(&mut self.selected_palette);
        }
    }

    pub(crate) fn toggle_segment(&mut self, segment: &str) {
        if !self.available_segments.iter().any(|s| s == segment) {
            return;
        }
        if !self.selected_segments.remove(segment) {
            self.selected_segments.insert(segment.to_owned());
        }
    }

    /// Cycle directory display; no-op unless "directory" is focused.
    pub(crate) fn cycle_directory_display(&mut self, forward: bool) {
        if self.focused_builtin() != Some(crate::plugin::BuiltinSegment::Directory) {
            return;
        }
        self.selected_directory_display = if forward {
            next_directory_display(self.selected_directory_display)
        } else {
            prev_directory_display(self.selected_directory_display)
        };
    }

    /// Toggle language icon/text; no-op unless "language" is focused.
    pub(crate) fn cycle_language_display(&mut self, forward: bool) {
        if self.focused_builtin() != Some(crate::plugin::BuiltinSegment::Language) {
            return;
        }
        self.selected_language_display = if forward {
            next_language_display(self.selected_language_display)
        } else {
            prev_language_display(self.selected_language_display)
        };
    }

    /// No-op unless "language" is focused.
    pub(crate) fn toggle_language_show_versions(&mut self) {
        if self.focused_builtin() != Some(crate::plugin::BuiltinSegment::Language) {
            return;
        }
        self.selected_language_show_versions = !self.selected_language_show_versions;
    }

    fn patch_focused_segment_theme(
        &mut self,
        segment: crate::plugin::BuiltinSegment,
        base: &ThemeConfig,
        patch: impl FnOnce(&mut ThemeConfig),
    ) {
        if self.focused_builtin() == Some(segment) {
            self.patch_active_theme(base, patch);
        }
    }

    pub(crate) fn cycle_clock_time_format(&mut self, base: &ThemeConfig) {
        let next = if self.clock_time_format(base) == "24" {
            "12"
        } else {
            "24"
        };
        self.patch_focused_segment_theme(crate::plugin::BuiltinSegment::Clock, base, |theme| {
            theme.segments.clock.time_format = Some(next.to_owned());
        });
    }

    pub(crate) fn toggle_clock_show_seconds(&mut self, base: &ThemeConfig) {
        let next = !self.clock_show_seconds(base);
        self.patch_focused_segment_theme(crate::plugin::BuiltinSegment::Clock, base, |theme| {
            theme.segments.clock.show_seconds = Some(next);
        });
    }

    /// Toggle `2.500s` vs `2s` duration precision.
    pub(crate) fn toggle_duration_show_milliseconds(&mut self, base: &ThemeConfig) {
        let next = !self.duration_show_milliseconds(base);
        self.patch_focused_segment_theme(crate::plugin::BuiltinSegment::Duration, base, |theme| {
            theme.segments.duration.show_milliseconds = next;
        });
    }

    pub(crate) fn toggle_git_show_branch(&mut self, base: &ThemeConfig) {
        let next = !self.git_show_branch(base);
        self.patch_focused_segment_theme(crate::plugin::BuiltinSegment::Git, base, |theme| {
            theme.segments.git.show_branch = Some(next);
        });
    }

    pub(crate) fn toggle_git_show_ahead_behind(&mut self, base: &ThemeConfig) {
        let next = !self.git_show_ahead_behind(base);
        self.patch_focused_segment_theme(crate::plugin::BuiltinSegment::Git, base, |theme| {
            theme.segments.git.show_ahead_behind = Some(next);
        });
    }

    pub(crate) fn toggle_git_show_stash(&mut self, base: &ThemeConfig) {
        let next = !self.git_show_stash(base);
        self.patch_focused_segment_theme(crate::plugin::BuiltinSegment::Git, base, |theme| {
            theme.segments.git.show_stash = Some(next);
        });
    }

    pub(crate) const fn next_section(&mut self) {
        self.focus = self.focus.next();
    }

    pub(crate) const fn prev_section(&mut self) {
        self.focus = self.focus.prev();
    }

    /// Move to the previous filtered entry; clamps rather than wraps.
    pub(crate) fn move_cursor_up(&mut self) {
        let indices = self.focused_filtered_indices();
        let Some(&first) = indices.first() else {
            return;
        };
        let cursor = self.focused_cursor_mut();
        let position = indices
            .iter()
            .position(|&i| i == *cursor)
            .unwrap_or(0_usize);
        let previous = position
            .checked_sub(1_usize)
            .and_then(|prev_position| indices.get(prev_position))
            .copied();
        *cursor = previous.unwrap_or(first);
    }

    pub(crate) fn move_cursor_down(&mut self) {
        let indices = self.focused_filtered_indices();
        let Some(&last) = indices.last() else {
            return;
        };
        let cursor = self.focused_cursor_mut();
        let position = indices
            .iter()
            .position(|&i| i == *cursor)
            .unwrap_or(0_usize);
        let next = position
            .checked_add(1_usize)
            .and_then(|next_position| indices.get(next_position))
            .copied();
        *cursor = next.unwrap_or(last);
    }

    /// Select (theme/palette) or toggle (segment) the cursor item. No-op when
    /// nothing is visible, so hidden items are never changed.
    pub(crate) fn activate_cursor(&mut self) {
        if self.focused_filtered_indices().is_empty() {
            return;
        }
        match self.focus {
            Section::Theme => {
                if let Some(name) = self.available_themes.get(self.theme_cursor).cloned() {
                    self.select_theme(&name);
                }
            }
            Section::Palette => {
                if let Some(name) = self.available_palettes.get(self.palette_cursor).cloned() {
                    self.select_palette(&name);
                }
            }
            Section::Segments => {
                if let Some(name) = self.available_segments.get(self.segment_cursor).cloned() {
                    self.toggle_segment(&name);
                }
            }
        }
    }

    /// Append to the filter, snapping cursors whose item no longer matches.
    pub(crate) fn push_filter_char(&mut self, c: char) {
        self.filter_query.push(c);
        self.snap_all_cursors_if_filtered_out();
    }

    /// No cursor snap needed: a prefix of a matching query still matches.
    pub(crate) fn pop_filter_char(&mut self) {
        self.filter_query.pop();
    }

    /// Cursors always hold raw indices, so nothing else to restore.
    pub(crate) fn clear_filter(&mut self) {
        self.filter_query.clear();
    }

    /// Snap every section's cursor (unfocused ones too) to its best match if
    /// filtered out.
    fn snap_all_cursors_if_filtered_out(&mut self) {
        for section in [Section::Theme, Section::Palette, Section::Segments] {
            let indices = self.filtered_indices(section);
            let Some(&first) = indices.first() else {
                continue;
            };
            let cursor = self.cursor_mut(section);
            if !indices.contains(cursor) {
                *cursor = first;
            }
        }
    }

    const fn cursor_mut(&mut self, section: Section) -> &mut usize {
        match section {
            Section::Theme => &mut self.theme_cursor,
            Section::Palette => &mut self.palette_cursor,
            Section::Segments => &mut self.segment_cursor,
        }
    }

    const fn focused_cursor_mut(&mut self) -> &mut usize {
        self.cursor_mut(self.focus)
    }

    pub(crate) fn dirty(&self) -> bool {
        self.selected_theme != self.initial_config.ui.theme.as_str()
            || self.selected_palette != self.initial_config.ui.palette.to_string()
            || self.selected_segments
                != initial_enabled_segments(&self.initial_config, &self.available_segments)
            || self.selected_directory_display != self.initial_config.ui.directory.display
            || self.selected_language_display != self.initial_config.language.display
            || self.selected_language_show_versions != self.initial_config.language.show_versions
            || self.pending_theme.is_some()
    }

    /// Apply the selection onto `config`, editing the segment list in place
    /// like `gpy enable`/`gpy disable`.
    pub(crate) fn apply_to(&self, config: &mut Config) {
        use crate::plugin::BuiltinSegment;

        if let Some(theme) = crate::config::types::ThemeName::new(self.selected_theme.clone()) {
            config.ui.theme = theme;
        }
        if let Some(palette) = crate::config::types::PaletteName::new(self.selected_palette.clone())
        {
            config.ui.palette = palette;
        }
        config.ui.directory.display = self.selected_directory_display;
        config.language.display = self.selected_language_display;
        config.language.show_versions = self.selected_language_show_versions;

        config.git.enabled = self.is_segment_enabled(BuiltinSegment::Git.as_str());
        config.language.enabled = self.is_segment_enabled(BuiltinSegment::Language.as_str());

        // Keep existing order; a deselected listed git/language stays (its
        // flag is cleared instead), unoffered names stay, new names append.
        config.ui.enabled_segments.retain(|name| {
            !self.available_segments.contains(name)
                || self.is_segment_enabled(name)
                || matches!(
                    BuiltinSegment::try_from(name.as_str()),
                    Ok(BuiltinSegment::Git | BuiltinSegment::Language)
                )
        });
        for name in &self.available_segments {
            if self.is_segment_enabled(name) && !config.ui.enabled_segments.contains(name) {
                config.ui.enabled_segments.push(name.clone());
            }
        }
    }
}

const fn next_directory_display(current: DirectoryDisplay) -> DirectoryDisplay {
    match current {
        DirectoryDisplay::Basename => DirectoryDisplay::Abbreviated,
        DirectoryDisplay::Abbreviated => DirectoryDisplay::Truncated,
        DirectoryDisplay::Truncated => DirectoryDisplay::Full,
        DirectoryDisplay::Full => DirectoryDisplay::Basename,
    }
}

const fn prev_directory_display(current: DirectoryDisplay) -> DirectoryDisplay {
    match current {
        DirectoryDisplay::Basename => DirectoryDisplay::Full,
        DirectoryDisplay::Abbreviated => DirectoryDisplay::Basename,
        DirectoryDisplay::Truncated => DirectoryDisplay::Abbreviated,
        DirectoryDisplay::Full => DirectoryDisplay::Truncated,
    }
}

const fn next_language_display(current: LanguageDisplay) -> LanguageDisplay {
    match current {
        LanguageDisplay::Icon => LanguageDisplay::Text,
        LanguageDisplay::Text => LanguageDisplay::Icon,
    }
}

const fn prev_language_display(current: LanguageDisplay) -> LanguageDisplay {
    next_language_display(current)
}

/// Starting segment set, using the same rule as the prompt export.
fn initial_enabled_segments(config: &Config, available: &[String]) -> BTreeSet<String> {
    available
        .iter()
        .filter(|segment| crate::commands::segments::is_effectively_enabled(config, segment))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests;
