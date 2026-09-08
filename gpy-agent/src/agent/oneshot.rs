//! Oneshot request handling (no background agent mode)
//!
//! This module handles direct, one-time requests for git status and language
//! detection without running the full background agent. Used primarily by
//! the CLI in non-daemon mode.

use crate::config::{
    Config,
    loader::{load_config, load_theme},
};
use crate::formatter::{Format, IsFirst, IsLast, RenderContext, SegmentPosition, create_formatter};
use crate::ipc::Response;
use crate::language::detector::Detector;
use crate::theme::ThemeConfig;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::str::FromStr;

/// The kind of oneshot request, selecting which handler serves it.
///
/// Each variant serializes to the `snake_case` string the JSON wire form has
/// always used (`GitStatus` ↔ `"git_status"`, `LanguageDetect` ↔
/// `"language_detect"`, and so on), so this is a compile-time-checked
/// replacement for the string matching the dispatcher used to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OneshotKind {
    /// Git repository status.
    GitStatus,
    /// Language detection for a directory.
    LanguageDetect,
    /// Current directory rendering.
    Directory,
    /// Command duration rendering.
    Duration,
    /// Prompt character rendering.
    Character,
    /// Hostname rendering.
    Hostname,
    /// Username rendering.
    Username,
}

/// Oneshot request structure for proper JSON serialization
#[derive(Debug, Serialize, Deserialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent wire-protocol flags, not a state machine"
)]
pub struct OneshotRequest {
    /// Which handler should serve this request.
    #[serde(rename = "type")]
    pub request_type: OneshotKind,
    /// Working directory the request applies to (empty for path-less requests).
    pub path: String,
    /// Output format name (see [`crate::formatter::Format`]); defaults to `json`.
    #[serde(default = "default_format")]
    pub format: String,
    /// Whether this is the last agent-rendered segment in the prompt.
    #[serde(default)]
    pub is_last: bool,
    /// Whether this is the first agent-rendered segment in the prompt.
    #[serde(default)]
    pub is_first: bool,
    /// A specific changed file to consider, for incremental git status.
    #[serde(default)]
    pub changed_file: Option<String>,
    /// Whether to render in benchmark mode.
    #[serde(default)]
    pub benchmark_mode: bool,
    /// Duration in milliseconds for `duration` requests; 0 for all other operations.
    #[serde(default)]
    pub duration_ms: u64,
    /// Whether the last command succeeded (exit code 0); for `character` requests.
    /// `None` means this is not a character request; the handler defaults to `false`.
    #[serde(default)]
    pub success: Option<bool>,
    /// The hostname string to render; for `hostname` requests.
    /// `None` means this is not a hostname request; the handler defaults to empty.
    #[serde(default)]
    pub hostname: Option<String>,
    /// The effective username string to render; for `username` requests.
    /// `None` means this is not a username request; the handler defaults to empty.
    #[serde(default)]
    pub username: Option<String>,
}

/// Default format for oneshot requests
fn default_format() -> String {
    "json".to_owned()
}

/// JSON response body for an empty language list, used when language
/// detection is disabled or a repository has none (#592).
const EMPTY_LANGUAGES_JSON: &str = "{\"languages\":[]}";

/// Handle a single, already-parsed oneshot request (no background agent).
///
/// This is the in-process entry point: the `gpy-agent` CLI builds an
/// [`OneshotRequest`] straight from its parsed arguments and calls this, with
/// no JSON in between. Dispatch is exhaustive over [`OneshotKind`], so an
/// unknown request kind is now a compile error rather than a runtime one.
///
/// # Errors
///
/// Returns an error if the request format is invalid, if git status detection fails,
/// or if language detection fails.
pub fn handle_oneshot_request(request: &OneshotRequest) -> Result<String> {
    match request.request_type {
        OneshotKind::GitStatus => handle_oneshot_git_parsed(request),
        OneshotKind::LanguageDetect => handle_oneshot_language_parsed(request),
        OneshotKind::Directory => handle_oneshot_directory_parsed(request),
        OneshotKind::Duration => handle_oneshot_duration_parsed(request),
        OneshotKind::Character => handle_oneshot_character_parsed(request),
        OneshotKind::Hostname => handle_oneshot_hostname_parsed(request),
        OneshotKind::Username => handle_oneshot_username_parsed(request),
    }
}

