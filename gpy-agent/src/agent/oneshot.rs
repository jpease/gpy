//! Oneshot request handling without the background agent (CLI non-daemon mode).

use crate::Result;
use crate::config::{
    Config,
    loader::{load_config, load_theme},
};
use crate::formatter::{
    DirectoryEnv, Format, IsFirst, IsLast, RenderContext, SegmentPosition, create_formatter,
};
use crate::ipc::Response;
use crate::language::detector::Detector;
use crate::language::display::{DisplayRequest, Versions, display_info};
use crate::theme::ThemeConfig;
use serde::{Deserialize, Serialize};
use std::path::Path;

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
    /// Status (exit-status pill) rendering.
    ExitStatus,
}

/// Oneshot request as parsed from CLI args or JSON.
#[derive(Debug, Serialize, Deserialize)]
pub struct OneshotRequest {
    /// Which handler should serve this request.
    #[serde(rename = "type")]
    pub request_type: OneshotKind,
    /// Working directory the request applies to (empty for path-less requests).
    pub path: String,
    /// Output format; defaults to `json`.
    #[serde(default)]
    pub format: Format,
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
    /// Whether the last command succeeded; `None` (non-character/status requests) means `false`.
    #[serde(default)]
    pub success: Option<bool>,
}

/// Empty language list response body.
const EMPTY_LANGUAGES_JSON: &str = "{\"languages\":[]}";

/// Handle a parsed oneshot request in-process.
///
/// # Errors
/// Returns an error if no theme loads, or if the requested detection or rendering fails.
pub fn handle_oneshot_request(request: &OneshotRequest) -> Result<String> {
    let _timer = (request.request_type == OneshotKind::GitStatus)
        .then(|| crate::profiling::Timer::new("git_total"));
    let format = request.format;
    let (config, theme) = load_config_and_theme(request)?;

    let Some(response) = response_for(request, &config, &theme)? else {
        return Ok(empty_language_output(format));
    };
    let directory = DirectoryEnv::for_response(&response, format, &config.ui.directory);
    let ctx = RenderContext::new(&config, &theme, request_position(request))
        .with_palette(crate::palette::active_palette(&config))
        .with_directory(&directory);
    create_formatter(format).render(&response, &ctx)
}

/// Parse a JSON oneshot request and handle it (test/schema entry point).
///
/// # Errors
/// Returns an error on invalid JSON or if [`handle_oneshot_request`] fails.
pub(super) fn handle_oneshot(request: &str) -> Result<String> {
    let parsed: OneshotRequest = serde_json::from_str(request)?;

    handle_oneshot_request(&parsed)
}

/// Build the response a request asks for; `None` means there is nothing to
/// render (no languages detected or language detection disabled).
fn response_for(
    request: &OneshotRequest,
    config: &Config,
    theme: &ThemeConfig,
) -> Result<Option<Response>> {
    Ok(match request.request_type {
        OneshotKind::GitStatus => Some(git_response(request, config)?),
        OneshotKind::LanguageDetect => language_response(request, config, theme),
        OneshotKind::Directory => Some(Response::Directory {
            cwd: request.path.clone(),
            read_only: crate::fs_util::directory_is_read_only(Path::new(&request.path)),
        }),
        OneshotKind::Duration => Some(Response::Duration {
            duration_ms: request.duration_ms,
        }),
        OneshotKind::Character => Some(Response::Character {
            success: request.success.unwrap_or(false),
        }),
        OneshotKind::ExitStatus => Some(Response::ExitStatus {
            success: request.success.unwrap_or(false),
        }),
    })
}

