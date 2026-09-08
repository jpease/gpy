//! Recommended-layout reconciliation engine for `gpy theme use --force`.
//!
//! When a theme declares a `[ui.recommended]` block, `--force` writes those
//! settings into `config.ui` (and the related `config.language.detection_mode`
//! / `config.ui.palette` fields) so the preset's intended look takes effect —
//! except for fields the user has explicitly configured, which are preserved.
//! This module contains the pure diff/partition/apply logic behind that
//! reconciliation; [`crate::commands::theme`] owns the CLI presentation
//! (`use_theme` and friends) and calls into it.
//!
//! The only I/O here is [`user_set_layout_fields`] reading the on-disk
//! `config.toml` to determine provenance — everything else operates on
//! already-loaded [`crate::config::Config`] / [`crate::config::UiSettings`]
//! and [`crate::theme::RecommendedUi`] values.

use crate::{config, theme};

/// Apply or preserve the theme's recommended detection mode under `--force`.
///
/// Mutates `config.language.detection_mode` only when the mode is applied.
/// Returns `(applied_fragment, preserved_fragment)` for the summary messages —
/// at most one is `Some`. A field is preserved when it is a genuine user edit
/// (present on disk and differing from the outgoing theme's recommendation).
pub(crate) fn resolve_forced_detection(
    config: &mut config::Config,
    recommended: Option<config::types::DetectionMode>,
    user_set: &UserSetLayoutFields,
    outgoing: Option<&theme::RecommendedUi>,
) -> (Option<String>, Option<String>) {
    let Some(mode) = recommended else {
        return (None, None);
    };
    let outgoing_mode = outgoing.and_then(|rec| rec.language_detection);
    let user_owned =
        user_set.language_detection && Some(config.language.detection_mode) != outgoing_mode;
    if user_owned {
        (
            None,
            Some(format!(
                "language detection ({})",
                config.language.detection_mode
            )),
        )
    } else {
        config.language.detection_mode = mode;
        (Some(format!("language detection ({mode})")), None)
    }
}

/// Apply or preserve the theme's recommended palette under `--force`.
///
/// Mutates `config.ui.palette` only when applied. Returns the applied-summary
/// fragment (`Some` when applied, `None` when the user's palette is preserved).
pub(crate) fn resolve_forced_palette(
    config: &mut config::Config,
    recommended: Option<config::types::PaletteName>,
    user_set_palette: bool,
) -> Option<String> {
    let palette = recommended?;
    if user_set_palette {
        return None;
    }
    let label = format!("palette ({})", palette.as_str());
    config.ui.palette = palette;
    Some(label)
}

/// Which `config.ui` layout fields the user explicitly wrote in `config.toml`.
///
/// Derived from the raw TOML so a value the user set is detected even when it
/// equals the built-in default — something the deserialized [`config::Config`]
/// (which fills defaults) cannot reveal.
#[expect(
    clippy::struct_excessive_bools,
    reason = "one presence flag per recommended-layout field; a set of booleans is the natural representation for this provenance probe"
)]
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct UserSetLayoutFields {
    enabled_segments: bool,
    directory_display: bool,
    directory_truncation_length: bool,
    directory_truncation_symbol: bool,
    directory_truncate_to_repo: bool,
    show_icons: bool,
    /// `[language].detection_mode` (applied from `[ui.recommended].language_detection`).
    language_detection: bool,
    /// `[ui].palette` (applied from `[ui.recommended].palette`).
    pub(crate) palette: bool,
}

/// Probe the raw `config.toml` for which layout keys the user explicitly set.
///
/// A missing or unparseable file yields all-`false` (treat nothing as user-set),
/// which is safe: `--force` then fills every recommended field.
pub(crate) fn user_set_layout_fields(path: &str) -> UserSetLayoutFields {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return UserSetLayoutFields::default();
    };
    let Ok(table) = contents.parse::<toml::Table>() else {
        return UserSetLayoutFields::default();
    };
    let ui = table.get("ui").and_then(toml::Value::as_table);
    let directory = ui
        .and_then(|ui_table| ui_table.get("directory"))
        .and_then(toml::Value::as_table);
    let language = table.get("language").and_then(toml::Value::as_table);
    UserSetLayoutFields {
        enabled_segments: ui.is_some_and(|ui_table| ui_table.contains_key("enabled_segments")),
        directory_display: directory.is_some_and(|dir| dir.contains_key("display")),
        directory_truncation_length: directory
            .is_some_and(|dir| dir.contains_key("truncation_length")),
        directory_truncation_symbol: directory
            .is_some_and(|dir| dir.contains_key("truncation_symbol")),
        directory_truncate_to_repo: directory
            .is_some_and(|dir| dir.contains_key("truncate_to_repo")),
        show_icons: ui.is_some_and(|ui_table| ui_table.contains_key("show_icons")),
        language_detection: language.is_some_and(|lang| lang.contains_key("detection_mode")),
        palette: ui.is_some_and(|ui_table| ui_table.contains_key("palette")),
    }
}

