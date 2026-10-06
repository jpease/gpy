//! Pure wizard selection/navigation state — no terminal I/O.
//!
//! Everything here is a plain data transition testable with `#[test]`. The
//! terminal render loop (`wizard/mod.rs`) only ever reads this state to draw
//! and calls its methods in response to key events; it never inspects or
//! mutates config directly.

use crate::config::Config;
use crate::config::types::{DirectoryDisplay, LanguageDisplay};
use crate::palette::manager::PaletteManager;
use crate::theme::{ThemeConfig, ThemeManager};
use std::collections::BTreeSet;

/// Which section of the wizard currently has input focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Theme,
    Palette,
    Segments,
}

impl Section {
    /// Cycle to the next section, wrapping from `Segments` back to `Theme`.
    #[must_use]
    pub(crate) const fn next(self) -> Self {
        match self {
            Self::Theme => Self::Palette,
            Self::Palette => Self::Segments,
            Self::Segments => Self::Theme,
        }
    }

    /// Cycle to the previous section, wrapping from `Theme` back to `Segments`.
    #[must_use]
    pub(crate) const fn prev(self) -> Self {
        match self {
            Self::Theme => Self::Segments,
            Self::Palette => Self::Theme,
            Self::Segments => Self::Palette,
        }
    }
}

/// In-memory wizard selection state, independent of the terminal.
pub struct WizardState {
    /// The config as loaded at wizard startup — never mutated; used both as
    /// the save-path base (fields other than theme/palette/segments pass
    /// through untouched) and as the baseline `dirty()` diffs against.
    initial_config: Config,

    available_themes: Vec<String>,
    available_palettes: Vec<String>,
    available_segments: Vec<String>,

    selected_theme: String,
    selected_palette: String,
    selected_segments: BTreeSet<String>,
    /// Directory segment display style (`ui.directory.display`). Unlike
    /// theme/palette/segments, this has no dedicated `Section` — it's cycled
    /// via `[`/`]` while `Section::Segments`'s cursor sits on `"directory"`
    /// (see `cycle_directory_display`).
    selected_directory_display: DirectoryDisplay,
    /// Language segment display style (`language.display`, icon vs text).
    /// Same no-dedicated-`Section` shape as `selected_directory_display`,
    /// cycled via `[`/`]` while `Section::Segments`'s cursor sits on
    /// `"language"` (see `cycle_language_display`).
    selected_language_display: LanguageDisplay,
    /// Whether the language segment shows detected version numbers
    /// (`language.show_versions`). Toggled via `v` while `Section::Segments`'s
    /// cursor sits on `"language"` (see `toggle_language_show_versions`).
    selected_language_show_versions: bool,

    focus: Section,

    /// Highlight-cursor index into `available_themes`, moved by Up/Down while
    /// `Section::Theme` has focus and applied via [`Self::activate_cursor`].
    /// Distinct from `selected_theme`: the cursor can move without changing
    /// the selection until Space/Enter activates it.
    theme_cursor: usize,
    /// Highlight-cursor index into `available_palettes`. See `theme_cursor`.
    palette_cursor: usize,
    /// Highlight-cursor index into `available_segments`. Segments are a
    /// multi-select (`selected_segments`), so unlike `theme_cursor`/
    /// `palette_cursor` there is no "currently selected" starting point to
    /// mirror — this starts at `0`.
    segment_cursor: usize,

    /// Live filter query, shared across all three sections: typing narrows
    /// Theme/Palette/Segments simultaneously to items matching the same
    /// query (see `filtered_indices`). Edited via `push_filter_char`/
    /// `pop_filter_char`/`clear_filter` while `keys::InputMode::Filter` is active.
    /// Persists across `next_section`/`prev_section` — tabbing to another
    /// section keeps filtering active with the same query; only `Esc`
    /// clears it.
    filter_query: String,

    /// Pending per-field edits to the active theme's `ThemeConfig`, or `None`
    /// when no theme field has been edited this session. Unlike
    /// `selected_theme`/`selected_palette`/segments (which live in
    /// `config.toml`), these fields live in the per-theme TOML file, so they
    /// persist through a different path — `save` writes this to the user
    /// themes directory via [`ThemeManager::save_user_theme`] (#407).
    ///
    /// Seeded and accumulated by [`patch_active_theme`](Self::patch_active_theme)
    /// and discarded by [`select_theme`](Self::select_theme) when the theme
    /// changes (the edits described the previously-selected theme).
    pending_theme: Option<ThemeConfig>,
}

impl WizardState {
    /// Build wizard state from the loaded config, discovering themes,
    /// palettes, and segments the same way `gpy theme list` / `gpy palette
    /// list` / `gpy segments` do.
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

    /// Build wizard state from the loaded config and already-gathered
    /// available-themes/palettes/segments lists — no discovery I/O.
    ///
    /// [`Self::new`] is the I/O-performing wrapper around this; tests call
    /// this directly with fixed lists so wizard unit tests are deterministic
    /// regardless of what themes/palettes a developer happens to have
    /// installed under `$HOME/.config/gpy/` (#607).
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

    // --- read accessors used by rendering (later tasks) ---

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

    /// The theme as loaded at wizard startup — the baseline `dirty()` and the
    /// master/detail panel's "changed from startup" note diff against.
    pub(crate) fn initial_theme(&self) -> &str {
        self.initial_config.ui.theme.as_str()
    }

    /// The palette as loaded at wizard startup. See `initial_theme`.
    pub(crate) fn initial_palette(&self) -> &str {
        self.initial_config.ui.palette.as_str()
    }

    pub(crate) fn is_segment_enabled(&self, segment: &str) -> bool {
        self.selected_segments.contains(segment)
    }

    /// The directory segment's currently selected display style.
    pub(crate) const fn directory_display(&self) -> DirectoryDisplay {
        self.selected_directory_display
    }

    /// The directory display style as loaded at wizard startup. See
    /// `initial_theme`.
    pub(crate) const fn initial_directory_display(&self) -> DirectoryDisplay {
        self.initial_config.ui.directory.display
    }

    /// The builtin segment `Section::Segments`'s cursor currently sits on, or
    /// `None` when focus isn't on `Section::Segments`, the cursor is out of
    /// range, or the highlighted segment isn't a builtin (e.g. it's a plugin
    /// segment). The single entry point every per-segment picker guard
    /// (`cycle_directory_display`, `cycle_language_display`,
    /// `toggle_language_show_versions`, `cycle_clock_time_format`,
    /// `toggle_clock_show_seconds`, `toggle_duration_show_milliseconds`,
    /// `toggle_git_show_branch`/`toggle_git_show_ahead_behind`/
    /// `toggle_git_show_stash`) compares against, replacing what used to be
    /// five near-identical `is_*_segment_focused` predicates.
    pub(crate) fn focused_builtin(&self) -> Option<crate::plugin::BuiltinSegment> {
        if self.focus != Section::Segments {
            return None;
        }
        self.available_segments
            .get(self.segment_cursor)
            .and_then(|name| crate::plugin::BuiltinSegment::try_from(name.as_str()).ok())
    }

    /// The language segment's currently selected display style (icon vs text).
    pub(crate) const fn language_display(&self) -> LanguageDisplay {
        self.selected_language_display
    }

    /// The language display style as loaded at wizard startup. See
    /// `initial_theme`.
    pub(crate) const fn initial_language_display(&self) -> LanguageDisplay {
        self.initial_config.language.display
    }

    /// Whether the language segment currently shows detected version numbers.
    pub(crate) const fn language_show_versions(&self) -> bool {
        self.selected_language_show_versions
    }

    /// The show-versions flag as loaded at wizard startup. See
    /// `initial_theme`.
    pub(crate) const fn initial_language_show_versions(&self) -> bool {
        self.initial_config.language.show_versions
    }

    /// The clock segment's effective time format (`"12"` or `"24"`): this
    /// session's pending theme edit (`pending_theme`) if one exists, else
    /// `base` — the loaded active theme (the render loop's cached
    /// `ThemeConfig`, same value callers pass to `patch_active_theme`).
    /// Absent in either (`None`) means `"12"`, `ClockTheme`'s documented
    /// default (`theme/model.rs`).
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