/// Parse a JSON oneshot request and handle it.
///
/// No production caller constructs a oneshot request as a JSON string — the
/// CLI path uses [`handle_oneshot_request`] directly — but this string form is
/// kept as the public example/test entry point for the request schema.
///
/// # Errors
///
/// Returns an error if `request` is not valid JSON for an [`OneshotRequest`]
/// (including an unrecognized `type`), or if the parsed request fails; see
/// [`handle_oneshot_request`].
pub(super) fn handle_oneshot(request: &str) -> Result<String> {
    // Parse request using serde for proper JSON handling
    let parsed: OneshotRequest = serde_json::from_str(request)
        .map_err(|e| Error::ipc(format!("Invalid JSON in oneshot request: {e}")))?;

    handle_oneshot_request(&parsed)
}

/// Handle oneshot git request using parsed request struct
///
/// # Errors
///
/// Returns an error when git status collection fails for the requested repository.
#[expect(
    clippy::too_many_lines,
    reason = "collects git status, resolves the previous-segment background, and renders through the requested formatter in one linear request-handling pass; splitting would scatter one oneshot request's happy path across helpers with no independent reuse"
)]
fn handle_oneshot_git_parsed(request: &OneshotRequest) -> Result<String> {
    let _timer = crate::profiling::Timer::new("git_total");
    let format = parse_format(&request.format)?;
    let position = request_position(request);
    let benchmark_mode = request.benchmark_mode && matches!(format, Format::Json);

    // Benchmark mode intentionally measures the oneshot git request path without
    // config/theme file I/O. The normal JSON path still loads real config so the
    // CLI remains consistent with user settings.
    let config = if benchmark_mode {
        Config::default()
    } else {
        load_config().unwrap_or_else(|e| {
            crate::debug::warn_fallback("Config loading", "using defaults", &e);
            Config::default()
        })
    };
    crate::language::version::set_version_cache_ttl(config.language.cache_ttl_hours.get());

    let theme = if benchmark_mode || matches!(format, Format::Json) {
        ThemeConfig::default()
    } else {
        load_theme(config.ui.theme.as_str()).unwrap_or_else(|e| {
            let theme_name = &config.ui.theme;
            crate::debug::warn_fallback(
                &format!("Theme '{theme_name}' loading"),
                "using defaults",
                &e,
            );
            ThemeConfig::default()
        })
    };

    let palette = crate::palette::active_palette(&config);
    let ctx = RenderContext::new(&config, &theme, position).with_palette(palette);

    // Formats without a real formatter yet (BashSource/ZshSource) always resolve
    // to an empty string here, regardless of git-enabled state or repository
    // lookup outcome, so a single early guard covers all three return points
    // below instead of repeating the same `matches!` at each one.
    if !format.is_renderable() {
        return Ok(String::new());
    }

    if !config.git.enabled {
        let response = Response::Error {
            message: "Git segment disabled via config".to_owned(),
        };
        return create_formatter(format)?.render(&response, &ctx);
    }

    let changed_file_path = request.changed_file.as_ref().map(std::path::PathBuf::from);
    let paths = changed_file_path.as_ref().map(|p| vec![p.clone()]);

    let Some(crate::git::CompleteStatus { status, .. }) = ({
        let _t = crate::profiling::Timer::new("git_load_repository");
        crate::git::status::load_repository_state_with(
            &crate::git::native::NativeGitBackend,
            std::path::Path::new(&request.path),
            paths.as_deref(),
            config.git.max_ahead_behind,
            config.git.stash_enabled,
            std::time::Duration::from_secs(config.git.timeout_seconds.get()),
        )
        .map_err(|e| Error::ipc(format!("Git error: {e}")))?
    }) else {
        let response = Response::Error {
            message: "Not in a git repository".to_owned(),
        };
        return create_formatter(format)?.render(&response, &ctx);
    };

    let response_status = if config.git.show_upstream {
        status
    } else {
        status.without_upstream()
    };

    let response = Response::RepositoryStatus(response_status);

    create_formatter(format)?.render(&response, &ctx)
}

