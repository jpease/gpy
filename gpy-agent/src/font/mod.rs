//! Nerd Font capability detection, so `gpy setup` can pick an icon default that renders.
//!
//! Heuristic: "no Nerd Font installed" is reliable; "installed" is only a hint, since
//! the terminal's configured font is unobservable. `GPY_NERD_FONT` overrides detection.

use crate::env_config::{Reading, parse_flag};
use std::path::{Path, PathBuf};

/// Whether the current system appears able to render Nerd Font glyphs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontCapability {
    /// A Nerd Font was found installed on the system.
    NerdFont,
    /// Font sources were readable but contained no Nerd Font.
    NoNerdFont,
    /// Capability could not be determined; treated as "no glyphs".
    Unknown,
}

/// The safe icon default: glyphs only for a positively detected Nerd Font.
#[must_use]
pub const fn recommend_show_icons(capability: FontCapability) -> bool {
    matches!(capability, FontCapability::NerdFont)
}

/// Detect a Nerd Font, honoring the `GPY_NERD_FONT` override first.
#[must_use]
pub fn detect_nerd_font() -> FontCapability {
    if let Some(forced) = override_from_env() {
        return forced;
    }
    detect_from_platform()
}

/// Read the `GPY_NERD_FONT` override; `None` means detect. An unrecognised
/// value is ignored with one warning.
/// Public so `gpy setup` can skip confirmation for an explicit user answer.
#[must_use]
pub fn override_from_env() -> Option<FontCapability> {
    crate::env_config::NERD_FONT.nerd_font()
}

/// Parse a `GPY_NERD_FONT` value. Flag spellings and `nerd` force a Nerd
/// Font; falsey spellings, `none` and `ascii` force none; `unknown` forces
/// ASCII as "fonts cannot be detected"; `auto` detects (`Value(None)`).
#[must_use]
pub(crate) fn parse_nerd_font(raw: Option<&str>) -> Reading<Option<FontCapability>> {
    let Some(value) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Reading::Unset;
    };
    if value.eq_ignore_ascii_case("nerd") {
        return Reading::Value(Some(FontCapability::NerdFont));
    }
    if value.eq_ignore_ascii_case("none") || value.eq_ignore_ascii_case("ascii") {
        return Reading::Value(Some(FontCapability::NoNerdFont));
    }
    if value.eq_ignore_ascii_case("unknown") {
        return Reading::Value(Some(FontCapability::Unknown));
    }
    if value.eq_ignore_ascii_case("auto") {
        return Reading::Value(None);
    }
    match parse_flag(Some(value)) {
        Reading::Value(true) => Reading::Value(Some(FontCapability::NerdFont)),
        Reading::Value(false) => Reading::Value(Some(FontCapability::NoNerdFont)),
        Reading::Unset | Reading::Invalid(_) => Reading::Invalid(value.to_owned()),
    }
}

/// Whether a font family or file name (case-insensitive) looks like a Nerd Font.
fn name_is_nerd_font(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("nerd font") || lower.contains("nerdfont") || lower.contains("nerd-font")
}

/// Scan font directories; `Unknown` when none was readable.
fn detect_from_font_dirs(dirs: &[PathBuf]) -> FontCapability {
    let mut any_readable = false;
    for dir in dirs {
        match dir_has_nerd_font(dir) {
            Some(true) => return FontCapability::NerdFont,
            Some(false) => any_readable = true,
            None => {}
        }
    }
    if any_readable {
        FontCapability::NoNerdFont
    } else {
        FontCapability::Unknown
    }
}

/// Check one directory (plus per-family subdirs); `None` if unreadable.
fn dir_has_nerd_font(dir: &Path) -> Option<bool> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let entry_name = entry.file_name();
        let name = entry_name.to_string_lossy();
        if name_is_nerd_font(&name) {
            return Some(true);
        }
        if entry.file_type().is_ok_and(|ft| ft.is_dir())
            && let Ok(children) = std::fs::read_dir(entry.path())
        {
            for child in children.flatten() {
                if name_is_nerd_font(&child.file_name().to_string_lossy()) {
                    return Some(true);
                }
            }
        }
    }
    Some(false)
}

/// Defensive bound against a wedged fontconfig; `fc-list` is normally instant.
#[cfg(target_os = "linux")]
const FC_LIST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

#[cfg(target_os = "linux")]
fn detect_from_platform() -> FontCapability {
    // Under WSL the terminal is a Windows app; Linux fonts say nothing about it.
    if crate::wsl::is_wsl() {
        return FontCapability::Unknown;
    }
    // Prefer fontconfig's index; fall back to scanning font directories.
    if let Some(listing) = run_fc_list() {
        return if listing.lines().any(name_is_nerd_font) {
            FontCapability::NerdFont
        } else {
            FontCapability::NoNerdFont
        };
    }
    detect_from_font_dirs(&linux_font_dirs())
}

/// Run `fc-list` with a timeout; `None` on any failure.
#[cfg(target_os = "linux")]
fn run_fc_list() -> Option<String> {
    let mut command = std::process::Command::new("fc-list");
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    crate::process::spawn_in_own_process_group(&mut command);

    let child = command.spawn().ok()?;
    let output = crate::process::wait_with_timeout(child, FC_LIST_TIMEOUT).ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(target_os = "linux")]