/// Partition one recommended field into `(to_apply, preserved)`.
///
/// The user's `current` value is **preserved** when they explicitly set it
/// (`user_set`) and it differs from the outgoing theme's own recommendation for
/// that field (a value a prior `--force` wrote, matching that theme's
/// recommendation, is theme-owned and stays overwritable). Otherwise the
/// `incoming` value is returned to be applied.
fn partition_field<T: Clone + PartialEq>(
    incoming: T,
    user_set: bool,
    current: &T,
    outgoing: Option<&T>,
) -> (Option<T>, Option<T>) {
    if user_set && Some(current) != outgoing {
        (None, Some(current.clone()))
    } else {
        (Some(incoming), None)
    }
}

/// Pending `config.ui` changes from a theme's `[ui.recommended]` block.
///
/// A field is `Some` only when applying it would change the current `config.ui`;
/// an all-`None` value means the recommendation already matches (or the theme
/// declares none), so `theme use` stays silent.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct PendingLayout {
    enabled_segments: Option<Vec<String>>,
    directory_display: Option<config::types::DirectoryDisplay>,
    directory_truncation_length: Option<config::types::DirectoryTruncationLength>,
    directory_truncation_symbol: Option<config::types::DirectoryTruncationSymbol>,
    directory_truncate_to_repo: Option<bool>,
    show_icons: Option<bool>,
}

impl PendingLayout {
    /// `true` when no recommended setting differs from the current config.
    const fn is_empty(&self) -> bool {
        self.enabled_segments.is_none()
            && self.directory_display.is_none()
            && self.directory_truncation_length.is_none()
            && self.directory_truncation_symbol.is_none()
            && self.directory_truncate_to_repo.is_none()
            && self.show_icons.is_none()
    }

    /// Write the differing settings into `config.ui`. No-op for `None` fields.
    pub(crate) fn apply_to(&self, ui: &mut config::UiSettings) {
        if let Some(segments) = &self.enabled_segments {
            segments.clone_into(&mut ui.enabled_segments);
        }
        if let Some(display) = self.directory_display {
            ui.directory.display = display;
        }
        if let Some(length) = self.directory_truncation_length {
            ui.directory.truncation_length = length;
        }
        if let Some(symbol) = &self.directory_truncation_symbol {
            symbol.clone_into(&mut ui.directory.truncation_symbol);
        }
        if let Some(truncate_to_repo) = self.directory_truncate_to_repo {
            ui.directory.truncate_to_repo = truncate_to_repo;
        }
        if let Some(show_icons) = self.show_icons {
            ui.show_icons = show_icons;
        }
    }