    /// The clock segment's effective show-seconds flag — same
    /// pending-edit-or-`base` shape as `clock_time_format`. Absent (`None`)
    /// means `false`, `ClockTheme`'s documented default.
    pub(crate) fn clock_show_seconds(&self, base: &ThemeConfig) -> bool {
        self.pending_theme
            .as_ref()
            .unwrap_or(base)
            .segments
            .clock
            .show_seconds
            .unwrap_or(false)
    }

    /// The duration segment's effective show-milliseconds flag: this
    /// session's pending theme edit (`pending_theme`) if one exists, else
    /// `base` — the loaded active theme (the render loop's cached
    /// `ThemeConfig`, same value callers pass to `patch_active_theme`).
    /// Unlike `clock_time_format`/`clock_show_seconds`, `DurationTheme`'s
    /// field is a plain `bool` (default `true`, `theme/model.rs`), not an
    /// `Option<bool>` — there's no "unset" state to fall back from.
    pub(crate) fn duration_show_milliseconds(&self, base: &ThemeConfig) -> bool {
        self.pending_theme
            .as_ref()
            .unwrap_or(base)
            .segments
            .duration
            .show_milliseconds
    }

    /// The git segment's effective show-branch flag: this session's pending
    /// theme edit (`pending_theme`) if one exists, else `base` — the loaded
    /// active theme (the render loop's cached `ThemeConfig`, same value
    /// callers pass to `patch_active_theme`). Absent (`None`) means `true`,
    /// `GitTheme`'s documented always-on default (`theme/model.rs`).
    pub(crate) fn git_show_branch(&self, base: &ThemeConfig) -> bool {
        self.pending_theme
            .as_ref()
            .unwrap_or(base)
            .segments
            .git
            .show_branch
            .unwrap_or(true)
    }

    /// The git segment's effective show-ahead/behind flag — same
    /// pending-edit-or-`base` shape as `git_show_branch`. Absent (`None`)
    /// means `true`, `GitTheme`'s always-on default.
    pub(crate) fn git_show_ahead_behind(&self, base: &ThemeConfig) -> bool {
        self.pending_theme
            .as_ref()
            .unwrap_or(base)
            .segments
            .git
            .show_ahead_behind
            .unwrap_or(true)
    }

    /// The git segment's effective show-stash flag — same
    /// pending-edit-or-`base` shape as `git_show_branch`. Absent (`None`)
    /// means `true`, `GitTheme`'s always-on default.
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

    /// Indices into `section`'s `available_*` list that match the shared
    /// `filter_query`, ordered by match quality (see
    /// `fuzzy::filter_and_sort`). An empty query returns every index in
    /// original order. The same query narrows all three sections at once —
    /// there is one filter, not one per section.
    pub(crate) fn filtered_indices(&self, section: Section) -> Vec<usize> {
        let items = match section {
            Section::Theme => &self.available_themes,
            Section::Palette => &self.available_palettes,
            Section::Segments => &self.available_segments,
        };
        super::fuzzy::filter_and_sort(&self.filter_query, items)
    }

    /// `filtered_indices` for whichever section currently has focus. Drives
    /// cursor movement (`move_cursor_up`/`move_cursor_down`).
    fn focused_filtered_indices(&self) -> Vec<usize> {
        self.filtered_indices(self.focus)
    }

    /// The live filter query shared by all three sections. See
    /// `filter_query` (the field) / `filtered_indices`.
    pub(crate) fn filter_query(&self) -> &str {
        &self.filter_query
    }

    // --- transitions ---

    pub(crate) fn select_theme(&mut self, name: &str) {
        if self.available_themes.iter().any(|t| t == name) {
            if name != self.selected_theme {
                // Pending per-field edits described the previously-selected
                // theme; switching themes drops them rather than misapplying
                // one theme's field edits onto a different theme's file.
                self.pending_theme = None;
            }
            name.clone_into(&mut self.selected_theme);
        }
    }

    /// Record an edit to one field (or several) of the active theme,
    /// accumulating onto any edits already pending this session.
    ///
    /// The first edit clones `base` — the currently-selected theme as loaded
    /// from disk (the render loop's cached `ThemeConfig`) — and applies `patch`
    /// to that clone; later edits patch the already-accumulated copy, so `base`
    /// only seeds the very first edit. The result is held in `pending_theme`
    /// until [`save`](crate::commands::wizard::save) writes it to the user
    /// themes directory via [`ThemeManager::save_user_theme`].
    ///
    /// This is the generic per-field theme-edit entry point (#407): the
    /// clock/duration/git (and eventual per-swatch color) pickers each call it
    /// with a closure touching only their own field, so nothing else in the
    /// theme changes. It performs no I/O — the caller supplies the loaded
    /// `base` — keeping it a pure state transition like every other method
    /// here. Switching themes ([`select_theme`](Self::select_theme)) discards
    /// any pending edits.
    pub(crate) fn patch_active_theme(
        &mut self,
        base: &ThemeConfig,
        patch: impl FnOnce(&mut ThemeConfig),
    ) {
        let mut theme = self.pending_theme.take().unwrap_or_else(|| base.clone());
        patch(&mut theme);
        self.pending_theme = Some(theme);
    }

    /// The active theme with this session's pending per-field edits applied, or
    /// `None` if no theme field has been edited. Read by `save` to persist the
    /// edits to the user themes directory, and by the render loop
    /// (`wizard/mod.rs`'s `draw_frame`) to feed the live preview the edited
    /// theme instead of the unedited on-disk one.
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

    /// Cycle the directory display style forward (`forward: true`) or
    /// backward through its 4 variants, wrapping at either end.
    ///
    /// A no-op unless `Section::Segments`'s cursor is on `"directory"` (see
    /// `focused_builtin`) — mirrors `toggle_segment`/
    /// `activate_cursor`'s pattern of guarding inside the transition itself
    /// rather than trusting the caller (`keys.rs`) to check first.
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

    /// Cycle the language display style between icon and text forward
    /// (`forward: true`) or backward — only 2 variants exist, so either
    /// direction toggles to the other one.
    ///
    /// A no-op unless `Section::Segments`'s cursor is on `"language"` (see
    /// `focused_builtin`) — mirrors `cycle_directory_display`'s
    /// guard-inside-the-transition pattern. Kept as a `forward` param (rather
    /// than a plain toggle) so `keys.rs`'s shared `[`/`]` handler can call
    /// this and `cycle_directory_display` identically.
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

    /// Toggle whether the language segment shows detected version numbers.
    ///
    /// A no-op unless `Section::Segments`'s cursor is on `"language"` (see
    /// `focused_builtin`) — mirrors `cycle_directory_display`/
    /// `toggle_segment`'s guard-inside-the-transition pattern.
    pub(crate) fn toggle_language_show_versions(&mut self) {
        if self.focused_builtin() != Some(crate::plugin::BuiltinSegment::Language) {
            return;
        }
        self.selected_language_show_versions = !self.selected_language_show_versions;
    }

    /// Toggle the clock segment's time format between `"12"` and `"24"`,
    /// via `patch_active_theme` (#407) since — unlike directory/language
    /// display — this field lives in the per-theme TOML, not `config.toml`.
    ///
    /// A no-op unless `Section::Segments`'s cursor is on `"clock"` (see
    /// `focused_builtin`) — mirrors `cycle_directory_display`/
    /// `cycle_language_display`'s guard-inside-the-transition pattern. Takes
    /// no `forward` direction: like language's icon/text, only 2 variants
    /// exist with no natural earlier/later ordering, so `keys.rs`'s shared
    /// `[`/`]` handler flips to the other one either way, same as
    /// `cycle_language_display`.
    pub(crate) fn cycle_clock_time_format(&mut self, base: &ThemeConfig) {
        if self.focused_builtin() != Some(crate::plugin::BuiltinSegment::Clock) {
            return;
        }
        let next = if self.clock_time_format(base) == "24" {
            "12"
        } else {
            "24"
        };
        self.patch_active_theme(base, |theme| {
            theme.segments.clock.time_format = Some(next.to_owned());
        });
    }

    /// Toggle whether the clock segment shows seconds, via
    /// `patch_active_theme` (#407) — see `cycle_clock_time_format`.
    ///
    /// A no-op unless `Section::Segments`'s cursor is on `"clock"` (see
    /// `focused_builtin`) — mirrors `toggle_language_show_versions`'s
    /// guard-inside-the-transition pattern.
    pub(crate) fn toggle_clock_show_seconds(&mut self, base: &ThemeConfig) {
        if self.focused_builtin() != Some(crate::plugin::BuiltinSegment::Clock) {
            return;
        }
        let next = !self.clock_show_seconds(base);
        self.patch_active_theme(base, |theme| {
            theme.segments.clock.show_seconds = Some(next);
        });
    }

