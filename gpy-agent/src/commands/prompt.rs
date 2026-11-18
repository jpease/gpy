//! PROTOTYPE (#1247): render the whole prompt in one process, so a shell hook
//! is a single `$(gpy prompt ...)`. Measures the single-exec frontend; not
//! wired into any shell by default.
//!
//! Known gaps, by design of the measurement: script (plugin) segments are
//! skipped; a git/language cache miss omits the segment instead of asking the
//! agent; `prev_bg` chevrons use the theme's colour names, so the cache read
//! usually takes the `.none` variant.

use crate::Result;
use crate::cache::InstantPromptCache;
use crate::config::{Config, loader::load_config};
use crate::formatter::{
    DirectoryEnv, Format, IsFirst, IsLast, PromptDialect, RenderContext, SegmentPosition,
    create_formatter,
};
use crate::ipc::Response;
use crate::shell::Shell;
use crate::template::{Palette, parse_color};
use crate::theme::{ThemeConfig, load::load_theme};
use std::path::{Path, PathBuf};

/// What the shell tells the prompt about the last command.
#[derive(Debug, Clone, Copy)]
pub struct PromptInput<'a> {
    /// Shell the prompt is rendered for.
    pub shell: Shell,
    /// Exit status of the last command.
    pub status: i32,
    /// Duration of the last command in milliseconds.
    pub duration_ms: u64,
    /// Working directory.
    pub cwd: &'a Path,
}

/// One segment, resolved before positions are assigned.
enum Piece {
    /// Rendered here from a response.
    Live(Response, String),
    /// A cached instant-prompt render: (variant base, key directory, bg name).
    Cached(&'static str, PathBuf, String),
}

/// Render the full prompt string for `input.shell`.
///
/// # Errors
/// Returns an error if neither the configured nor the `default` theme loads.
pub fn render(input: &PromptInput<'_>) -> Result<String> {
    let config = load_config().unwrap_or_default();
    let theme = load_theme(config.ui.theme.as_str()).or_else(|_| load_theme("default"))?;
    let palette = crate::palette::active_palette(&config);
    let (format, dialect) = match input.shell {
        Shell::Fish => (Format::Ansi, PromptDialect::Ansi),
        Shell::Zsh => (Format::ZshPrompt, PromptDialect::ZshPrompt),
        Shell::Bash => (Format::BashPrompt, PromptDialect::BashPrompt),
    };
    let success = input.status == 0_i32;
    let pieces = collect_pieces(input, &config, &theme, dialect, success);

    let mut out = String::new();
    let mut prev_bg = String::from("black");
    let count = pieces.len();
    for (index, piece) in pieces.iter().enumerate() {
        let position = SegmentPosition::new(
            IsLast::from(index.saturating_add(1) == count),
            IsFirst::from(index == 0),
        );
        match piece {
            Piece::Live(response, bg) => {
                out.push_str(&render_one(
                    response, format, &config, &theme, &palette, position, &prev_bg,
                )?);
                prev_bg.clone_from(bg);
            }
            Piece::Cached(base, dir, bg) => {
                let variant = format!("{base}{}", variant_suffix(position));
                let exact =
                    InstantPromptCache::cache_file_for_dir(dir, &variant, Some(&prev_bg), dialect)?;
                let path = if exact.is_file() {
                    exact
                } else {
                    InstantPromptCache::cache_file_for_dir(dir, &variant, None, dialect)?
                };
                if let Ok(text) = std::fs::read_to_string(&path) {
                    out.push_str(text.trim_end_matches('\n'));
                }
                prev_bg = if *base == "git" {
                    InstantPromptCache::cache_file_for_dir(dir, "git_bg", None, dialect)
                        .ok()
                        .and_then(|bg_path| std::fs::read_to_string(bg_path).ok())
                        .unwrap_or_else(|| {
                            theme
                                .segments
                                .git
                                .get_bg_color(crate::theme::GitState::Clean)
                                .to_owned()
                        })
                } else {
                    bg.clone()
                };
            }
        }
    }
    let position = SegmentPosition::new(IsLast::Yes, IsFirst::from(count == 0));
    out.push_str(&render_one(
        &Response::Character { success },
        format,
        &config,
        &theme,
        &palette,
        position,
        &prev_bg,
    )?);
    Ok(out)
}

/// The segments to draw, in config order, with cache misses already omitted
/// (as the shells do, so the rest get the right positions).
fn collect_pieces(
    input: &PromptInput<'_>,
    config: &Config,
    theme: &ThemeConfig,
    dialect: PromptDialect,
    success: bool,
) -> Vec<Piece> {
    let mut pieces = Vec::new();
    for segment in &config.ui.enabled_segments {
        match segment.as_str() {
            "directory" => pieces.push(Piece::Live(
                Response::Directory {
                    cwd: input.cwd.to_string_lossy().into_owned(),
                    read_only: crate::fs_util::directory_is_read_only(input.cwd),
                },
                theme.segments.directory.bg_color.to_string(),
            )),
            "clock" => pieces.push(Piece::Live(
                Response::Clock { shell: input.shell },
                theme.segments.clock.bg_color.to_string(),
            )),
            "duration" if input.duration_ms >= theme.segments.duration.show_if_exceeds_ms => {
                pieces.push(Piece::Live(
                    Response::Duration {
                        duration_ms: input.duration_ms,
                    },
                    theme.segments.duration.bg_color.to_string(),
                ));
            }
            "status" if !success => pieces.push(Piece::Live(
                Response::ExitStatus { success },
                theme.segments.status.fail_bg_color.to_string(),
            )),
            "git" => {
                if let Some(root) = crate::git::find_repo_root(input.cwd) {
                    pieces.push(Piece::Cached("git", root, String::new()));
                }
            }
            "language" => pieces.push(Piece::Cached(
                "lang",
                crate::language::project_root(input.cwd).into_path(),
                theme.segments.language.bg_color.to_string(),
            )),
            _ => {}
        }
    }
    pieces.retain(|piece| match piece {
        Piece::Live(..) => true,
        Piece::Cached(base, dir, _) => {
            InstantPromptCache::cache_file_for_dir(dir, base, None, dialect)
                .is_ok_and(|path| path.is_file())
        }
    });
    pieces
}

/// Variant suffix for a position, as `name::position_suffix` spells it.
const fn variant_suffix(position: SegmentPosition) -> &'static str {
    match (position.is_first, position.is_last) {
        (IsFirst::No, IsLast::No) => "",
        (IsFirst::No, IsLast::Yes) => "_last",
        (IsFirst::Yes, IsLast::No) => "_first",
        (IsFirst::Yes, IsLast::Yes) => "_first_last",
    }
}

fn render_one(
    response: &Response,
    format: Format,
    config: &Config,
    theme: &ThemeConfig,
    palette: &Palette,
    position: SegmentPosition,
    prev_bg: &str,
) -> Result<String> {
    let directory = DirectoryEnv::for_response(response, format, &config.ui.directory);
    let ctx = RenderContext::new(config, theme, position)
        .with_palette(palette.clone())
        .with_prev_colors(None, parse_color(prev_bg).ok())
        .with_directory(&directory);
    create_formatter(format).render(response, &ctx)
}