/// Handle oneshot language detection using parsed request struct
///
/// # Errors
///
/// Returns an error when language detection or serialization fails.
fn handle_oneshot_language_parsed(request: &OneshotRequest) -> Result<String> {
    let format = parse_format(&request.format)?;
    let position = request_position(request);

    let config = load_config().unwrap_or_else(|e| {
        crate::debug::warn_fallback("Config loading", "using defaults", &e);
        Config::default()
    });

    if !config.language.enabled {
        return match format {
            Format::Json => Ok(EMPTY_LANGUAGES_JSON.to_owned()),
            _ => Ok(String::new()),
        };
    }

    let theme = load_theme(config.ui.theme.as_str()).unwrap_or_else(|e| {
        let theme_name = &config.ui.theme;
        crate::debug::warn_fallback(
            &format!("Theme '{theme_name}' loading"),
            "using defaults",
            &e,
        );
        ThemeConfig::default()
    });

    let detected_languages =
        Detector::detect_directory_bounded(&request.path, config.language.detection_mode);

    if detected_languages.is_empty() {
        return match format {
            Format::Json => Ok(EMPTY_LANGUAGES_JSON.to_owned()),
            _ => Ok(String::new()),
        };
    }

    let languages_info = crate::language::display::build_language_display_info_at(
        &detected_languages,
        &theme,
        &config.language,
        Some(std::path::Path::new(&request.path)),
        None,
    );

    if languages_info.is_empty() {
        return match format {
            Format::Json => Ok(EMPTY_LANGUAGES_JSON.to_owned()),
            _ => Ok(String::new()),
        };
    }

    let response = Response::Language {
        languages: languages_info,
    };

    if !format.is_renderable() {
        return Ok(String::new());
    }

    let ctx = RenderContext::new(&config, &theme, position)
        .with_palette(crate::palette::active_palette(&config));
    create_formatter(format)?.render(&response, &ctx)
}

/// Handle oneshot directory request using parsed request struct
///
/// # Errors
///
/// Returns an error when config/theme loading or formatting fails.
fn handle_oneshot_directory_parsed(request: &OneshotRequest) -> Result<String> {
    let format = parse_format(&request.format)?;
    let position = request_position(request);
    let (config, theme) = load_config_and_theme(format);

    let cwd = request.path.clone();
    let read_only = crate::fs_util::directory_is_read_only(Path::new(&cwd));

    let response = Response::Directory { cwd, read_only };
    render_response(&response, format, &config, &theme, position)
}

/// Handle oneshot duration request using parsed request struct.
///
/// # Errors
///
/// Returns an error when config/theme loading or formatting fails.
fn handle_oneshot_duration_parsed(request: &OneshotRequest) -> Result<String> {
    let format = parse_format(&request.format)?;
    let position = request_position(request);
    let (config, theme) = load_config_and_theme(format);

    let response = Response::Duration {
        duration_ms: request.duration_ms,
    };
    render_response(&response, format, &config, &theme, position)
}

/// Handle oneshot character request using parsed request struct.
///
/// # Errors
///
/// Returns an error when config/theme loading or formatting fails.
fn handle_oneshot_character_parsed(request: &OneshotRequest) -> Result<String> {
    let format = parse_format(&request.format)?;
    let position = request_position(request);
    let (config, theme) = load_config_and_theme(format);

    let response = Response::Character {
        success: request.success.unwrap_or(false),
    };
    render_response(&response, format, &config, &theme, position)
}

/// Handle oneshot hostname request using parsed request struct.
///
/// # Errors
///
/// Returns an error when config/theme loading or formatting fails.
fn handle_oneshot_hostname_parsed(request: &OneshotRequest) -> Result<String> {
    let format = parse_format(&request.format)?;
    let position = request_position(request);
    let (config, theme) = load_config_and_theme(format);

    let response = Response::Hostname {
        hostname: request.hostname.clone().unwrap_or_default(),
    };
    render_response(&response, format, &config, &theme, position)
}

