//! Recommended-layout application for `gpy theme use --force`: diffs a theme's
//! `[ui.recommended]` block against the config and applies every field.

use crate::{config, theme};

/// One setting `--force` changes, as `get_config_value` prints it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LayoutChange {
    pub(crate) key: &'static str,
    pub(crate) old: String,
    pub(crate) new: String,
}

/// Pending config changes from a theme's `[ui.recommended]` block.
///
/// `Some` only for fields that would change; all-`None` keeps `theme use` silent.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct PendingLayout {
    enabled_segments: Option<Vec<String>>,
    directory_display: Option<config::types::DirectoryDisplay>,
    directory_truncation_length: Option<config::types::DirectoryTruncationLength>,
    directory_truncation_symbol: Option<config::types::DirectoryTruncationSymbol>,
    directory_truncate_to_repo: Option<bool>,
    language_detection: Option<config::types::DetectionMode>,
    palette: Option<config::types::PaletteName>,
}

impl PendingLayout {
    /// `true` when no recommended setting differs from the current config.
    const fn is_empty(&self) -> bool {
        self.enabled_segments.is_none()
            && self.directory_display.is_none()
            && self.directory_truncation_length.is_none()
            && self.directory_truncation_symbol.is_none()
            && self.directory_truncate_to_repo.is_none()
            && self.language_detection.is_none()
            && self.palette.is_none()
    }

    /// Write the differing settings into `config`. No-op for `None` fields.
    pub(crate) fn apply_to(&self, config: &mut config::Config) {
        if let Some(segments) = &self.enabled_segments {
            segments.clone_into(&mut config.ui.enabled_segments);
        }
        if let Some(display) = self.directory_display {
            config.ui.directory.display = display;
        }
        if let Some(length) = self.directory_truncation_length {
            config.ui.directory.truncation_length = length;
        }
        if let Some(symbol) = &self.directory_truncation_symbol {
            symbol.clone_into(&mut config.ui.directory.truncation_symbol);
        }
        if let Some(truncate_to_repo) = self.directory_truncate_to_repo {
            config.ui.directory.truncate_to_repo = truncate_to_repo;
        }
        if let Some(mode) = self.language_detection {
            config.language.detection_mode = mode;
        }
        if let Some(palette) = &self.palette {
            palette.clone_into(&mut config.ui.palette);
        }
    }

    /// The old and new value of every pending field, for `--force` output.
    pub(crate) fn changes(&self, config: &config::Config) -> Vec<LayoutChange> {
        let mut applied = config.clone();
        self.apply_to(&mut applied);
        let pending_keys = [
            (self.enabled_segments.is_some(), "ui.enabled_segments"),
            (self.directory_display.is_some(), "ui.directory.display"),
            (
                self.directory_truncation_length.is_some(),
                "ui.directory.truncation_length",
            ),
            (
                self.directory_truncation_symbol.is_some(),
                "ui.directory.truncation_symbol",
            ),
            (
                self.directory_truncate_to_repo.is_some(),
                "ui.directory.truncate_to_repo",
            ),
            (self.language_detection.is_some(), "language.detection_mode"),
            (self.palette.is_some(), "ui.palette"),
        ];
        pending_keys
            .into_iter()
            .filter(|(pending, _)| *pending)
            .filter_map(|(_, key)| {
                let old = config::metadata::get_config_value(config, key).ok()?;
                let new = config::metadata::get_config_value(&applied, key).ok()?;
                Some(LayoutChange { key, old, new })
            })
            .collect()
    }

    /// Comma-joined pending field names for the terse `theme use` hint.
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
        if self.language_detection.is_some() {
            parts.push("language.detection_mode");
        }
        if self.palette.is_some() {
            parts.push("ui.palette");
        }
        Some(parts.join(", "))
    }
}

/// Diff a theme's `[ui.recommended]` block against the current `config`.
///
/// An empty recommended segment list means "no recommendation".
#[must_use]
pub(crate) fn pending_layout(
    recommended: Option<&theme::RecommendedUi>,
    config: &config::Config,
) -> PendingLayout {
    let Some(rec) = recommended else {
        return PendingLayout::default();
    };
    let ui = &config.ui;
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
        language_detection: rec
            .language_detection
            .filter(|mode| *mode != config.language.detection_mode),
        palette: rec.palette.clone().filter(|palette| *palette != ui.palette),
    }
}

#[cfg(test)]
mod tests {

    use super::pending_layout;
    use crate::config;
    use crate::config::types::{
        DetectionMode, DirectoryDisplay, DirectoryTruncationLength, PaletteName,
    };
    use crate::theme::{RecommendedDirectory, RecommendedUi};