    /// Split pending changes into `(to_apply, preserved)` for `--force`.
    ///
    /// A field is **preserved** (left as the user has it) when it is both:
    /// - present in the raw `config.toml` (`user_set`), and
    /// - different from the outgoing theme's recommendation for that field — so a
    ///   value a prior `--force` wrote (which equals that theme's recommendation)
    ///   is treated as theme-owned and stays overwritable, keeping theme→theme
    ///   re-layout clean.
    ///
    /// `to_apply` carries the incoming theme's value for non-preserved fields;
    /// `preserved` carries the user's current value (for the summary message).
    pub(crate) fn partition_preserving(
        self,
        current: &config::UiSettings,
        user_set: &UserSetLayoutFields,
        outgoing: Option<&theme::RecommendedUi>,
    ) -> (Self, Self) {
        let outgoing_dir = outgoing.and_then(|rec| rec.directory.as_ref());
        let mut to_apply = Self::default();
        let mut preserved = Self::default();

        if let Some(segments) = self.enabled_segments {
            (to_apply.enabled_segments, preserved.enabled_segments) = partition_field(
                segments,
                user_set.enabled_segments,
                &current.enabled_segments,
                outgoing.and_then(|r| r.enabled_segments.as_ref()),
            );
        }
        if let Some(display) = self.directory_display {
            (to_apply.directory_display, preserved.directory_display) = partition_field(
                display,
                user_set.directory_display,
                &current.directory.display,
                outgoing_dir.and_then(|d| d.display).as_ref(),
            );
        }
        if let Some(length) = self.directory_truncation_length {
            let (apply, preserve) = partition_field(
                length,
                user_set.directory_truncation_length,
                &current.directory.truncation_length,
                outgoing_dir.and_then(|d| d.truncation_length).as_ref(),
            );
            to_apply.directory_truncation_length = apply;
            preserved.directory_truncation_length = preserve;
        }
        if let Some(symbol) = self.directory_truncation_symbol {
            let (apply, preserve) = partition_field(
                symbol,
                user_set.directory_truncation_symbol,
                &current.directory.truncation_symbol,
                outgoing_dir.and_then(|d| d.truncation_symbol.as_ref()),
            );
            to_apply.directory_truncation_symbol = apply;
            preserved.directory_truncation_symbol = preserve;
        }
        if let Some(truncate_to_repo) = self.directory_truncate_to_repo {
            let (apply, preserve) = partition_field(
                truncate_to_repo,
                user_set.directory_truncate_to_repo,
                &current.directory.truncate_to_repo,
                outgoing_dir.and_then(|d| d.truncate_to_repo).as_ref(),
            );
            to_apply.directory_truncate_to_repo = apply;
            preserved.directory_truncate_to_repo = preserve;
        }
        if let Some(show_icons) = self.show_icons {
            (to_apply.show_icons, preserved.show_icons) = partition_field(
                show_icons,
                user_set.show_icons,
                &current.show_icons,
                outgoing.and_then(|r| r.show_icons).as_ref(),
            );
        }
        (to_apply, preserved)
    }

    /// Human-readable, comma-joined description of the pending changes, or `None`
    /// when nothing would change.
    pub(crate) fn summary(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let mut parts = Vec::new();
        if let Some(segments) = &self.enabled_segments {
            parts.push(format!("segment order ({})", segments.join(", ")));
        }
        if let Some(display) = self.directory_display {
            parts.push(format!("directory display ({display})"));
        }
        if let Some(length) = self.directory_truncation_length {
            parts.push(format!("directory truncation length ({length})"));
        }
        if let Some(symbol) = &self.directory_truncation_symbol {
            parts.push(format!("directory truncation symbol ({symbol})"));
        }
        if let Some(truncate_to_repo) = self.directory_truncate_to_repo {
            parts.push(format!(
                "truncate to repo ({})",
                if truncate_to_repo { "on" } else { "off" }
            ));
        }
        if let Some(show_icons) = self.show_icons {
            parts.push(format!(
                "language icons ({})",
                if show_icons { "on" } else { "off" }
            ));
        }
        Some(parts.join("; "))
    }

    /// Comma-joined names (no values) of the pending fields, or `None` when
    /// nothing would change. Used for the terse `theme use` hint, as opposed
    /// to [`Self::summary`]'s full "field (value)" form used under `--force`.
    pub(crate) fn field_names(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let mut parts = Vec::new();
        if self.enabled_segments.is_some() {
            parts.push("segment order");
        }
        if self.directory_display.is_some() {
            parts.push("directory display");
        }
        if self.directory_truncation_length.is_some() {
            parts.push("directory truncation length");
        }
        if self.directory_truncation_symbol.is_some() {
            parts.push("directory truncation symbol");
        }
        if self.directory_truncate_to_repo.is_some() {
            parts.push("truncate to repo");
        }
        if self.show_icons.is_some() {
            parts.push("language icons");
        }
        Some(parts.join(", "))
    }
}