/// Handle oneshot username request using parsed request struct.
///
/// # Errors
///
/// Returns an error when config/theme loading or formatting fails.
fn handle_oneshot_username_parsed(request: &OneshotRequest) -> Result<String> {
    let format = parse_format(&request.format)?;
    let position = request_position(request);
    let (config, theme) = load_config_and_theme(format);

    let response = Response::Username {
        username: request.username.clone().unwrap_or_default(),
    };
    render_response(&response, format, &config, &theme, position)
}

/// Parse format string into `Format` enum.
///
/// # Errors
///
/// Returns an error if `format_str` does not name a known format (#572):
/// previously this silently coerced an unknown format to `Format::Json`,
/// hiding a typo'd `--format` flag behind a wrong-but-successful render.
fn parse_format(format_str: &str) -> Result<Format> {
    Format::from_str(format_str)
}

/// Load config (falling back to defaults) and the active theme, skipping theme
/// file I/O for JSON output where the theme is never rendered.
fn load_config_and_theme(format: Format) -> (Config, ThemeConfig) {
    let config = load_config().unwrap_or_else(|e| {
        crate::debug::warn_fallback("Config loading", "using defaults", &e);
        Config::default()
    });

    let theme = if matches!(format, Format::Json) {
        ThemeConfig::default()
    } else {
        load_theme(config.ui.theme.as_str()).unwrap_or_else(|e| {
            let theme_name = &config.ui.theme;
            crate::debug::warn_fallback(
                &format!("Theme '{theme_name}' loading"),
                "using defaults",
                &e,
            );
            ThemeConfig::default()
        })
    };

    (config, theme)
}

/// The chain position a oneshot request asks for.
///
/// The CLI's `--is-last`/`--is-first` flags arrive as bools; this is the one
/// place they become a [`SegmentPosition`], which then travels unconverted to
/// the formatter (#586).
fn request_position(request: &OneshotRequest) -> SegmentPosition {
    SegmentPosition::new(
        IsLast::from(request.is_last),
        IsFirst::from(request.is_first),
    )
}

/// Render a response through the standard oneshot render context.
///
/// # Errors
///
/// Returns an error when the formatter cannot be created or rendering fails.
fn render_response(
    response: &Response,
    format: Format,
    config: &Config,
    theme: &ThemeConfig,
    position: SegmentPosition,
) -> Result<String> {
    let ctx = RenderContext::new(config, theme, position)
        .with_palette(crate::palette::active_palette(config));
    create_formatter(format)?.render(response, &ctx)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    use super::{handle_oneshot, parse_format};

    /// #572: an unknown `--format` must error, not silently coerce to JSON.
    ///
    /// # Panics
    ///
    /// Panics if an unrecognized format string is accepted.
    #[test]
    fn parse_format_rejects_unknown_format() {
        let err = parse_format("not-a-real-format")
            .expect_err("an unrecognized format string must be rejected, not defaulted to JSON");

        assert!(
            err.to_string().contains("not-a-real-format"),
            "error should name the unrecognized format, got: {err}"
        );
    }

    /// #572 end-to-end: a oneshot request carrying an invalid format must
    /// fail the whole request rather than silently rendering as JSON.
    ///
    /// `directory` is the cheapest request type to exercise here since
    /// `parse_format` is the very first thing every `handle_oneshot_*_parsed`
    /// function does, so this returns before any config/theme file I/O.
    ///
    /// # Panics
    ///
    /// Panics if a request with an invalid format string succeeds.
    #[test]
    fn handle_oneshot_rejects_invalid_format_end_to_end() {
        let request = serde_json::json!({
            "type": "directory",
            "path": ".",
            "format": "not-a-real-format",
        })
        .to_string();

        let err = handle_oneshot(&request)
            .expect_err("a oneshot request with an invalid format must fail");

        assert!(
            err.to_string().contains("not-a-real-format"),
            "error should name the unrecognized format, got: {err}"
        );
    }
}