    /// Toggle whether the duration segment renders millisecond precision
    /// (`2.500s`) versus whole seconds only (`2s`), via `patch_active_theme`
    /// (#407) — see `cycle_clock_time_format`.
    ///
    /// A no-op unless `Section::Segments`'s cursor is on `"duration"` (see
    /// `focused_builtin`) — mirrors
    /// `toggle_clock_show_seconds`'s guard-inside-the-transition pattern.
    pub(crate) fn toggle_duration_show_milliseconds(&mut self, base: &ThemeConfig) {
        if self.focused_builtin() != Some(crate::plugin::BuiltinSegment::Duration) {
            return;
        }
        let next = !self.duration_show_milliseconds(base);
        self.patch_active_theme(base, |theme| {
            theme.segments.duration.show_milliseconds = next;
        });
    }

    /// Toggle whether the git segment renders the branch name, via
    /// `patch_active_theme` (#407) — see `cycle_clock_time_format`. The git
    /// content-visibility toggles (branch/ahead-behind/stash) together form
    /// the "which git info to show" multi-select (#410); each is an
    /// independent `Option<bool>` on `GitTheme` defaulting to on when unset.
    ///
    /// A no-op unless `Section::Segments`'s cursor is on `"git"` (see
    /// `focused_builtin`) — mirrors `toggle_clock_show_seconds`'s
    /// guard-inside-the-transition pattern.
    pub(crate) fn toggle_git_show_branch(&mut self, base: &ThemeConfig) {
        if self.focused_builtin() != Some(crate::plugin::BuiltinSegment::Git) {
            return;
        }
        let next = !self.git_show_branch(base);
        self.patch_active_theme(base, |theme| {
            theme.segments.git.show_branch = Some(next);
        });
    }

    /// Toggle whether the git segment renders the ahead/behind arrows, via
    /// `patch_active_theme` (#407) — see `toggle_git_show_branch`.
    ///
    /// A no-op unless `Section::Segments`'s cursor is on `"git"` (see
    /// `focused_builtin`).
    pub(crate) fn toggle_git_show_ahead_behind(&mut self, base: &ThemeConfig) {
        if self.focused_builtin() != Some(crate::plugin::BuiltinSegment::Git) {
            return;
        }
        let next = !self.git_show_ahead_behind(base);
        self.patch_active_theme(base, |theme| {
            theme.segments.git.show_ahead_behind = Some(next);
        });
    }

    /// Toggle whether the git segment renders the stash indicator, via
    /// `patch_active_theme` (#407) — see `toggle_git_show_branch`.
    ///
    /// A no-op unless `Section::Segments`'s cursor is on `"git"` (see
    /// `focused_builtin`).
    pub(crate) fn toggle_git_show_stash(&mut self, base: &ThemeConfig) {
        if self.focused_builtin() != Some(crate::plugin::BuiltinSegment::Git) {
            return;
        }
        let next = !self.git_show_stash(base);
        self.patch_active_theme(base, |theme| {
            theme.segments.git.show_stash = Some(next);
        });
    }

    pub(crate) const fn next_section(&mut self) {
        self.focus = self.focus.next();
    }

    pub(crate) const fn prev_section(&mut self) {
        self.focus = self.focus.prev();
    }

    /// Move the focused section's highlight-cursor to the previous entry in
    /// its *filtered* list (see `filtered_indices`), clamped at the first
    /// entry (a no-op past the top, not a wrap) — clamping is less
    /// surprising than wrapping for a linear list UI, and keeps cursor
    /// movement independent of Tab's separate section-wrapping behavior. A
    /// no-op when the filtered list is empty (the no-match case) — never
    /// panics.
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

    /// Move the focused section's highlight-cursor to the next entry in its
    /// *filtered* list, clamped at the last entry. A no-op when the
    /// filtered list is empty. See `move_cursor_up`.
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

    /// Apply the focused section's highlight-cursor to the underlying
    /// selection: selects the theme/palette at the cursor, or toggles the
    /// segment at the cursor.
    /// A no-op when the focused section's filtered list is empty (the
    /// no-match case) — mirrors the same guard on `move_cursor_up`/
    /// `move_cursor_down`, since activating a cursor that isn't pointing at
    /// any currently-visible (filtered-in) item would silently
    /// select/toggle something the user can't see.
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

    /// Append `c` to the shared filter query. Every section's filtered list
    /// may narrow as a result, so each section's cursor is independently
    /// snapped to its own filtered list's top entry if its previously
    /// highlighted item no longer matches — mirrors fzf's "narrowing jumps
    /// to the best result" feel, applied per section since Theme/Palette/
    /// Segments each have their own cursor into their own list.
    pub(crate) fn push_filter_char(&mut self, c: char) {
        self.filter_query.push(c);
        self.snap_all_cursors_if_filtered_out();
    }

    /// Remove the last character from the shared filter query. Never needs
    /// to snap any cursor: if the longer query matched a section's
    /// highlighted item as a subsequence, every prefix of that query (i.e.
    /// every shorter query reachable by backspacing) matches it too, using
    /// the same match positions minus the trailing one.
    pub(crate) fn pop_filter_char(&mut self) {
        self.filter_query.pop();
    }

    /// Clear the shared filter query, restoring every section's full list.
    /// Every cursor already holds a valid raw index into its list (the
    /// invariant every cursor mutation preserves), so each previously
    /// highlighted item stays highlighted without further bookkeeping.
    pub(crate) fn clear_filter(&mut self) {
        self.filter_query.clear();
    }

    /// For each of the three sections, if that section's cursor no longer
    /// appears in that section's filtered list (per the shared query), snap
    /// it to that list's first (best-ranked) entry. A no-op for any section
    /// whose filtered list is empty. Runs for all three sections (not just
    /// the focused one) because the shared query can narrow a
    /// currently-unfocused section too — e.g. typing while Theme is
    /// focused still narrows Palette/Segments, and their cursors should
    /// already be sensible by the time the user tabs to them.
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

    /// Mutable reference to whichever cursor field belongs to `section`.
    const fn cursor_mut(&mut self, section: Section) -> &mut usize {
        match section {
            Section::Theme => &mut self.theme_cursor,
            Section::Palette => &mut self.palette_cursor,
            Section::Segments => &mut self.segment_cursor,
        }
    }

    /// Mutable reference to whichever cursor field belongs to `self.focus`.
    const fn focused_cursor_mut(&mut self) -> &mut usize {
        self.cursor_mut(self.focus)
    }

    /// Whether the current selection differs from what was loaded at startup.
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

    /// Apply the current selection onto `config`.
    ///
    /// Edits the list order-preservingly (new names are appended), like
    /// `gpy enable`/`gpy disable`, so a save through the wizard produces the
    /// same shape of config a sequence of `gpy enable`/`gpy disable`/`gpy
    /// theme use`/`gpy palette use` calls would: a selected `git`/`language`
    /// is listed with its flag set, and a deselected one has its flag cleared
    /// while an existing list entry is kept (like `gpy disable`).
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