/// Diff a theme's `[ui.recommended]` block against the current `config.ui`.
///
/// Keeps only fields whose recommended value differs. An empty recommended
/// segment list is treated as "no recommendation". Pure helper so the apply/hint
/// decision is testable without touching disk.
#[must_use]
pub(crate) fn pending_layout(
    recommended: Option<&theme::RecommendedUi>,
    ui: &config::UiSettings,
) -> PendingLayout {
    let Some(rec) = recommended else {
        return PendingLayout::default();
    };
    let rec_dir = rec.directory.as_ref();
    PendingLayout {
        enabled_segments: rec
            .enabled_segments
            .as_ref()
            .filter(|segments| !segments.is_empty() && **segments != ui.enabled_segments)
            .cloned(),
        directory_display: rec_dir
            .and_then(|dir| dir.display)
            .filter(|display| *display != ui.directory.display),
        directory_truncation_length: rec_dir
            .and_then(|dir| dir.truncation_length)
            .filter(|length| *length != ui.directory.truncation_length),
        directory_truncation_symbol: rec_dir
            .and_then(|dir| dir.truncation_symbol.clone())
            .filter(|symbol| *symbol != ui.directory.truncation_symbol),
        directory_truncate_to_repo: rec_dir
            .and_then(|dir| dir.truncate_to_repo)
            .filter(|truncate| *truncate != ui.directory.truncate_to_repo),
        show_icons: rec.show_icons.filter(|show| *show != ui.show_icons),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    use super::{UserSetLayoutFields, pending_layout, user_set_layout_fields};
    use crate::config;
    use crate::config::UiSettings;
    use crate::config::types::{DirectoryDisplay, DirectoryTruncationLength, PaletteName};
    use crate::theme::{RecommendedDirectory, RecommendedUi};

    fn segments(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn forced_palette_applies_recommended_when_user_unset() {
        let mut cfg = config::Config::default();
        let recommended = Some(PaletteName::new("starship".to_owned()).unwrap());
        let user_set_palette = false;
        let applied = super::resolve_forced_palette(&mut cfg, recommended, user_set_palette);
        assert_eq!(cfg.ui.palette.as_str(), "starship");
        assert_eq!(applied.as_deref(), Some("palette (starship)"));
    }

    #[test]
    fn forced_palette_preserves_user_palette() {
        let mut cfg = config::Config::default();
        cfg.ui.palette = PaletteName::new("nord".to_owned()).unwrap();
        let recommended = Some(PaletteName::new("starship".to_owned()).unwrap());
        let user_set_palette = true;
        let applied = super::resolve_forced_palette(&mut cfg, recommended, user_set_palette);
        assert_eq!(cfg.ui.palette.as_str(), "nord", "user palette preserved");
        assert!(applied.is_none());
    }

    #[test]
    fn pending_layout_is_empty_without_recommendation() {
        let pending = pending_layout(None, &UiSettings::default());
        assert!(pending.is_empty());
        assert_eq!(pending.summary(), None);
    }

    #[test]
    fn pending_layout_keeps_only_differing_fields() {
        // Default config: basename, icons on, default segment order.
        let recommended = RecommendedUi {
            enabled_segments: Some(segments(&["directory", "git"])),
            directory: Some(RecommendedDirectory {
                display: Some(DirectoryDisplay::Abbreviated),
                truncation_length: None,
                truncation_symbol: None,
                truncate_to_repo: None,
            }),
            // Equal to the default (icons on) -> must be dropped from the diff.
            show_icons: Some(true),
            language_detection: None,
            palette: None,
        };
        let pending = pending_layout(Some(&recommended), &UiSettings::default());
        assert_eq!(
            pending.enabled_segments,
            Some(segments(&["directory", "git"]))
        );
        assert_eq!(
            pending.directory_display,
            Some(DirectoryDisplay::Abbreviated)
        );
        assert_eq!(
            pending.show_icons, None,
            "matching show_icons must not be pending"
        );
    }

    #[test]
    fn pending_layout_is_empty_when_recommendation_matches_config() {
        let ui = UiSettings::default();
        let recommended = RecommendedUi {
            enabled_segments: Some(ui.enabled_segments.clone()),
            directory: Some(RecommendedDirectory {
                display: Some(ui.directory.display),
                truncation_length: Some(ui.directory.truncation_length),
                truncation_symbol: None,
                truncate_to_repo: None,
            }),
            show_icons: Some(ui.show_icons),
            language_detection: None,
            palette: None,
        };
        assert!(pending_layout(Some(&recommended), &ui).is_empty());
    }

    #[test]
    fn pending_layout_treats_empty_segment_list_as_no_recommendation() {
        let recommended = RecommendedUi {
            enabled_segments: Some(vec![]),
            ..RecommendedUi::default()
        };
        assert_eq!(
            pending_layout(Some(&recommended), &UiSettings::default()).enabled_segments,
            None
        );
    }

    #[test]
    fn pending_layout_apply_writes_only_differing_fields() {
        let mut ui = UiSettings::default();
        let recommended = RecommendedUi {
            enabled_segments: Some(segments(&["directory", "git"])),
            directory: Some(RecommendedDirectory {
                display: Some(DirectoryDisplay::Truncated),
                truncation_length: Some(DirectoryTruncationLength::new(5).expect("5 valid")),
                truncation_symbol: None,
                truncate_to_repo: None,
            }),
            show_icons: Some(false),
            language_detection: None,
            palette: None,
        };
        let pending = pending_layout(Some(&recommended), &ui);
        pending.apply_to(&mut ui);
        assert_eq!(ui.enabled_segments, segments(&["directory", "git"]));
        assert_eq!(ui.directory.display, DirectoryDisplay::Truncated);
        assert_eq!(ui.directory.truncation_length.get(), 5);
        assert!(!ui.show_icons);
    }

    #[test]
    fn pending_layout_summary_lists_each_change() {
        let recommended = RecommendedUi {
            enabled_segments: Some(segments(&["directory", "git"])),
            directory: Some(RecommendedDirectory {
                display: Some(DirectoryDisplay::Truncated),
                truncation_length: Some(DirectoryTruncationLength::new(5).expect("5 valid")),
                truncation_symbol: None,
                truncate_to_repo: None,
            }),
            show_icons: Some(false),
            language_detection: None,
            palette: None,
        };
        let summary = pending_layout(Some(&recommended), &UiSettings::default())
            .summary()
            .expect("summary present");
        assert!(summary.contains("segment order (directory, git)"));
        assert!(summary.contains("directory display (truncated)"));
        assert!(summary.contains("directory truncation length (5)"));
        assert!(summary.contains("language icons (off)"));
    }

    #[test]
    fn pending_layout_carries_truncate_to_repo() {
        let recommended = RecommendedUi {
            enabled_segments: None,
            directory: Some(RecommendedDirectory {
                display: None,
                truncation_length: None,
                truncation_symbol: None,
                truncate_to_repo: Some(true),
            }),
            show_icons: None,
            language_detection: None,
            palette: None,
        };
        // Default config has truncate_to_repo = false, so true is pending.
        let mut ui = UiSettings::default();
        let pending = pending_layout(Some(&recommended), &ui);
        assert_eq!(pending.directory_truncate_to_repo, Some(true));
        assert!(
            pending
                .summary()
                .expect("summary present")
                .contains("truncate to repo (on)")
        );
        pending.apply_to(&mut ui);
        assert!(ui.directory.truncate_to_repo);

        // Matching recommendation -> not pending.
        let matching = pending_layout(Some(&recommended), &ui);
        assert_eq!(matching.directory_truncate_to_repo, None);
        assert!(matching.is_empty());
    }

    #[test]
    fn pending_layout_carries_truncation_symbol() {
        use crate::config::types::DirectoryTruncationSymbol;
        let symbol = DirectoryTruncationSymbol::new("…/".to_owned()).expect("valid symbol");
        let recommended = RecommendedUi {
            enabled_segments: None,
            directory: Some(RecommendedDirectory {
                display: None,
                truncation_length: None,
                truncation_symbol: Some(symbol.clone()),
                truncate_to_repo: None,
            }),
            show_icons: None,
            language_detection: None,
            palette: None,
        };
        // Default config has an empty truncation symbol, so "…/" is pending.
        let mut ui = UiSettings::default();
        let pending = pending_layout(Some(&recommended), &ui);
        assert_eq!(pending.directory_truncation_symbol, Some(symbol));
        assert!(
            pending
                .summary()
                .expect("summary present")
                .contains("directory truncation symbol (…/)")
        );
        pending.apply_to(&mut ui);
        assert_eq!(ui.directory.truncation_symbol.as_str(), "…/");

        // Matching recommendation -> not pending.
        let matching = pending_layout(Some(&recommended), &ui);
        assert_eq!(matching.directory_truncation_symbol, None);
        assert!(matching.is_empty());
    }

    #[test]
    fn user_set_layout_fields_detects_only_present_keys() {
        let temp = tempfile::tempdir().expect("temp dir");
        let path = temp.path().join("config.toml");
        // show_icons present but equal to the default (true): provenance must
        // still flag it, which the deserialized Config could not.
        std::fs::write(
            &path,
            "[ui]\nshow_icons = true\n\n[ui.directory]\ntruncation_length = 4\n",
        )
        .expect("write config");
        let user_set = user_set_layout_fields(path.to_str().expect("path str"));
        assert!(user_set.show_icons, "show_icons key is present");
        assert!(
            user_set.directory_truncation_length,
            "directory.truncation_length key is present"
        );
        assert!(!user_set.enabled_segments, "enabled_segments key is absent");
        assert!(
            !user_set.directory_display,
            "directory.display key is absent"
        );
    }

    #[test]
    fn user_set_layout_fields_missing_file_is_all_false() {
        let user_set = user_set_layout_fields("/nonexistent/path/config.toml");
        assert_eq!(user_set, UserSetLayoutFields::default());
    }

    #[test]
    fn partition_preserving_keeps_user_edits_and_applies_the_rest() {
        // User explicitly set enabled_segments and show_icons (both present and
        // differing from the outgoing theme's recommendation); directory display
        // is recommended but not user-set, so it should be applied.
        let current = UiSettings {
            enabled_segments: segments(&["clock", "git"]),
            show_icons: false,
            ..UiSettings::default()
        };

        let incoming = RecommendedUi {
            enabled_segments: Some(segments(&["directory", "git", "language"])),
            directory: Some(RecommendedDirectory {
                display: Some(DirectoryDisplay::Truncated),
                truncation_length: None,
                truncation_symbol: None,
                truncate_to_repo: None,
            }),
            show_icons: Some(true),
            language_detection: None,
            palette: None,
        };
        let pending = pending_layout(Some(&incoming), &current);

        let user_set = UserSetLayoutFields {
            enabled_segments: true,
            show_icons: true,
            ..UserSetLayoutFields::default()
        };
        // Outgoing theme recommended different values, so the user's are genuine.
        let outgoing = RecommendedUi {
            enabled_segments: Some(segments(&["clock", "duration"])),
            directory: None,
            show_icons: Some(true),
            language_detection: None,
            palette: None,
        };

        let (applied, preserved) =
            pending.partition_preserving(&current, &user_set, Some(&outgoing));

        assert_eq!(
            applied.directory_display,
            Some(DirectoryDisplay::Truncated),
            "non-user-set recommended field is applied"
        );
        assert_eq!(
            applied.enabled_segments, None,
            "user-set segment order is not applied"
        );
        assert_eq!(
            preserved.enabled_segments,
            Some(segments(&["clock", "git"])),
            "preserved carries the user's current segment order"
        );
        assert_eq!(
            preserved.show_icons,
            Some(false),
            "user-set show_icons is preserved"
        );
    }

    #[test]
    fn partition_preserving_overwrites_values_matching_outgoing_theme() {
        // A field whose on-disk value equals the outgoing theme's recommendation
        // was theme-written by a prior --force, so even though the key is present
        // it must be overwritten by the incoming theme (clean re-layout).
        let current = UiSettings {
            enabled_segments: segments(&["directory", "git", "language", "duration"]),
            ..UiSettings::default()
        };

        let incoming = RecommendedUi {
            enabled_segments: Some(segments(&[
                "clock",
                "duration",
                "language",
                "directory",
                "git",
            ])),
            directory: None,
            show_icons: None,
            language_detection: None,
            palette: None,
        };
        let pending = pending_layout(Some(&incoming), &current);

        let user_set = UserSetLayoutFields {
            enabled_segments: true, // present on disk (written by prior --force)
            ..UserSetLayoutFields::default()
        };
        let outgoing = RecommendedUi {
            // Equals current on-disk value => theme-owned, not a user edit.
            enabled_segments: Some(segments(&["directory", "git", "language", "duration"])),
            directory: None,
            show_icons: None,
            language_detection: None,
            palette: None,
        };

        let (applied, preserved) =
            pending.partition_preserving(&current, &user_set, Some(&outgoing));
        assert_eq!(
            applied.enabled_segments,
            Some(segments(&[
                "clock",
                "duration",
                "language",
                "directory",
                "git"
            ])),
            "theme-owned field is overwritten by the incoming recommendation"
        );
        assert_eq!(preserved.enabled_segments, None, "nothing preserved");
    }
}