/// Git status for the requested path, or the error response the agent would send.
fn git_response(request: &OneshotRequest, config: &Config) -> Result<Response> {
    crate::language::version::set_version_cache_ttl(config.language.cache_ttl_hours.get());

    // `skip_paths` gates the repository root, like the agent. Resolved from the
    // canonical path (best effort), as with the IPC `SafePath`, so `~/` and
    // symlinked entries apply here too.
    let request_path = Path::new(&request.path);
    let canonical_request_path =
        std::fs::canonicalize(request_path).unwrap_or_else(|_| request_path.to_path_buf());
    let git_allowed = config.segment_enabled("git")
        && crate::git::find_repo_root(&canonical_request_path)
            .is_none_or(|root| config.git_enabled_for(&root));
    if !git_allowed {
        return Ok(Response::Error {
            message: "Git segment disabled via config".to_owned(),
        });
    }

    let _t = crate::profiling::Timer::new("git_load_repository");
    let Some(crate::git::CompleteStatus { status, .. }) =
        crate::git::native::NativeGitBackend::get_complete_status(
            request_path,
            config.git.max_ahead_behind,
            config.git.stash_enabled,
            std::time::Duration::from_secs(config.git.timeout_seconds.get()),
        )?
    else {
        return Ok(Response::Error {
            message: "Not in a git repository".to_owned(),
        });
    };

    Ok(Response::RepositoryStatus(if config.git.show_upstream {
        status
    } else {
        status.without_upstream()
    }))
}

/// Detected languages for the requested path, or `None` when there are none to show.
fn language_response(
    request: &OneshotRequest,
    config: &Config,
    theme: &ThemeConfig,
) -> Option<Response> {
    if !config.segment_enabled("language") {
        return None;
    }

    // Resolve the project root like the agent's lang handler so answers match.
    let root = crate::language::project_root(Path::new(&request.path)).into_path();
    let detected_languages =
        Detector::detect_directory_bounded(&root, config.language.detection_mode);
    if detected_languages.is_empty() {
        return None;
    }

    let display_request = DisplayRequest::new(&detected_languages, theme, &config.language)
        .with_cwd(Some(root.as_path()));
    let languages = display_info(&display_request, Versions::Probe).unwrap_or_default();
    (!languages.is_empty()).then_some(Response::Language { languages })
}

/// The output for a language request with nothing to show.
fn empty_language_output(format: Format) -> String {
    if Format::is_json(format) {
        EMPTY_LANGUAGES_JSON.to_owned()
    } else {
        String::new()
    }
}

/// Load config (falling back to defaults) and the theme the request renders with.
///
/// The theme file is read only when something consumes it: non-JSON output, or
/// the language list, whose JSON carries theme-derived colors and icons. A theme
/// that fails to load falls back to the `default` theme, as in the agent.
/// Benchmark-mode JSON git requests skip all config and theme file I/O.
///
/// # Errors
/// Returns an error if neither the configured nor the `default` theme loads.
fn load_config_and_theme(request: &OneshotRequest) -> Result<(Config, ThemeConfig)> {
    let json = Format::is_json(request.format);
    if request.benchmark_mode && json && request.request_type == OneshotKind::GitStatus {
        return Ok((Config::default(), ThemeConfig::default()));
    }

    let config = load_config().unwrap_or_else(|e| {
        crate::debug::warn_fallback("Config loading", "using defaults", &e);
        Config::default()
    });

    let theme_consumed = match request.request_type {
        OneshotKind::LanguageDetect => config.segment_enabled("language"),
        _ => !json,
    };
    let theme = if theme_consumed {
        load_theme(config.ui.theme.as_str()).or_else(|e| {
            let theme_name = &config.ui.theme;
            crate::debug::warn_fallback(
                &format!("Theme '{theme_name}' loading"),
                "using default",
                &e,
            );
            load_theme("default")
        })?
    } else {
        ThemeConfig::default()
    };

    Ok((config, theme))
}

/// The chain position a oneshot request asks for.
fn request_position(request: &OneshotRequest) -> SegmentPosition {
    SegmentPosition::new(
        IsLast::from(request.is_last),
        IsFirst::from(request.is_first),
    )
}

#[cfg(test)]
mod tests {

    use super::handle_oneshot;

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
