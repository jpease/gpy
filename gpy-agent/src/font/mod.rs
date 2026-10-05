//! Nerd Font capability detection.
//!
//! GPY's default prompt uses Nerd Font glyphs. On a machine whose terminal has
//! no Nerd Font, those glyphs render as tofu (□) or `?` — the most visible way
//! a prompt tool can look "broken" on someone else's machine (#411). This
//! module lets the install/init path pick an icon default that actually renders
//! where the user is, rather than assuming a Nerd Font is present.
//!
//! ## What this can and cannot tell us
//!
//! Reliable *per-glyph render* detection at runtime is effectively unsolvable —
//! a missing private-use glyph is substituted with a one-cell tofu box that is
//! indistinguishable, by cursor-width probing, from a correctly rendered
//! one-cell glyph. This is why Starship deliberately punts and just documents
//! "install a Nerd Font".
//!
//! So font detection here is an **asymmetric heuristic**, useful mainly in the
//! negative direction:
//!
//! - **No Nerd Font installed anywhere** → glyphs cannot render under any font
//!   on the system → tofu is essentially guaranteed. Reliable; the exact
//!   fresh-machine failure case from #411.
//! - **A Nerd Font is installed** → glyphs *might* render, depending on the
//!   terminal's configured font, which we cannot observe. A weak positive.
//!
//! Callers therefore treat [`FontCapability`] as a *suggested* default (see
//! [`recommend_show_icons`]) and, when interactive, still let the user confirm
//! against a rendered sample — the only reliable oracle is the human eye.
//!
//! ## Override
//!
//! `GPY_NERD_FONT` short-circuits detection entirely (both a user escape hatch
//! and the injection point used by tests):
//!
//! | Value                                  | Result                     |
//! |----------------------------------------|----------------------------|
//! | `1`, `true`, `yes`, `on`, `nerd`       | [`FontCapability::NerdFont`]   |
//! | `0`, `false`, `no`, `off`, `none`, `ascii` | [`FontCapability::NoNerdFont`] |
//! | `unknown`                              | [`FontCapability::Unknown`]    |
//! | unset / `auto` / anything else         | run platform detection     |

use std::path::{Path, PathBuf};

/// Whether the current system appears able to render Nerd Font glyphs.
///
/// See the module docs for why the positive case is only a hint while the
/// negative case is reliable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontCapability {
    /// A Nerd Font was found installed on the system.
    NerdFont,
    /// Font sources were readable but contained no Nerd Font.
    NoNerdFont,
    /// Capability could not be determined (no readable font source / no
    /// detection tool). Treated as "assume no glyphs" by [`recommend_show_icons`].
    Unknown,
}

/// The safe icon default for a detected capability.
///
/// Only a positively-detected Nerd Font recommends glyph icons; both
/// [`FontCapability::NoNerdFont`] and [`FontCapability::Unknown`] recommend
/// ASCII so an undetectable machine never renders tofu on first run (#411).
#[must_use]
pub const fn recommend_show_icons(capability: FontCapability) -> bool {
    matches!(capability, FontCapability::NerdFont)
}

/// Detect whether this machine has a Nerd Font available.
///
/// Honors the `GPY_NERD_FONT` override (see module docs) before falling back to
/// platform-specific scanning.
#[must_use]
pub fn detect_nerd_font() -> FontCapability {
    if let Some(forced) = override_from_env() {
        return forced;
    }
    detect_from_platform()
}

/// Parse the `GPY_NERD_FONT` override into a forced capability, if set to a
/// recognized value.
///
/// Public so `gpy-agent init` can tell an explicit answer apart from a
/// detected one: a set override is the user's decision and must not be
/// followed by a confirmation prompt (#640).
#[must_use]
pub fn override_from_env() -> Option<FontCapability> {
    let raw = std::env::var("GPY_NERD_FONT").ok()?;
    match raw.trim().to_lowercase().as_str() {
        "1" | "true" | "yes" | "on" | "nerd" => Some(FontCapability::NerdFont),
        "0" | "false" | "no" | "off" | "none" | "ascii" => Some(FontCapability::NoNerdFont),
        "unknown" => Some(FontCapability::Unknown),
        // "auto", empty, or anything unrecognized: fall through to detection.
        _ => None,
    }
}

/// Whether a font family/file name looks like a Nerd Font.
///
/// Matches the family form `fc-list` prints (`FiraCode Nerd Font`) and the
/// filename form fonts ship as (`FiraCodeNerdFont-Regular.ttf`), case-insensitively.
#[must_use]
fn name_is_nerd_font(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("nerd font") || lower.contains("nerdfont") || lower.contains("nerd-font")
}

/// Scan a set of font directories for any Nerd Font file.
///
/// Returns [`FontCapability::NerdFont`] on the first match, [`FontCapability::NoNerdFont`]
/// when at least one directory was readable but none matched, and
/// [`FontCapability::Unknown`] when no directory could be read at all (so we
/// genuinely don't know rather than falsely reporting "none installed").
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

/// Check one directory (recursively one level, to cover per-family subdirs) for
/// a Nerd Font file. `None` means the directory could not be read.
fn dir_has_nerd_font(dir: &Path) -> Option<bool> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let entry_name = entry.file_name();
        let name = entry_name.to_string_lossy();
        if name_is_nerd_font(&name) {
            return Some(true);
        }
        // One level of nesting: many installs group faces under a per-family
        // subdirectory (e.g. `~/.local/share/fonts/FiraCode/…`).
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

/// Hard timeout for the `fc-list` probe.
///
/// `fc-list` reads fontconfig's cached index and normally returns near-
/// instantly; this is defense against a genuinely wedged/broken fontconfig
/// (e.g. a stale or corrupt cache directory on a network mount), not a
/// normal-case budget. Mirrors the reasoning behind
/// `language::version::VERSION_COMMAND_TIMEOUT` (also a "normally instant,
/// defensively bounded" external tool call) at a shorter bound, since a font
/// listing has less legitimate work to do than a version-manager shim.
#[cfg(target_os = "linux")]
const FC_LIST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

#[cfg(target_os = "linux")]
fn detect_from_platform() -> FontCapability {
    // Prefer fontconfig's index (authoritative and fast) when available; fall
    // back to scanning common font directories if `fc-list` is not installed,
    // fails, or (#590) hangs past FC_LIST_TIMEOUT.
    if let Some(listing) = run_fc_list() {
        return if listing.lines().any(name_is_nerd_font) {
            FontCapability::NerdFont
        } else {
            FontCapability::NoNerdFont
        };
    }
    detect_from_font_dirs(&linux_font_dirs())
}

/// Run `fc-list` with a bounded timeout, returning its stdout on success.
///
/// Returns `None` on any failure -- spawn error, non-zero exit, or timeout --
/// so the caller falls back to scanning font directories exactly as it did
/// before this had a timeout at all.
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
    if let Some(home_os) = std::env::var_os("HOME") {
        let home = PathBuf::from(home_os);
        dirs.push(home.join(".local/share/fonts"));
        dirs.push(home.join(".fonts"));
    }
    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(data_home).join("fonts"));
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
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join("Library/Fonts"));
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
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(PathBuf::from(local).join("Microsoft/Windows/Fonts"));
    }
    dirs
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn detect_from_platform() -> FontCapability {
    FontCapability::Unknown
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

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
}
