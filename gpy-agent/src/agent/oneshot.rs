//! Oneshot request handling without the background agent (CLI non-daemon mode).

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

/// The kind of oneshot request; serializes as `snake_case` (`"git_status"`, ...).
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

/// Oneshot request as parsed from CLI args or JSON.
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
    /// Whether to render in benchmark mode.
    #[serde(default)]
    pub benchmark_mode: bool,
    /// Duration in milliseconds for `duration` requests; 0 for all other operations.
    #[serde(default)]
    pub duration_ms: u64,
    /// Whether the last command succeeded; `None` (non-character requests) means `false`.
    #[serde(default)]
    pub success: Option<bool>,
    /// The hostname to render; `None` (non-hostname requests) means empty.
    #[serde(default)]
    pub hostname: Option<String>,
    /// Whether the session is over SSH; gates the theme icon like IPC `is_ssh`.
    #[serde(default)]
    pub is_ssh: bool,
    /// The effective username to render; `None` (non-username requests) means empty.
    #[serde(default)]
    pub username: Option<String>,
}

/// Default format for oneshot requests.
fn default_format() -> String {
    "json".to_owned()
}

/// Empty language list response body.
const EMPTY_LANGUAGES_JSON: &str = "{\"languages\":[]}";

/// Handle a parsed oneshot request in-process.
///
/// # Errors
/// Returns an error if the format is invalid or the requested detection fails.
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

/// Parse a JSON oneshot request and handle it (test/schema entry point).
///
/// # Errors
/// Returns an error on invalid JSON or if [`handle_oneshot_request`] fails.
pub(super) fn handle_oneshot(request: &str) -> Result<String> {
    let parsed: OneshotRequest = serde_json::from_str(request)
        .map_err(|e| Error::ipc(format!("Invalid JSON in oneshot request: {e}")))?;

    handle_oneshot_request(&parsed)
}

/// Handle a oneshot git status request.
fn handle_oneshot_git_parsed(request: &OneshotRequest) -> Result<String> {
    let _timer = crate::profiling::Timer::new("git_total");
    let format = parse_format(&request.format)?;
    let position = request_position(request);
    let benchmark_mode = request.benchmark_mode && matches!(format, Format::Json);

    // Benchmark mode skips config/theme file I/O.
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

    // `skip_paths` gates the repository root, like the agent. Resolved from the
    // canonical path (best effort), as with the IPC `SafePath`, so `~/` and
    // symlinked entries apply here too.
    let request_path = Path::new(&request.path);
    let canonical_request_path =
        std::fs::canonicalize(request_path).unwrap_or_else(|_| request_path.to_path_buf());
    let git_allowed = config.git.enabled
        && crate::git::find_repo_root(&canonical_request_path)
            .is_none_or(|root| config.git.is_enabled_for(&root));
    if !git_allowed {
        let response = Response::Error {
            message: "Git segment disabled via config".to_owned(),
        };
        return create_formatter(format).render(&response, &ctx);
    }

    let Some(crate::git::CompleteStatus { status, .. }) = ({
        let _t = crate::profiling::Timer::new("git_load_repository");
        crate::git::status::load_repository_state_with(
            &crate::git::native::NativeGitBackend,
            std::path::Path::new(&request.path),
            None,
            config.git.max_ahead_behind,
            config.git.stash_enabled,
            std::time::Duration::from_secs(config.git.timeout_seconds.get()),
        )
        .map_err(|e| Error::ipc(format!("Git error: {e}")))?
    }) else {
        let response = Response::Error {
            message: "Not in a git repository".to_owned(),
        };
        return create_formatter(format).render(&response, &ctx);
    };

    let response_status = if config.git.show_upstream {
        status
    } else {
        status.without_upstream()
    };

    let response = Response::RepositoryStatus(response_status);

    create_formatter(format).render(&response, &ctx)
}

/// Handle a oneshot language detection request.
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

    // Resolve the project root like the agent's lang handler so answers match.
    let root = crate::language::project_root(Path::new(&request.path)).into_path();
    let detected_languages =
        Detector::detect_directory_bounded(&root, config.language.detection_mode);

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
        Some(root.as_path()),
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

    let ctx = RenderContext::new(&config, &theme, position)
        .with_palette(crate::palette::active_palette(&config));
    create_formatter(format).render(&response, &ctx)
}

/// Handle a oneshot directory request.
fn handle_oneshot_directory_parsed(request: &OneshotRequest) -> Result<String> {
    let format = parse_format(&request.format)?;
    let position = request_position(request);
    let (config, theme) = load_config_and_theme(format);

    let cwd = request.path.clone();
    let read_only = crate::fs_util::directory_is_read_only(Path::new(&cwd));

    let response = Response::Directory { cwd, read_only };
    render_response(&response, format, &config, &theme, position)
}

/// Handle a oneshot duration request.
fn handle_oneshot_duration_parsed(request: &OneshotRequest) -> Result<String> {
    let format = parse_format(&request.format)?;
    let position = request_position(request);
    let (config, theme) = load_config_and_theme(format);

    let response = Response::Duration {
        duration_ms: request.duration_ms,
    };
    render_response(&response, format, &config, &theme, position)
}

/// Handle a oneshot character request.
fn handle_oneshot_character_parsed(request: &OneshotRequest) -> Result<String> {
    let format = parse_format(&request.format)?;
    let position = request_position(request);
    let (config, theme) = load_config_and_theme(format);

    let response = Response::Character {
        success: request.success.unwrap_or(false),
    };
    render_response(&response, format, &config, &theme, position)
}

/// Handle a oneshot hostname request.
fn handle_oneshot_hostname_parsed(request: &OneshotRequest) -> Result<String> {
    let format = parse_format(&request.format)?;
    let position = request_position(request);
    let (config, theme) = load_config_and_theme(format);

    let response = Response::Hostname {
        hostname: request.hostname.clone().unwrap_or_default(),
        is_ssh: request.is_ssh,
    };
    render_response(&response, format, &config, &theme, position)
}

/// Handle a oneshot username request.
fn handle_oneshot_username_parsed(request: &OneshotRequest) -> Result<String> {
    let format = parse_format(&request.format)?;
    let position = request_position(request);
    let (config, theme) = load_config_and_theme(format);

    let response = Response::Username {
        username: request.username.clone().unwrap_or_default(),
    };
    render_response(&response, format, &config, &theme, position)
}

/// Parse a format name; unknown names error rather than defaulting to JSON.
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
fn request_position(request: &OneshotRequest) -> SegmentPosition {
    SegmentPosition::new(
        IsLast::from(request.is_last),
        IsFirst::from(request.is_first),
    )
}

/// Render a response through the standard oneshot render context.
fn render_response(
    response: &Response,
    format: Format,
    config: &Config,
    theme: &ThemeConfig,
    position: SegmentPosition,
) -> Result<String> {
    let ctx = RenderContext::new(config, theme, position)
        .with_palette(crate::palette::active_palette(config));
    create_formatter(format).render(response, &ctx)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    use super::{handle_oneshot, parse_format};

    /// An unknown `--format` must error, not silently coerce to JSON.
    #[test]
    fn parse_format_rejects_unknown_format() {
        let err = parse_format("not-a-real-format")
            .expect_err("an unrecognized format string must be rejected, not defaulted to JSON");

        assert!(
            err.to_string().contains("not-a-real-format"),
            "error should name the unrecognized format, got: {err}"
        );
    }

    /// A oneshot request with an invalid format must fail, not render as JSON.
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