    fn segments(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn pending_layout_is_empty_without_recommendation() {
        let pending = pending_layout(None, &config::Config::default());
        assert!(pending.is_empty());
        assert_eq!(pending.field_names(), None);
    }

    #[test]
    fn pending_layout_keeps_only_differing_fields() {
        let recommended = RecommendedUi {
            enabled_segments: Some(segments(&["directory", "git"])),
            directory: Some(RecommendedDirectory {
                display: Some(DirectoryDisplay::Abbreviated),
                truncation_length: None,
                truncation_symbol: None,
                truncate_to_repo: None,
            }),
            language_detection: Some(DetectionMode::default()),
            palette: Some(PaletteName::default()),
        };
        let pending = pending_layout(Some(&recommended), &config::Config::default());
        assert_eq!(
            pending.enabled_segments,
            Some(segments(&["directory", "git"]))
        );
        assert_eq!(
            pending.directory_display,
            Some(DirectoryDisplay::Abbreviated)
        );
        assert_eq!(pending.language_detection, None);
        assert_eq!(pending.palette, None);
    }

    #[test]
    fn pending_layout_is_empty_when_recommendation_matches_config() {
        let config = config::Config::default();
        let ui = &config.ui;
        let recommended = RecommendedUi {
            enabled_segments: Some(ui.enabled_segments.clone()),
            directory: Some(RecommendedDirectory {
                display: Some(ui.directory.display),
                truncation_length: Some(ui.directory.truncation_length),
                truncation_symbol: None,
                truncate_to_repo: None,
            }),
            language_detection: Some(config.language.detection_mode),
            palette: Some(ui.palette.clone()),
        };
        assert!(pending_layout(Some(&recommended), &config).is_empty());
    }

    #[test]
    fn pending_layout_treats_empty_segment_list_as_no_recommendation() {
        let recommended = RecommendedUi {
            enabled_segments: Some(vec![]),
            ..RecommendedUi::default()
        };
        assert_eq!(
            pending_layout(Some(&recommended), &config::Config::default()).enabled_segments,
            None
        );
    }

    #[test]
    fn pending_layout_apply_writes_only_differing_fields() {
        let mut config = config::Config::default();
        let recommended = RecommendedUi {
            enabled_segments: Some(segments(&["directory", "git"])),
            directory: Some(RecommendedDirectory {
                display: Some(DirectoryDisplay::Truncated),
                truncation_length: Some(DirectoryTruncationLength::new(5).expect("5 valid")),
                truncation_symbol: None,
                truncate_to_repo: None,
            }),
            language_detection: Some(DetectionMode::Markers),
            palette: Some(PaletteName::new("starship".to_owned()).expect("valid")),
        };
        let pending = pending_layout(Some(&recommended), &config);
        pending.apply_to(&mut config);
        assert_eq!(config.ui.enabled_segments, segments(&["directory", "git"]));
        assert_eq!(config.ui.directory.display, DirectoryDisplay::Truncated);
        assert_eq!(config.ui.directory.truncation_length.get(), 5);
        assert_eq!(config.language.detection_mode, DetectionMode::Markers);
        assert_eq!(config.ui.palette.as_str(), "starship");
    }

    #[test]
    fn changes_lists_only_differing_fields() {
        let mut config = config::Config::default();
        config.ui.palette = PaletteName::new("nord".to_owned()).expect("valid");
        let recommended = RecommendedUi {
            enabled_segments: Some(segments(&["directory", "git"])),
            directory: Some(RecommendedDirectory {
                display: Some(config.ui.directory.display),
                truncation_length: None,
                truncation_symbol: None,
                truncate_to_repo: None,
            }),
            language_detection: None,
            palette: Some(PaletteName::new("starship".to_owned()).expect("valid")),
        };
        let changes = pending_layout(Some(&recommended), &config).changes(&config);
        let keys: Vec<&str> = changes.iter().map(|change| change.key).collect();
        assert_eq!(keys, ["ui.enabled_segments", "ui.palette"]);
        let [segments_change, palette_change] = changes.as_slice() else {
            panic!("expected two changes, got {}", changes.len());
        };
        assert_eq!(segments_change.new, "directory git");
        assert_eq!(palette_change.old, "nord");
        assert_eq!(palette_change.new, "starship");
    }

    #[test]
    fn pending_layout_carries_truncate_to_repo() {
        let recommended = RecommendedUi {
            directory: Some(RecommendedDirectory {
                display: None,
                truncation_length: None,
                truncation_symbol: None,
                truncate_to_repo: Some(true),
            }),
            ..RecommendedUi::default()
        };
        // Default config has truncate_to_repo = false, so true is pending.
        let mut config = config::Config::default();
        let pending = pending_layout(Some(&recommended), &config);
        assert_eq!(pending.directory_truncate_to_repo, Some(true));
        assert_eq!(pending.field_names().as_deref(), Some("truncate to repo"));
        pending.apply_to(&mut config);
        assert!(config.ui.directory.truncate_to_repo);

        // Matching recommendation -> not pending.
        let matching = pending_layout(Some(&recommended), &config);
        assert_eq!(matching.directory_truncate_to_repo, None);
        assert!(matching.is_empty());
    }

    #[test]
    fn pending_layout_carries_truncation_symbol() {
        use crate::config::types::DirectoryTruncationSymbol;
        let symbol = DirectoryTruncationSymbol::new("…/".to_owned()).expect("valid symbol");
        let recommended = RecommendedUi {
            directory: Some(RecommendedDirectory {
                display: None,
                truncation_length: None,
                truncation_symbol: Some(symbol.clone()),
                truncate_to_repo: None,
            }),
            ..RecommendedUi::default()
        };
        // Default config has an empty truncation symbol, so "…/" is pending.
        let mut config = config::Config::default();
        let pending = pending_layout(Some(&recommended), &config);
        assert_eq!(pending.directory_truncation_symbol, Some(symbol));
        pending.apply_to(&mut config);
        assert_eq!(config.ui.directory.truncation_symbol.as_str(), "…/");

        // Matching recommendation -> not pending.
        let matching = pending_layout(Some(&recommended), &config);
        assert_eq!(matching.directory_truncation_symbol, None);
        assert!(matching.is_empty());
    }
}