        // Edit ui.enabled_segments in place so the existing order survives:
        // drop offered names the user deselected (a listed git/language entry
        // stays, like `gpy disable`), keep every name the wizard does not
        // offer, then append newly selected names in `available_segments`
        // order.
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

/// Cycle a `DirectoryDisplay` forward, wrapping from `Full` back to
/// `Basename`. Mirrors `Section::next`'s wrapping-match style.
const fn next_directory_display(current: DirectoryDisplay) -> DirectoryDisplay {
    match current {
        DirectoryDisplay::Basename => DirectoryDisplay::Abbreviated,
        DirectoryDisplay::Abbreviated => DirectoryDisplay::Truncated,
        DirectoryDisplay::Truncated => DirectoryDisplay::Full,
        DirectoryDisplay::Full => DirectoryDisplay::Basename,
    }
}

/// Cycle a `DirectoryDisplay` backward, wrapping from `Basename` back to
/// `Full`. Mirrors `Section::prev`'s wrapping-match style.
const fn prev_directory_display(current: DirectoryDisplay) -> DirectoryDisplay {
    match current {
        DirectoryDisplay::Basename => DirectoryDisplay::Full,
        DirectoryDisplay::Abbreviated => DirectoryDisplay::Basename,
        DirectoryDisplay::Truncated => DirectoryDisplay::Abbreviated,
        DirectoryDisplay::Full => DirectoryDisplay::Truncated,
    }
}

/// Cycle a `LanguageDisplay` forward.
///
/// Only 2 variants exist, so this simply toggles — kept as a named cycling fn (rather than
/// inlining the toggle) to mirror `next_directory_display`'s shape for the shared `[`/`]`
/// handler.
const fn next_language_display(current: LanguageDisplay) -> LanguageDisplay {
    match current {
        LanguageDisplay::Icon => LanguageDisplay::Text,
        LanguageDisplay::Text => LanguageDisplay::Icon,
    }
}

/// Cycle a `LanguageDisplay` backward — identical to `next_language_display`
/// since there are only 2 variants (toggling either direction lands on the
/// other one).
const fn prev_language_display(current: LanguageDisplay) -> LanguageDisplay {
    next_language_display(current)
}

/// Compute the starting enabled-segment set from `config`.
///
/// Uses `commands::segments::is_effectively_enabled`, the same rule
/// `gpy segments` and the prompt export apply, so the wizard starts from
/// what the prompt actually renders (`git`/`language` need both list
/// membership and their feature flag).
fn initial_enabled_segments(config: &Config, available: &[String]) -> BTreeSet<String> {
    available
        .iter()
        .filter(|segment| crate::commands::segments::is_effectively_enabled(config, segment))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::missing_panics_doc
    )]

    use super::*;
    use crate::config::types::{PaletteName, ThemeName};

    /// Fixed, deterministic stand-ins for the real discovery functions.
    ///
    /// Matches `ThemeManager::discover_available_themes()` /
    /// `PaletteManager::discover_available_palettes()` /
    /// `commands::segments::available_segments()` in a clean environment (no
    /// user-installed themes/palettes/plugins under `$HOME/.config/gpy/`).
    ///
    /// Wizard unit tests build `WizardState` via [`test_state`], which feeds
    /// these into `WizardState::from_parts` instead of calling `WizardState::new`
    /// (which performs real filesystem discovery) — this is what makes the
    /// tests deterministic regardless of what's actually on disk (#607).
    const FIXED_THEMES: [&str; 3] = ["default", "starship", "text"];
    const FIXED_PALETTES: [&str; 8] = [
        "catppuccin-frappe",
        "catppuccin-latte",
        "catppuccin-macchiato",
        "catppuccin-mocha",
        "default",
        "gruvbox-dark-medium",
        "nord",
        "starship",
    ];
    const FIXED_SEGMENTS: [&str; 6] = [
        "clock",
        "duration",
        "language",
        "directory",
        "git",
        "status",
    ];

    /// Build a `WizardState` from `config` via `from_parts` and the fixed
    /// lists above — the shared entry point every test in this module uses
    /// instead of `WizardState::new` (see `FIXED_THEMES`'s doc comment).
    fn test_state(config: Config) -> WizardState {
        WizardState::from_parts(
            config,
            FIXED_THEMES.iter().map(|&s| s.to_owned()).collect(),
            FIXED_PALETTES.iter().map(|&s| s.to_owned()).collect(),
            FIXED_SEGMENTS.iter().map(|&s| s.to_owned()).collect(),
        )
    }

    /// Bundles the git/language toggles behind one param — `.clippy.toml`
    /// caps functions at 1 bool parameter (`max-fn-params-bools`).
    #[derive(Clone, Copy)]
    struct GitLanguageFlags {
        git: bool,
        language: bool,
    }

    /// Build a `Config` with known theme/palette/segment settings for tests.
    ///
    /// `enabled_segments` deliberately excludes `"git"`/`"language"` unless
    /// the caller explicitly adds them, since those two are driven by
    /// `git.enabled`/`language.enabled` instead of list membership.
    fn make_config(
        theme: &str,
        palette: &str,
        flags: GitLanguageFlags,
        enabled_segments: &[&str],
    ) -> Config {
        let mut config = Config::default();
        config.ui.theme = ThemeName::new(theme.to_owned()).expect("valid theme name");
        config.ui.palette = PaletteName::new(palette.to_owned()).expect("valid palette name");
        config.git.enabled = flags.git;
        config.language.enabled = flags.language;
        config.ui.enabled_segments = enabled_segments.iter().map(|s| (*s).to_owned()).collect();
        config
    }

    #[test]
    fn new_reflects_config_theme_and_palette() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &["directory"],
        );
        let state = test_state(config);

        assert_eq!(state.selected_theme(), "default");
        assert_eq!(state.selected_palette(), "default");
    }

    #[test]
    fn initial_theme_and_palette_reflect_startup_config() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &["directory"],
        );
        let state = test_state(config);

        assert_eq!(state.initial_theme(), "default");
        assert_eq!(state.initial_palette(), "default");
    }

    #[test]
    fn initial_theme_stays_fixed_after_selection_changes() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &[],
        );
        let mut state = test_state(config);

        let Some(other) = state
            .available_themes()
            .iter()
            .find(|name| name.as_str() != "default")
            .cloned()
        else {
            // Nothing to switch to in this environment; nothing to assert.
            return;
        };

        state.select_theme(&other);

        assert_eq!(state.initial_theme(), "default");
        assert_eq!(state.selected_theme(), other);
    }

    #[test]
    fn new_reflects_flag_and_list_membership_for_git_and_language() {
        // language's flag is on but it is absent from enabled_segments, so the
        // prompt does not render it: the wizard must start with it unchecked.
        // git is listed but its flag is off: also unchecked.
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: false,
                language: true,
            },
            &["directory", "git"],
        );
        let state = test_state(config);

        assert!(!state.is_segment_enabled("git"));
        assert!(!state.is_segment_enabled("language"));
        assert!(state.is_segment_enabled("directory"));

        let both = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &["language", "directory", "git"],
        );
        let both_state = test_state(both);
        assert!(both_state.is_segment_enabled("git"));
        assert!(both_state.is_segment_enabled("language"));
    }

    #[test]
    fn select_theme_ignores_unknown_name() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &[],
        );
        let mut state = test_state(config);

        state.select_theme("does-not-exist");

        assert_eq!(state.selected_theme(), "default");
    }

    #[test]
    fn select_theme_updates_known_name() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &[],
        );
        let mut state = test_state(config);

        let maybe_other = state
            .available_themes()
            .iter()
            .find(|name| name.as_str() != "default")
            .cloned();

        let Some(other) = maybe_other else {
            // Only one built-in theme discovered in this environment;
            // nothing to switch to, so verify the no-op path instead.
            state.select_theme("default");
            assert_eq!(state.selected_theme(), "default");
            return;
        };

        state.select_theme(&other);

        assert_eq!(state.selected_theme(), other);
    }

    #[test]
    fn toggle_segment_is_idempotent_pair() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &["directory"],
        );
        let mut state = test_state(config);

        let original = state.is_segment_enabled("directory");
        state.toggle_segment("directory");
        assert_ne!(state.is_segment_enabled("directory"), original);
        state.toggle_segment("directory");
        assert_eq!(state.is_segment_enabled("directory"), original);

        // Unknown segment name is a no-op.
        state.toggle_segment("not-a-real-segment");
        assert!(!state.is_segment_enabled("not-a-real-segment"));
    }

    #[test]
    fn dirty_false_immediately_after_new() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &["directory"],
        );
        let state = test_state(config);

        assert!(!state.dirty());
    }

    #[test]
    fn dirty_true_after_theme_change() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &[],
        );
        let mut state = test_state(config);

        let Some(other) = state
            .available_themes()
            .iter()
            .find(|name| name.as_str() != "default")
            .cloned()
        else {
            // Nothing to switch to in this environment; nothing to assert.
            return;
        };

        state.select_theme(&other);

        assert!(state.dirty());
    }

    #[test]
    fn dirty_true_after_palette_change() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &[],
        );
        let mut state = test_state(config);

        let Some(other) = state
            .available_palettes()
            .iter()
            .find(|name| name.as_str() != "default")
            .cloned()
        else {
            // Nothing to switch to in this environment; nothing to assert.
            return;
        };

        state.select_palette(&other);

        assert!(state.dirty());
    }

    #[test]
    fn dirty_true_after_segment_toggle() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &["directory"],
        );
        let mut state = test_state(config);

        state.toggle_segment("directory");

        assert!(state.dirty());
    }

    #[test]
    fn dirty_false_after_toggle_and_untoggle() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &["directory"],
        );
        let mut state = test_state(config);

        state.toggle_segment("directory");
        state.toggle_segment("directory");

        assert!(!state.dirty());
    }

    #[test]
    fn apply_to_updates_theme_palette_and_git_language_flags() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: false,
                language: false,
            },
            &["directory"],
        );
        let mut state = test_state(config);

        // Theme/palette: only "default" is guaranteed to be discovered in
        // every environment (see select_theme_updates_known_name's fallback
        // above), so exercise the no-op-reselect path plus the toggles,
        // which don't depend on a second built-in theme/palette existing.
        state.select_theme("default");
        state.select_palette("default");
        state.toggle_segment("git");
        state.toggle_segment("language");

        let mut applied = Config::default();
        state.apply_to(&mut applied);

        assert_eq!(applied.ui.theme.as_str(), "default");
        assert_eq!(applied.ui.palette.to_string(), "default");
        assert!(applied.git.enabled);
        assert!(applied.language.enabled);
        assert!(
            applied
                .ui
                .enabled_segments
                .contains(&"directory".to_owned())
        );
        // A selected git/language is listed (the export needs both the list
        // entry and the flag), in BUILTIN_ORDER.
        assert_eq!(
            applied.ui.enabled_segments,
            vec!["language", "directory", "git"]
        );
    }

    #[test]
    fn apply_to_keeps_default_git_and_language_in_enabled_segments() {
        // #692: a wizard save with no changes must not drop git/language from
        // the rendered prompt.
        let state = test_state(Config::default());
        let mut applied = Config::default();
        state.apply_to(&mut applied);

        assert_eq!(
            applied.ui.enabled_segments,
            Config::default().ui.enabled_segments
        );
        assert!(applied.git.enabled);
        assert!(applied.language.enabled);
    }

    #[test]
    fn apply_to_deselected_git_clears_flag_and_keeps_list_entry() {
        // Same config shape as `gpy disable git`: flag off, entry kept so a
        // later enable restores its position.
        let mut state = test_state(Config::default());
        state.toggle_segment("git");
        let mut applied = Config::default();
        state.apply_to(&mut applied);

        assert!(!applied.git.enabled);
        assert_eq!(
            applied.ui.enabled_segments,
            Config::default().ui.enabled_segments
        );
    }

    /// Move the Segments cursor onto `"directory"` — the only place
    /// `cycle_directory_display` is reachable from (see
    /// `focused_builtin`).
    fn focus_directory_segment(state: &mut WizardState) {
        state.next_section(); // Theme -> Palette
        state.next_section(); // Palette -> Segments
        while state
            .available_segments()
            .get(state.segment_cursor())
            .map(String::as_str)
            != Some("directory")
        {
            state.move_cursor_down();
        }
    }

    #[test]
    fn directory_display_defaults_to_the_config_value() {
        let mut config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &["directory"],
        );
        config.ui.directory.display = crate::config::types::DirectoryDisplay::Truncated;
        let state = test_state(config);

        assert_eq!(
            state.directory_display(),
            crate::config::types::DirectoryDisplay::Truncated
        );
    }

    #[test]
    fn cycle_directory_display_forward_wraps_through_all_variants() {
        use crate::config::types::DirectoryDisplay;

        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &["directory"],
        );
        let mut state = test_state(config);
        focus_directory_segment(&mut state);
        assert_eq!(state.directory_display(), DirectoryDisplay::Basename);

        state.cycle_directory_display(true);
        assert_eq!(state.directory_display(), DirectoryDisplay::Abbreviated);
        state.cycle_directory_display(true);
        assert_eq!(state.directory_display(), DirectoryDisplay::Truncated);
        state.cycle_directory_display(true);
        assert_eq!(state.directory_display(), DirectoryDisplay::Full);
        state.cycle_directory_display(true);
        assert_eq!(
            state.directory_display(),
            DirectoryDisplay::Basename,
            "cycling forward past Full should wrap back to Basename"
        );
    }

    #[test]
    fn cycle_directory_display_backward_wraps_from_basename_to_full() {
        use crate::config::types::DirectoryDisplay;

        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &["directory"],
        );
        let mut state = test_state(config);
        focus_directory_segment(&mut state);

        state.cycle_directory_display(false);

        assert_eq!(state.directory_display(), DirectoryDisplay::Full);
    }

    #[test]
    fn cycle_directory_display_is_noop_unless_directory_segment_is_focused() {
        use crate::config::types::DirectoryDisplay;

        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &["directory"],
        );
        let mut state = test_state(config);
        // Still on Section::Theme (the default focus) — not Segments.
        assert_eq!(state.focused_builtin(), None);

        state.cycle_directory_display(true);

        assert_eq!(state.directory_display(), DirectoryDisplay::Basename);
    }

    #[test]
    fn dirty_true_after_directory_display_cycle() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &["directory"],
        );
        let mut state = test_state(config);
        focus_directory_segment(&mut state);

        state.cycle_directory_display(true);

        assert!(state.dirty());
    }

    #[test]
    fn apply_to_updates_directory_display() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &["directory"],
        );
        let mut state = test_state(config);
        focus_directory_segment(&mut state);
        state.cycle_directory_display(true);

        let mut applied = Config::default();
        state.apply_to(&mut applied);

        assert_eq!(
            applied.ui.directory.display,
            crate::config::types::DirectoryDisplay::Abbreviated
        );
    }

    /// Move the Segments cursor onto `"language"` — the only place
    /// `cycle_language_display`/`toggle_language_show_versions` are
    /// reachable from (see `focused_builtin`).
    fn focus_language_segment(state: &mut WizardState) {
        state.next_section(); // Theme -> Palette
        state.next_section(); // Palette -> Segments
        while state
            .available_segments()
            .get(state.segment_cursor())
            .map(String::as_str)
            != Some("language")
        {
            state.move_cursor_down();
        }
    }

    #[test]
    fn language_display_defaults_to_the_config_value() {
        let mut config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &["directory"],
        );
        config.language.display = crate::config::types::LanguageDisplay::Text;
        let state = test_state(config);

        assert_eq!(
            state.language_display(),
            crate::config::types::LanguageDisplay::Text
        );
    }

    #[test]
    fn cycle_language_display_toggles_between_icon_and_text() {
        use crate::config::types::LanguageDisplay;

        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &["directory"],
        );
        let mut state = test_state(config);
        focus_language_segment(&mut state);
        assert_eq!(state.language_display(), LanguageDisplay::Icon);

        state.cycle_language_display(true);
        assert_eq!(state.language_display(), LanguageDisplay::Text);
        state.cycle_language_display(true);
        assert_eq!(state.language_display(), LanguageDisplay::Icon);

        state.cycle_language_display(false);
        assert_eq!(state.language_display(), LanguageDisplay::Text);
    }

    #[test]
    fn cycle_language_display_is_noop_unless_language_segment_is_focused() {
        use crate::config::types::LanguageDisplay;

        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &["directory"],
        );
        let mut state = test_state(config);
        // Still on Section::Theme (the default focus) — not Segments.
        assert_eq!(state.focused_builtin(), None);

        state.cycle_language_display(true);

        assert_eq!(state.language_display(), LanguageDisplay::Icon);
    }

    #[test]
    fn dirty_true_after_language_display_cycle() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &["directory"],
        );
        let mut state = test_state(config);
        focus_language_segment(&mut state);

        state.cycle_language_display(true);

        assert!(state.dirty());
    }

    #[test]
    fn apply_to_updates_language_display() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &["directory"],
        );
        let mut state = test_state(config);
        focus_language_segment(&mut state);
        state.cycle_language_display(true);

        let mut applied = Config::default();
        state.apply_to(&mut applied);

        assert_eq!(
            applied.language.display,
            crate::config::types::LanguageDisplay::Text
        );
    }

    #[test]
    fn toggle_language_show_versions_flips_the_flag() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &["directory"],
        );
        let mut state = test_state(config);
        focus_language_segment(&mut state);
        let before = state.language_show_versions();

        state.toggle_language_show_versions();
        assert_eq!(state.language_show_versions(), !before);

        state.toggle_language_show_versions();
        assert_eq!(state.language_show_versions(), before);
    }

    #[test]
    fn toggle_language_show_versions_is_noop_unless_language_segment_is_focused() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &["directory"],
        );
        let mut state = test_state(config);
        let before = state.language_show_versions();
        assert_eq!(state.focused_builtin(), None);

        state.toggle_language_show_versions();

        assert_eq!(state.language_show_versions(), before);
    }

    #[test]
    fn dirty_true_after_language_show_versions_toggle() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &["directory"],
        );
        let mut state = test_state(config);
        focus_language_segment(&mut state);

        state.toggle_language_show_versions();

        assert!(state.dirty());
    }

    #[test]
    fn apply_to_updates_language_show_versions() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &["directory"],
        );
        let mut state = test_state(config);
        focus_language_segment(&mut state);
        let before = state.language_show_versions();
        state.toggle_language_show_versions();

        let mut applied = Config::default();
        state.apply_to(&mut applied);

        assert_eq!(applied.language.show_versions, !before);
    }

    /// Move `state`'s cursor onto the `"clock"` segment within
    /// `Section::Segments`.
    fn focus_clock_segment(state: &mut WizardState) {
        state.next_section(); // Theme -> Palette
        state.next_section(); // Palette -> Segments
        while state
            .available_segments()
            .get(state.segment_cursor())
            .map(String::as_str)
            != Some("clock")
        {
            state.move_cursor_down();
        }
    }

    #[test]
    fn clock_time_format_and_show_seconds_default_when_unset_in_base() {
        let state = clean_state();
        let base = ThemeConfig::default();

        assert_eq!(state.clock_time_format(&base), "12");
        assert!(!state.clock_show_seconds(&base));
    }

    #[test]
    fn cycle_clock_time_format_toggles_between_12_and_24() {
        let mut state = clean_state();
        let base = ThemeConfig::default();
        focus_clock_segment(&mut state);
        assert_eq!(state.clock_time_format(&base), "12");

        state.cycle_clock_time_format(&base);
        assert_eq!(state.clock_time_format(&base), "24");

        state.cycle_clock_time_format(&base);
        assert_eq!(state.clock_time_format(&base), "12");
    }

    #[test]
    fn cycle_clock_time_format_is_noop_unless_clock_segment_is_focused() {
        let mut state = clean_state();
        let base = ThemeConfig::default();
        assert_eq!(state.focus(), Section::Theme);

        state.cycle_clock_time_format(&base);

        assert_eq!(state.clock_time_format(&base), "12");
        assert!(state.pending_theme().is_none());
    }

    #[test]
    fn dirty_true_after_clock_time_format_cycle() {
        let mut state = clean_state();
        let base = ThemeConfig::default();
        focus_clock_segment(&mut state);
        assert!(!state.dirty());

        state.cycle_clock_time_format(&base);

        assert!(state.dirty());
    }

    #[test]
    fn toggle_clock_show_seconds_flips_the_flag() {
        let mut state = clean_state();
        let base = ThemeConfig::default();
        focus_clock_segment(&mut state);
        let before = state.clock_show_seconds(&base);

        state.toggle_clock_show_seconds(&base);

        assert_eq!(state.clock_show_seconds(&base), !before);
    }

    #[test]
    fn toggle_clock_show_seconds_is_noop_unless_clock_segment_is_focused() {
        let mut state = clean_state();
        let base = ThemeConfig::default();
        let before = state.clock_show_seconds(&base);
        assert_eq!(state.focus(), Section::Theme);

        state.toggle_clock_show_seconds(&base);

        assert_eq!(state.clock_show_seconds(&base), before);
        assert!(state.pending_theme().is_none());
    }

    #[test]
    fn dirty_true_after_clock_show_seconds_toggle() {
        let mut state = clean_state();
        let base = ThemeConfig::default();
        focus_clock_segment(&mut state);
        assert!(!state.dirty());

        state.toggle_clock_show_seconds(&base);

        assert!(state.dirty());
    }

    #[test]
    fn clock_edits_accumulate_in_pending_theme_without_clobbering_each_other() {
        let mut state = clean_state();
        let base = ThemeConfig::default();
        focus_clock_segment(&mut state);

        state.cycle_clock_time_format(&base);
        state.toggle_clock_show_seconds(&base);

        let pending = state.pending_theme().expect("pending theme present");
        assert_eq!(pending.segments.clock.time_format.as_deref(), Some("24"));
        assert_eq!(pending.segments.clock.show_seconds, Some(true));
    }

    /// Move `state`'s cursor onto the `"duration"` segment within
    /// `Section::Segments`.
    fn focus_duration_segment(state: &mut WizardState) {
        state.next_section(); // Theme -> Palette
        state.next_section(); // Palette -> Segments
        while state
            .available_segments()
            .get(state.segment_cursor())
            .map(String::as_str)
            != Some("duration")
        {
            state.move_cursor_down();
        }
    }

    #[test]
    fn duration_show_milliseconds_defaults_true_when_unset_in_base() {
        let state = clean_state();
        let base = ThemeConfig::default();

        assert!(state.duration_show_milliseconds(&base));
    }

    #[test]
    fn toggle_duration_show_milliseconds_flips_the_flag() {
        let mut state = clean_state();
        let base = ThemeConfig::default();
        focus_duration_segment(&mut state);
        let before = state.duration_show_milliseconds(&base);

        state.toggle_duration_show_milliseconds(&base);

        assert_eq!(state.duration_show_milliseconds(&base), !before);
    }

    #[test]
    fn toggle_duration_show_milliseconds_is_noop_unless_duration_segment_is_focused() {
        let mut state = clean_state();
        let base = ThemeConfig::default();
        assert_eq!(state.focus(), Section::Theme);

        state.toggle_duration_show_milliseconds(&base);

        assert!(state.duration_show_milliseconds(&base));
        assert!(state.pending_theme().is_none());
    }

    #[test]
    fn dirty_true_after_duration_show_milliseconds_toggle() {
        let mut state = clean_state();
        let base = ThemeConfig::default();
        focus_duration_segment(&mut state);
        assert!(!state.dirty());

        state.toggle_duration_show_milliseconds(&base);

        assert!(state.dirty());
    }

    /// Move `state`'s cursor onto the `"git"` segment within
    /// `Section::Segments`.
    fn focus_git_segment(state: &mut WizardState) {
        state.next_section(); // Theme -> Palette
        state.next_section(); // Palette -> Segments
        while state
            .available_segments()
            .get(state.segment_cursor())
            .map(String::as_str)
            != Some("git")
        {
            state.move_cursor_down();
        }
    }

    #[test]
    fn git_content_flags_default_true_when_unset_in_base() {
        let state = clean_state();
        let base = ThemeConfig::default();

        assert!(state.git_show_branch(&base));
        assert!(state.git_show_ahead_behind(&base));
        assert!(state.git_show_stash(&base));
    }

    #[test]
    fn toggle_git_show_branch_flips_only_the_branch_flag() {
        let mut state = clean_state();
        let base = ThemeConfig::default();
        focus_git_segment(&mut state);

        state.toggle_git_show_branch(&base);

        assert!(!state.git_show_branch(&base));
        // The other two content flags are untouched.
        assert!(state.git_show_ahead_behind(&base));
        assert!(state.git_show_stash(&base));
    }

    #[test]
    fn toggle_git_show_ahead_behind_flips_the_flag() {
        let mut state = clean_state();
        let base = ThemeConfig::default();
        focus_git_segment(&mut state);

        state.toggle_git_show_ahead_behind(&base);

        assert!(!state.git_show_ahead_behind(&base));
    }

    #[test]
    fn toggle_git_show_stash_flips_the_flag() {
        let mut state = clean_state();
        let base = ThemeConfig::default();
        focus_git_segment(&mut state);

        state.toggle_git_show_stash(&base);

        assert!(!state.git_show_stash(&base));
    }

    #[test]
    fn git_toggles_are_noop_unless_git_segment_is_focused() {
        let mut state = clean_state();
        let base = ThemeConfig::default();
        assert_eq!(state.focus(), Section::Theme);

        state.toggle_git_show_branch(&base);
        state.toggle_git_show_ahead_behind(&base);
        state.toggle_git_show_stash(&base);

        assert!(state.git_show_branch(&base));
        assert!(state.git_show_ahead_behind(&base));
        assert!(state.git_show_stash(&base));
        assert!(state.pending_theme().is_none());
    }

    #[test]
    fn dirty_true_after_git_content_toggle() {
        let mut state = clean_state();
        let base = ThemeConfig::default();
        focus_git_segment(&mut state);
        assert!(!state.dirty());

        state.toggle_git_show_stash(&base);

        assert!(state.dirty());
    }

    /// A fresh state with a `default`/`default` config and no segment edits.
    fn clean_state() -> WizardState {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &["directory"],
        );
        test_state(config)
    }

    #[test]
    fn patch_active_theme_seeds_pending_and_marks_dirty() {
        let mut state = clean_state();
        assert!(!state.dirty(), "state should start clean");
        assert!(state.pending_theme().is_none());

        let base = ThemeConfig::default();
        state.patch_active_theme(&base, |theme| {
            theme.segments.clock.time_format = Some("24".to_owned());
        });

        assert!(
            state.dirty(),
            "a pending theme edit must mark the state dirty"
        );
        let pending = state
            .pending_theme()
            .expect("pending theme present after patch");
        assert_eq!(pending.segments.clock.time_format.as_deref(), Some("24"));
    }

    #[test]
    fn patch_active_theme_accumulates_successive_edits() {
        let mut state = clean_state();
        let base = ThemeConfig::default();

        state.patch_active_theme(&base, |theme| {
            theme.segments.clock.time_format = Some("24".to_owned());
        });
        state.patch_active_theme(&base, |theme| {
            theme.segments.duration.show_milliseconds = false;
        });

        let pending = state.pending_theme().expect("pending theme present");
        // Both edits survive — the second patch builds on the first, it does
        // not reset back to `base`.
        assert_eq!(pending.segments.clock.time_format.as_deref(), Some("24"));
        assert!(!pending.segments.duration.show_milliseconds);
    }

    #[test]
    fn patch_active_theme_consults_base_only_for_the_first_edit() {
        let mut state = clean_state();

        // First edit seeds pending from `first_base` (which carries a distinctive
        // unrelated field value).
        let mut first_base = ThemeConfig::default();
        first_base.segments.clock.show_leading_zero = Some(true);
        state.patch_active_theme(&first_base, |theme| {
            theme.segments.clock.time_format = Some("24".to_owned());
        });

        // Second edit passes a *different* base; it must be ignored because
        // pending already exists, so the first base's field is preserved.
        let mut second_base = ThemeConfig::default();
        second_base.segments.clock.show_leading_zero = Some(false);
        state.patch_active_theme(&second_base, |theme| {
            theme.segments.duration.show_milliseconds = false;
        });

        let pending = state.pending_theme().expect("pending theme present");
        assert_eq!(
            pending.segments.clock.show_leading_zero,
            Some(true),
            "second call's base must not overwrite the accumulated copy"
        );
    }

    #[test]
    fn select_theme_change_discards_pending_theme_edits() {
        let mut state = clean_state();
        let base = ThemeConfig::default();
        state.patch_active_theme(&base, |theme| {
            theme.segments.clock.time_format = Some("24".to_owned());
        });
        assert!(state.pending_theme().is_some());

        // "text" is always a discoverable builtin, so this selection always
        // changes the theme away from "default".
        state.select_theme("text");

        assert!(
            state.pending_theme().is_none(),
            "switching themes must drop edits that described the previous theme"
        );
    }

    #[test]
    fn select_theme_same_name_keeps_pending_theme_edits() {
        let mut state = clean_state();
        let base = ThemeConfig::default();
        state.patch_active_theme(&base, |theme| {
            theme.segments.clock.time_format = Some("24".to_owned());
        });

        // Re-selecting the already-selected theme is not a theme change and
        // must not discard in-progress edits.
        state.select_theme("default");

        assert_eq!(
            state
                .pending_theme()
                .and_then(|theme| theme.segments.clock.time_format.as_deref()),
            Some("24")
        );
    }

    #[test]
    fn apply_to_preserves_builtin_order_in_enabled_segments() {
        // Start with nothing enabled, then toggle builtin segments on in an
        // order that's scrambled relative to BUILTIN_ORDER
        // ("clock", "duration", "language", "directory", "git", "status").
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: false,
                language: false,
            },
            &[],
        );
        let mut state = test_state(config.clone());

        for segment in ["status", "clock", "directory", "duration"] {
            state.toggle_segment(segment);
        }

        // Apply onto the config the wizard started from, as `save_to` does: a
        // deselected git/language keeps any existing list entry (#692), so a
        // `Config::default()` target would legitimately retain them.
        let mut applied = config;
        state.apply_to(&mut applied);

        assert_eq!(
            applied.ui.enabled_segments,
            vec![
                "clock".to_owned(),
                "duration".to_owned(),
                "directory".to_owned(),
                "status".to_owned(),
            ],
            "enabled_segments must follow BUILTIN_ORDER, not toggle order"
        );
    }

    #[test]
    fn apply_to_preserves_existing_order_and_unoffered_entries() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: false,
                language: false,
            },
            &["directory", "clock", "removed-plugin-seg"],
        );
        let state = test_state(config.clone());

        let mut applied = config.clone();
        state.apply_to(&mut applied);

        assert_eq!(applied.ui.enabled_segments, config.ui.enabled_segments);
    }

    #[test]
    fn section_next_wraps_theme_palette_segments_theme() {
        assert_eq!(Section::Theme.next(), Section::Palette);
        assert_eq!(Section::Palette.next(), Section::Segments);
        assert_eq!(Section::Segments.next(), Section::Theme);
    }

    #[test]
    fn section_prev_wraps_the_other_direction() {
        assert_eq!(Section::Theme.prev(), Section::Segments);
        assert_eq!(Section::Segments.prev(), Section::Palette);
        assert_eq!(Section::Palette.prev(), Section::Theme);
    }

    // --- cursor tests ---

    #[test]
    fn theme_cursor_starts_at_selected_theme_index() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &[],
        );
        let state = test_state(config);

        let expected = state
            .available_themes()
            .iter()
            .position(|t| t == "default")
            .expect("default theme should be discoverable");

        assert_eq!(state.theme_cursor(), expected);
    }

    #[test]
    fn palette_cursor_starts_at_selected_palette_index() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &[],
        );
        let state = test_state(config);

        let expected = state
            .available_palettes()
            .iter()
            .position(|p| p == "default")
            .expect("default palette should be discoverable");

        assert_eq!(state.palette_cursor(), expected);
    }

    #[test]
    fn move_cursor_down_advances_within_focused_section() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &[],
        );
        let mut state = test_state(config);
        state.next_section(); // Theme -> Palette
        state.next_section(); // Palette -> Segments
        assert_eq!(state.focus(), Section::Segments);
        assert!(
            state.available_segments().len() >= 2,
            "test relies on at least two builtin segments existing"
        );

        let start = state.segment_cursor();
        state.move_cursor_down();

        assert_eq!(state.segment_cursor(), start + 1);
    }

    #[test]
    fn move_cursor_up_retreats_within_focused_section() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &[],
        );
        let mut state = test_state(config);
        state.next_section();
        state.next_section();
        assert_eq!(state.focus(), Section::Segments);

        state.move_cursor_down();
        state.move_cursor_down();
        let after_two_down = state.segment_cursor();
        state.move_cursor_up();

        assert_eq!(state.segment_cursor(), after_two_down - 1);
    }

    #[test]
    fn move_cursor_down_clamps_at_list_end() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &[],
        );
        let mut state = test_state(config);
        state.next_section();
        state.next_section();
        assert_eq!(state.focus(), Section::Segments);

        let len = state.available_segments().len();
        for _ in 0..len + 5 {
            state.move_cursor_down();
        }

        assert_eq!(state.segment_cursor(), len - 1);
    }

    #[test]
    fn move_cursor_up_clamps_at_zero() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &[],
        );
        let mut state = test_state(config);
        state.next_section();
        state.next_section();
        assert_eq!(state.focus(), Section::Segments);
        assert_eq!(state.segment_cursor(), 0);

        state.move_cursor_up();

        assert_eq!(state.segment_cursor(), 0);
    }

    #[test]
    fn move_cursor_only_affects_focused_section() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &[],
        );
        let mut state = test_state(config);
        assert_eq!(state.focus(), Section::Theme);

        let palette_before = state.palette_cursor();
        let segment_before = state.segment_cursor();

        state.move_cursor_down();

        assert_eq!(state.palette_cursor(), palette_before);
        assert_eq!(state.segment_cursor(), segment_before);
    }

    #[test]
    fn activate_cursor_selects_theme_at_cursor_when_theme_focused() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &[],
        );
        let mut state = test_state(config);
        assert_eq!(state.focus(), Section::Theme);

        let expected = state
            .available_themes()
            .get(state.theme_cursor())
            .cloned()
            .expect("theme cursor should point at a valid theme");

        state.activate_cursor();

        assert_eq!(state.selected_theme(), expected);
    }

    #[test]
    fn activate_cursor_selects_palette_at_cursor_when_palette_focused() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &[],
        );
        let mut state = test_state(config);
        state.next_section();
        assert_eq!(state.focus(), Section::Palette);

        let expected = state
            .available_palettes()
            .get(state.palette_cursor())
            .cloned()
            .expect("palette cursor should point at a valid palette");

        state.activate_cursor();

        assert_eq!(state.selected_palette(), expected);
    }

    #[test]
    fn activate_cursor_toggles_segment_at_cursor_when_segments_focused() {
        let config = make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &[],
        );
        let mut state = test_state(config);
        state.next_section();
        state.next_section();
        assert_eq!(state.focus(), Section::Segments);

        let target = state
            .available_segments()
            .first()
            .cloned()
            .expect("at least one builtin segment should exist");
        let before = state.is_segment_enabled(&target);

        state.activate_cursor();

        assert_ne!(state.is_segment_enabled(&target), before);
    }

    // --- filtered_indices tests ---

    #[test]
    fn filtered_indices_is_identity_when_query_empty() {
        let state = test_state(make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &[],
        ));

        let indices = state.filtered_indices(Section::Segments);

        assert_eq!(
            indices,
            (0..state.available_segments().len()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn filtered_indices_narrows_to_matching_items() {
        let state = test_state(make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &[],
        ));

        let indices = state.filtered_indices(Section::Segments);
        let has_match = indices.into_iter().any(|i| {
            state
                .available_segments()
                .get(i)
                .is_some_and(|segment| segment.contains("dir"))
        });

        assert!(
            has_match,
            "test relies on a segment containing 'dir' (e.g. directory) existing"
        );
    }

    #[test]
    fn move_cursor_down_steps_through_filtered_list_only() {
        // Regression for the filtered-cursor-movement rewrite: with an
        // empty query this must behave identically to the pre-filtering
        // implementation (already covered by
        // move_cursor_down_advances_within_focused_section above); this
        // test instead proves cursor movement stays within *filtered*
        // indices, exercised here via filtered_indices with an empty query.
        let mut state = test_state(make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &[],
        ));
        state.next_section();
        state.next_section();
        assert_eq!(state.focus(), Section::Segments);

        let indices = state.filtered_indices(Section::Segments);
        assert!(indices.len() >= 2, "test relies on 2+ builtin segments");

        let start = state.segment_cursor();
        state.move_cursor_down();

        assert_eq!(
            state.segment_cursor(),
            *indices.get(1).expect("indices has at least 2 entries")
        );
        let _ = start;
    }

    // --- filter mutation tests ---

    fn segments_focused_state() -> WizardState {
        let mut state = test_state(make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &[],
        ));
        state.next_section();
        state.next_section();
        assert_eq!(state.focus(), Section::Segments);
        state
    }

    #[test]
    fn push_filter_char_narrows_focused_section_list() {
        let mut state = segments_focused_state();

        for c in "dir".chars() {
            state.push_filter_char(c);
        }

        assert_eq!(state.filter_query(), "dir");
        let indices = state.filtered_indices(Section::Segments);
        assert!(!indices.is_empty());
        assert!(
            indices.iter().all(|&i| state
                .available_segments()
                .get(i)
                .is_some_and(|segment| segment.contains("dir"))),
            "every filtered index should match the typed query"
        );
    }

    #[test]
    fn push_filter_char_snaps_cursor_to_top_match_when_current_item_no_longer_matches() {
        let mut state = segments_focused_state();
        let git_index = state
            .available_segments()
            .iter()
            .position(|s| s == "git")
            .expect("git should be a builtin segment");
        while state.segment_cursor() != git_index {
            state.move_cursor_down();
        }

        for c in "dir".chars() {
            state.push_filter_char(c);
        }

        let cursor_item = state
            .available_segments()
            .get(state.segment_cursor())
            .expect("segment cursor should point at a valid segment");
        assert!(
            cursor_item.contains("dir"),
            "cursor should have snapped onto a matching item, was {cursor_item:?}"
        );
    }

    #[test]
    fn pop_filter_char_never_invalidates_cursor() {
        let mut state = segments_focused_state();
        for c in "dir".chars() {
            state.push_filter_char(c);
        }
        let cursor_after_push = state.segment_cursor();

        state.pop_filter_char();

        assert_eq!(state.filter_query(), "di");
        assert_eq!(
            state.segment_cursor(),
            cursor_after_push,
            "widening the query should never move a still-valid cursor"
        );
    }

    #[test]
    fn clear_filter_restores_full_list_and_keeps_cursor_item_highlighted() {
        let mut state = segments_focused_state();
        for c in "dir".chars() {
            state.push_filter_char(c);
        }
        let highlighted = state
            .available_segments()
            .get(state.segment_cursor())
            .expect("segment cursor should point at a valid segment")
            .clone();

        state.clear_filter();

        assert_eq!(state.filter_query(), "");
        assert_eq!(
            state.filtered_indices(Section::Segments).len(),
            state.available_segments().len()
        );
        assert_eq!(
            state
                .available_segments()
                .get(state.segment_cursor())
                .expect("segment cursor should point at a valid segment"),
            &highlighted,
            "clearing the filter should not move the cursor off the item it was on"
        );
    }

    #[test]
    fn filter_query_is_shared_across_sections_and_persists_across_tab() {
        let mut state = test_state(make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &[],
        ));
        assert_eq!(state.focus(), Section::Theme);

        state.push_filter_char('d');
        assert_eq!(state.filter_query(), "d");

        state.next_section(); // Theme -> Palette
        assert_eq!(
            state.filter_query(),
            "d",
            "the filter query is shared, not per-section — switching focus must not change it"
        );

        state.next_section(); // Palette -> Segments
        state.next_section(); // Segments -> Theme
        assert_eq!(state.focus(), Section::Theme);
        assert_eq!(
            state.filter_query(),
            "d",
            "the filter query must survive being tabbed away from and back"
        );
    }

    #[test]
    fn push_filter_char_narrows_every_section_not_just_focused() {
        // The filter is global: typing while Theme has focus must also
        // narrow Palette's and Segments' filtered_indices, not just
        // Theme's.
        let mut state = test_state(make_config(
            "default",
            "default",
            GitLanguageFlags {
                git: true,
                language: true,
            },
            &[],
        ));
        assert_eq!(state.focus(), Section::Theme);

        for c in "dir".chars() {
            state.push_filter_char(c);
        }

        let segment_indices = state.filtered_indices(Section::Segments);
        assert!(
            !segment_indices.is_empty(),
            "Segments should still narrow to matches even though Theme has focus"
        );
        assert!(
            segment_indices.iter().all(|&i| state
                .available_segments()
                .get(i)
                .is_some_and(|segment| segment.contains("dir"))),
            "every filtered Segments index should match the shared query"
        );
    }

    #[test]
    fn empty_filtered_result_makes_cursor_movement_and_activation_no_ops() {
        let mut state = segments_focused_state();
        let cursor_before = state.segment_cursor();
        let enabled_before: Vec<bool> = state
            .available_segments()
            .iter()
            .map(|s| state.is_segment_enabled(s))
            .collect();

        for c in "zzzznotarealsegment".chars() {
            state.push_filter_char(c);
        }
        assert!(state.filtered_indices(Section::Segments).is_empty());

        state.move_cursor_up();
        state.move_cursor_down();
        state.activate_cursor();

        assert_eq!(
            state.segment_cursor(),
            cursor_before,
            "cursor should not move against an empty filtered list"
        );
        let enabled_after: Vec<bool> = state
            .available_segments()
            .iter()
            .map(|s| state.is_segment_enabled(s))
            .collect();
        assert_eq!(
            enabled_after, enabled_before,
            "activate_cursor should be a no-op against an empty filtered list"
        );
    }
}