fn linux_font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = crate::paths::home_path() {
        dirs.push(home.join(".local/share/fonts"));
        dirs.push(home.join(".fonts"));
    }
    if let Some(data_home) = crate::paths::xdg_data_home() {
        dirs.push(data_home.join("fonts"));
    }
    dirs.push(PathBuf::from("/usr/share/fonts"));
    dirs.push(PathBuf::from("/usr/local/share/fonts"));
    dirs
}

#[cfg(target_os = "macos")]
fn detect_from_platform() -> FontCapability {
    detect_from_font_dirs(&macos_font_dirs())
}

#[cfg(target_os = "macos")]
fn macos_font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = crate::paths::home_path() {
        dirs.push(home.join("Library/Fonts"));
    }
    dirs.push(PathBuf::from("/Library/Fonts"));
    dirs.push(PathBuf::from("/System/Library/Fonts"));
    dirs
}

#[cfg(target_os = "windows")]
fn detect_from_platform() -> FontCapability {
    detect_from_font_dirs(&windows_font_dirs())
}

#[cfg(target_os = "windows")]
fn windows_font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(windir) = std::env::var_os("WINDIR") {
        dirs.push(PathBuf::from(windir).join("Fonts"));
    }
    if let Some(local) = crate::paths::local_app_data() {
        dirs.push(local.join("Microsoft/Windows/Fonts"));
    }
    dirs
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn detect_from_platform() -> FontCapability {
    FontCapability::Unknown
}

#[cfg(test)]
mod tests {

    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn recommend_show_icons_only_true_for_nerd_font() {
        assert!(recommend_show_icons(FontCapability::NerdFont));
        assert!(!recommend_show_icons(FontCapability::NoNerdFont));
        assert!(
            !recommend_show_icons(FontCapability::Unknown),
            "an undetectable machine must default to ASCII, not glyphs"
        );
    }

    #[test]
    fn name_is_nerd_font_matches_family_and_file_spellings() {
        assert!(name_is_nerd_font("FiraCode Nerd Font"));
        assert!(name_is_nerd_font("FiraCodeNerdFont-Regular.ttf"));
        assert!(name_is_nerd_font("JetBrainsMono Nerd-Font Mono.ttf"));
        assert!(name_is_nerd_font("HACK NERD FONT")); // case-insensitive
    }

    #[test]
    fn name_is_nerd_font_rejects_plain_fonts() {
        assert!(!name_is_nerd_font("FiraCode-Regular.ttf"));
        assert!(!name_is_nerd_font("Menlo.ttc"));
        assert!(!name_is_nerd_font("Arial"));
    }

    #[test]
    fn detect_from_font_dirs_finds_nerd_font_file() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("JetBrainsMonoNerdFont-Regular.ttf"), b"x").unwrap();
        assert_eq!(
            detect_from_font_dirs(&[dir.path().to_path_buf()]),
            FontCapability::NerdFont
        );
    }

    #[test]
    fn detect_from_font_dirs_finds_nerd_font_in_subdir() {
        let dir = TempDir::new().unwrap();
        let family = dir.path().join("FiraCode");
        fs::create_dir_all(&family).unwrap();
        fs::write(family.join("FiraCodeNerdFont-Bold.ttf"), b"x").unwrap();
        assert_eq!(
            detect_from_font_dirs(&[dir.path().to_path_buf()]),
            FontCapability::NerdFont
        );
    }

    #[test]
    fn detect_from_font_dirs_readable_but_empty_is_no_nerd_font() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("Menlo.ttc"), b"x").unwrap();
        assert_eq!(
            detect_from_font_dirs(&[dir.path().to_path_buf()]),
            FontCapability::NoNerdFont
        );
    }

    #[test]
    fn detect_from_font_dirs_no_readable_dir_is_unknown() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("does-not-exist");
        assert_eq!(
            detect_from_font_dirs(&[missing]),
            FontCapability::Unknown,
            "an unreadable font source must be Unknown, not a false NoNerdFont"
        );
    }

    #[test]
    fn parse_nerd_font_accepts_flag_spellings_and_aliases() {
        let cases = [
            ("1", Some(FontCapability::NerdFont)),
            ("true", Some(FontCapability::NerdFont)),
            ("YES", Some(FontCapability::NerdFont)),
            (" on ", Some(FontCapability::NerdFont)),
            ("nerd", Some(FontCapability::NerdFont)),
            ("Nerd", Some(FontCapability::NerdFont)),
            ("0", Some(FontCapability::NoNerdFont)),
            ("false", Some(FontCapability::NoNerdFont)),
            ("no", Some(FontCapability::NoNerdFont)),
            ("OFF", Some(FontCapability::NoNerdFont)),
            ("none", Some(FontCapability::NoNerdFont)),
            ("ascii", Some(FontCapability::NoNerdFont)),
            ("unknown", Some(FontCapability::Unknown)),
            ("auto", None),
            ("AUTO", None),
        ];
        for (raw, expected) in cases {
            assert_eq!(
                parse_nerd_font(Some(raw)),
                Reading::Value(expected),
                "{raw}"
            );
        }
    }

    #[test]
    fn parse_nerd_font_rejects_unknown_values() {
        assert_eq!(parse_nerd_font(None), Reading::Unset);
        assert_eq!(parse_nerd_font(Some("")), Reading::Unset);
        assert_eq!(parse_nerd_font(Some("  ")), Reading::Unset);
        assert_eq!(
            parse_nerd_font(Some("blah")),
            Reading::Invalid("blah".to_owned())
        );
    }
}
