//! `gpy prompt`: the whole Bash prompt from one process, so the shell hook is
//! one `PS1=$(gpy prompt ...)` instead of a fork per segment (#1247). It
//! follows `bash/core/init.bash`'s `__gpy_render_prompt` step for step and
//! reads the same theme-export values, cache files, refresh-throttle stamps
//! and late window, so the two render paths print the same `PS1` and can run
//! side by side. Script segments are not drawn here yet: the shell keeps its
//! own path while any is enabled.

mod link;
mod pill;
mod vars;

use crate::Result;
use crate::agent::oneshot::{OneshotKind, OneshotRequest, handle_oneshot_request};
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
use link::{Link, Reply};
use pill::{Depth, Place};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use vars::ShellVars;

/// What the shell knows about the command that just finished.
#[derive(Debug, Clone, Copy)]
pub struct PromptArgs {
    /// Exit status of the last command.
    pub status: i32,
    /// Its duration in milliseconds; `None` before the first command.
    pub duration_ms: Option<u64>,
    /// The shell's PID: names its refresh-throttle stamps and late window.
    pub pid: u32,
}

/// The built-in segments `gpy prompt` draws.
const BUILTIN_SEGMENTS: [&str; 6] = [
    "clock",
    "duration",
    "language",
    "directory",
    "git",
    "status",
];

/// Whether `gpy prompt` can draw every enabled segment of `config`: false
/// while a script or plugin segment is enabled.
fn covers(config: &Config) -> bool {
    config
        .effective_segments()
        .all(|segment| BUILTIN_SEGMENTS.contains(&segment))
}

/// Render the Bash `PS1` for the current directory (`$PWD`); `None` when an
/// enabled segment needs the shell's own renderer.
///
/// # Errors
/// Returns an error if neither the configured nor the `default` theme loads.
pub fn render_bash(args: PromptArgs) -> Result<Option<String>> {
    let config = load_config().unwrap_or_else(|e| {
        crate::debug::warn_fallback("Config loading", "using defaults", &e);
        Config::default()
    });
    if !covers(&config) {
        return Ok(None);
    }
    let theme = load_theme(config.ui.theme.as_str()).or_else(|_| load_theme("default"))?;
    let palette = crate::palette::active_palette(&config);
    let vars = ShellVars::new(crate::theme::export::builtin_shell_values(
        &theme,
        config.ui.theme.as_str(),
        &config,
        &palette,
    ));
    let pwd = logical_pwd();
    let canonical = std::fs::canonicalize(&pwd).ok();
    let git_root = canonical.as_deref().and_then(find_git_root);
    let enabled = agent_enabled(&config);
    let mut render = Render {
        depth: Depth::from_env(
            env("GPY_COLOR_DEPTH").as_deref(),
            env("TERM_PROGRAM").as_deref(),
            env("TERM").as_deref(),
        ),
        link: Link::new(enabled, args.pid),
        config: &config,
        theme: &theme,
        palette,
        vars,
        args,
        pwd,
        canonical,
        git_root,
        oneshot_used: false,
    };
    Ok(Some(render.prompt()))
}

/// `$PWD` when it names the working directory, else the physical one.
fn logical_pwd() -> PathBuf {
    let physical = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    std::env::var_os("PWD")
        .map(PathBuf::from)
        .filter(|pwd| {
            pwd.is_absolute()
                && std::fs::canonicalize(pwd)
                    .ok()
                    .zip(std::fs::canonicalize(&physical).ok())
                    .is_some_and(|(a, b)| a == b)
        })
        .unwrap_or(physical)
}

/// A non-empty environment value.
fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// The shared boolean table: `Some(true)` for 1/true/yes/on, `Some(false)`
/// for 0/false/no/off (case-insensitive), `None` otherwise.
fn flag(name: &str) -> Option<bool> {
    match env(name)?.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// `__gpy_agent_enabled`: `GPY_AGENT_ENABLED`, else `agent.enabled`.
fn agent_enabled(config: &Config) -> bool {
    flag("GPY_AGENT_ENABLED").unwrap_or(config.agent.enabled)
}

/// `__gpy_find_git_root`: the nearest ancestor of `canonical` (not `/`)
/// holding a `.git`.
fn find_git_root(canonical: &Path) -> Option<PathBuf> {
    canonical
        .ancestors()
        .take_while(|dir| dir.parent().is_some())
        .find(|dir| dir.join(".git").exists())
        .map(Path::to_path_buf)
}

/// One render's inputs and the oneshot budget.
struct Render<'a> {
    depth: Depth,
    link: Link,
    config: &'a Config,
    theme: &'a ThemeConfig,
    palette: Palette,
    vars: ShellVars,
    args: PromptArgs,
    /// Logical `$PWD`: request paths and refresh buckets use it.
    pwd: PathBuf,
    /// `$PWD` with symlinks resolved, when it resolves.
    canonical: Option<PathBuf>,
    git_root: Option<PathBuf>,
    /// One in-process oneshot per render (`__gpy_oneshot_claim`).
    oneshot_used: bool,
}

/// What a segment printed and whether it fell back to oneshot.
struct Drawn {
    text: String,
    oneshot: bool,
}

impl Drawn {
    const fn plain(text: String) -> Self {
        Self {
            text,
            oneshot: false,
        }
    }
}

impl Render<'_> {
    /// `__gpy_render_prompt`.
    fn prompt(&mut self) -> String {
        let mut out = String::new();
        if self.vars.or_nonempty("__gpy_add_newline", "1") == "1" {
            out.push('\n');
        }
        let segments: Vec<String> = self
            .config
            .effective_segments()
            .filter(|segment| self.would_render(segment))
            .map(str::to_owned)
            .collect();
        let mut prev_bg = String::from("black");
        let count = segments.len();
        for (index, segment) in segments.iter().enumerate() {
            let place = Place {
                is_last: index.saturating_add(1) == count,
                is_first: index == 0,
            };
            let drawn = self.segment(segment, place, &prev_bg);
            if !drawn.text.is_empty() {
                out.push_str(&drawn.text);
                let bg = self.segment_bg(segment, drawn.oneshot);
                if !bg.is_empty() {
                    prev_bg = bg;
                }
            }
        }
        if self.vars.or_nonempty("__gpy_two_line", "0") == "1" {
            out.push('\n');
        }
        let root = nix::unistd::geteuid().is_root();
        let character = if root {
            String::new()
        } else {
            self.render_response(
                &Response::Character {
                    success: self.args.status == 0_i32,
                },
                Place {
                    is_last: true,
                    is_first: false,
                },
                &prev_bg,
            )
        };
        if character.is_empty() {
            out.push_str(&pill::prompt_tail(
                &self.vars,
                self.depth,
                self.args.status,
                root,
                flag("GPY_SHOW_STATUS").unwrap_or(true),
            ));
        } else {
            out.push_str(&character);
        }
        out
    }

    /// `__gpy_segment_would_render`: detect, then omit.
    fn would_render(&self, segment: &str) -> bool {
        match segment {
            "directory" | "clock" | "status" => true,
            "duration" => self
                .args
                .duration_ms
                .is_some_and(|ms| ms >= self.duration_threshold()),
            "git" => {
                let Some(root) = &self.git_root else {
                    return false;
                };
                !(self.link.enabled()
                    && !marker_present(root, "git")
                    && !self.link.socket_present())
            }
            "language" => {
                if !lang_detect(&self.pwd) {
                    return false;
                }
                let present = self
                    .lang_key()
                    .is_some_and(|key| marker_present(&key, "lang"));
                if !present {
                    self.refresh(
                        "lang",
                        "lang",
                        "",
                        Place {
                            is_last: false,
                            is_first: false,
                        },
                        "",
                    );
                }
                present
            }
            _ => false,
        }
    }

    /// `__duration_threshold_ms`, default 100.
    fn duration_threshold(&self) -> u64 {
        self.vars
            .or_nonempty("__duration_threshold_ms", "100")
            .parse()
            .unwrap_or(100)
    }

    /// The language cache key path: the git root, else the canonical `$PWD`.
    fn lang_key(&self) -> Option<PathBuf> {
        self.git_root.clone().or_else(|| self.canonical.clone())
    }

    fn segment(&mut self, segment: &str, place: Place, prev_bg: &str) -> Drawn {
        match segment {
            "directory" => Drawn::plain(self.render_response(
                &Response::Directory {
                    cwd: self.pwd.to_string_lossy().into_owned(),
                    read_only: crate::fs_util::directory_is_read_only(
                        self.canonical.as_deref().unwrap_or(&self.pwd),
                    ),
                },
                place,
                prev_bg,
            )),
            "clock" => Drawn::plain(self.clock(place, prev_bg)),
            "status" => Drawn::plain(self.status(place, prev_bg)),
            "duration" => Drawn::plain(self.render_response(
                &Response::Duration {
                    duration_ms: self.args.duration_ms.unwrap_or(0),
                },
                place,
                prev_bg,
            )),
            "git" => self.git(place, prev_bg),
            "language" => Drawn::plain(self.language(place, prev_bg)),
            _ => Drawn::plain(String::new()),
        }
    }

    /// `__gpy_segment_clock`: the theme's clock template, else the local pill
    /// with Bash's live `\D{…}`.
    fn clock(&self, place: Place, prev_bg: &str) -> String {
        if self.vars.is_set("__clock_format") {
            let rendered =
                self.render_response(&Response::Clock { shell: Shell::Bash }, place, prev_bg);
            if !rendered.is_empty() {
                return rendered;
            }
        }
        let content = format!(
            "{}\\D{{{}}}",
            pill::prompt_escape(self.vars.or("__prompt_time_pad", " ")),
            pill::clock_time_spec(&self.vars)
        );
        pill::section_standalone(
            &self.vars,
            self.depth,
            self.vars.or_nonempty("__color_clock_bg", ""),
            self.vars.or_nonempty("__color_clock_fg", ""),
            &content,
            place,
        )
    }

    /// `__gpy_segment_status`: the theme's template, else the local pill.
    fn status(&self, place: Place, prev_bg: &str) -> String {
        let success = self.args.status == 0_i32;
        if self.vars.is_set("__status_format") {
            let rendered = self.render_response(&Response::ExitStatus { success }, place, prev_bg);
            if !rendered.is_empty() {
                return rendered;
            }
        }
        let (bg, fg) = if success {
            (
                self.vars.or_nonempty("__color_status_ok_bg", "green"),
                self.vars.or_nonempty("__color_status_ok_fg", "black"),
            )
        } else {
            (
                self.vars.or_nonempty("__color_status_fail_bg", "red"),
                self.vars.or_nonempty("__color_status_fail_fg", "black"),
            )
        };
        let icon = pill::prompt_escape(&format!(" {}", pill::status_icon(&self.vars, success)));
        pill::section_standalone(&self.vars, self.depth, bg, fg, &icon, place)
    }

    /// `__gpy_segment_git`: serve the instant cache, ask the agent on a cold
    /// or wrong-variant miss, refresh a stale entry, and fall back to an
    /// in-process oneshot when the agent is off or down.
    fn git(&mut self, place: Place, prev_bg: &str) -> Drawn {
        let suffix = variant_suffix("git", place);
        let Some(root) = self.git_root.clone() else {
            return Drawn::plain(String::new());
        };
        let Some(entry) = read_entry(&root, &suffix, prev_bg, "git") else {
            if !self.link.enabled() {
                return self.oneshot_git(place);
            }
            let request = self.data_request("git", place, prev_bg, true);
            if let Reply::Text(text) = self.link.send(&request, Some(link::cold_timeout_ms())) {
                return Drawn::plain(text);
            }
            self.refresh("git", &suffix, "", place, prev_bg);
            return Drawn::plain(String::new());
        };
        if entry.text.is_empty() {
            return Drawn::plain(String::new());
        }
        if entry.variant
            && let Reply::Text(text) = self
                .link
                .send(&self.data_request("git", place, prev_bg, true), None)
        {
            return Drawn::plain(text);
        }
        if !entry.stale {
            return Drawn::plain(entry.text);
        }
        if self.link.socket_present() {
            self.refresh("git", &suffix, "", place, prev_bg);
            return Drawn::plain(entry.text);
        }
        let fresh = self.oneshot_git(place);
        if fresh.text.is_empty() {
            Drawn {
                text: entry.text,
                oneshot: fresh.oneshot,
            }
        } else {
            fresh
        }
    }

    /// `__gpy_segment_language`: cache only; misses and stale or
    /// wrong-variant entries ask for a background refresh.
    fn language(&self, place: Place, prev_bg: &str) -> String {
        let suffix = variant_suffix("lang", place);
        let Some(entry) = self
            .lang_key()
            .and_then(|key| read_entry(&key, &suffix, prev_bg, "lang"))
        else {
            self.refresh("lang", &suffix, "", place, prev_bg);
            return String::new();
        };
        if entry.variant {
            let token = InstantPromptCache::prev_bg_token(prev_bg);
            self.refresh("lang", &suffix, &token, place, prev_bg);
        }
        if entry.stale {
            self.refresh("lang", &suffix, "", place, prev_bg);
        }
        entry.text
    }

    /// The in-process twin of `gpy-agent oneshot git`, once per render.
    fn oneshot_git(&mut self, place: Place) -> Drawn {
        if self.oneshot_used {
            return Drawn::plain(String::new());
        }
        self.oneshot_used = true;
        let request = OneshotRequest {
            request_type: OneshotKind::GitStatus,
            path: self.pwd.to_string_lossy().into_owned(),
            format: Format::BashPrompt,
            is_last: place.is_last,
            is_first: place.is_first,
            benchmark_mode: false,
            duration_ms: 0,
            success: None,
        };
        Drawn {
            text: handle_oneshot_request(&request).map_or_else(
                |_| String::new(),
                |text| text.trim_end_matches('\n').to_owned(),
            ),
            oneshot: true,
        }
    }

    /// A git/lang request as `__gpy_build_data_request_into` builds it.
    /// Background refreshes leave out `is_first`, as the shell's do.
    fn data_request(&self, op: &str, place: Place, prev_bg: &str, with_first: bool) -> Value {
        let mut request = json!({
            "op": op,
            "cwd": self.pwd.to_string_lossy(),
            "format": "bash-prompt",
        });
        if let Some(map) = request.as_object_mut() {
            if place.is_last {
                map.insert("is_last".to_owned(), Value::Bool(true));
            }
            if with_first && place.is_first {
                map.insert("is_first".to_owned(), Value::Bool(true));
            }
            if !prev_bg.is_empty() {
                map.insert("prev_bg".to_owned(), Value::String(prev_bg.to_owned()));
            }
            if op == "lang"
                && let Some(venv) = forwarded_venv()
            {
                map.insert("virtual_env".to_owned(), Value::String(venv));
            }
        }
        request
    }

    fn refresh(&self, op: &str, suffix: &str, extra: &str, place: Place, prev_bg: &str) {
        let request = self.data_request(op, place, prev_bg, false);
        self.link
            .maybe_refresh(&request, [op, suffix, extra], &self.pwd);
    }

    /// `__gpy_segment_bg`: the background the next chevron starts from.
    fn segment_bg(&self, segment: &str, git_oneshot: bool) -> String {
        let var = |name: &str, default: &str| self.vars.or_nonempty(name, default).to_owned();
        match segment {
            "git" => {
                let recorded = (!git_oneshot)
                    .then_some(self.git_root.as_deref())
                    .flatten()
                    .and_then(|root| {
                        InstantPromptCache::cache_file_for_dir(
                            root,
                            "git_bg",
                            None,
                            PromptDialect::BashPrompt,
                        )
                        .ok()
                    })
                    .and_then(|path| std::fs::read_to_string(path).ok())
                    .filter(|bg| !bg.is_empty());
                recorded.unwrap_or_else(|| var("__color_git_clean_bg", ""))
            }
            "language" => var("__color_language_bg", ""),
            "directory" => var("__color_directory_bg", ""),
            "duration" => var("__color_duration_bg", ""),
            "clock" => var("__color_clock_bg", ""),
            "status" if self.args.status == 0_i32 => var("__color_status_ok_bg", "green"),
            "status" => var("__color_status_fail_bg", "red"),
            _ => String::new(),
        }
    }

    /// Render `response` at `place` after a segment with background
    /// `prev_bg`, as the agent's `render_response` does; empty on failure.
    fn render_response(&self, response: &Response, place: Place, prev_bg: &str) -> String {
        let format = Format::BashPrompt;
        let position =
            SegmentPosition::new(IsLast::from(place.is_last), IsFirst::from(place.is_first));
        let directory = DirectoryEnv::for_response(response, format, &self.config.ui.directory);
        let ctx = RenderContext::new(self.config, self.theme, position)
            .with_palette(self.palette.clone())
            .with_prev_colors(None, parse_color(prev_bg).ok())
            .with_directory(&directory);
        create_formatter(format)
            .render(response, &ctx)
            .unwrap_or_default()
    }
}

/// `__gpy_cache_variant_suffix`: `git`, `git_first`, `git_last`, `git_first_last`.
fn variant_suffix(base: &str, place: Place) -> String {
    let mut suffix = base.to_owned();
    if place.is_first {
        suffix.push_str("_first");
    }
    if place.is_last {
        suffix.push_str("_last");
    }
    suffix
}

/// Whether the agent's presence marker for `base` exists under `key`.
fn marker_present(key: &Path, base: &str) -> bool {
    InstantPromptCache::presence_marker_for_dir(key, base, PromptDialect::BashPrompt)
        .is_ok_and(|path| path.is_file())
}

/// One instant-cache read (`__gpy_read_instant_cache`).
struct Entry {
    text: String,
    /// Older than the shell-side TTL.
    stale: bool,
    /// Served from the `.none` file for a request with a real `prev_bg`.
    variant: bool,
}

/// Read the entry for `suffix` under `key`, falling back to `.none`.
fn read_entry(key: &Path, suffix: &str, prev_bg: &str, base: &str) -> Option<Entry> {
    let dialect = PromptDialect::BashPrompt;
    let exact = InstantPromptCache::cache_file_for_dir(
        key,
        suffix,
        (!prev_bg.is_empty()).then_some(prev_bg),
        dialect,
    )
    .ok()?;
    let (path, variant) = if !exact.is_file() && !prev_bg.is_empty() {
        (
            InstantPromptCache::cache_file_for_dir(key, suffix, None, dialect).ok()?,
            true,
        )
    } else {
        (exact, false)
    };
    let metadata = std::fs::metadata(&path)
        .ok()
        .filter(std::fs::Metadata::is_file)?;
    let (ttl_var, default_ttl) = if base == "git" {
        ("GPY_GIT_INSTANT_CACHE_TTL_SECONDS", 5)
    } else {
        ("GPY_LANGUAGE_CACHE_TTL_SECONDS", 30)
    };
    let ttl = env(ttl_var)
        .filter(|v| v.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default_ttl);
    let stale = metadata
        .modified()
        .ok()
        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
        .is_some_and(|m| link::now_secs().saturating_sub(m.as_secs()) >= ttl);
    let text = std::fs::read_to_string(&path).ok()?;
    Some(Entry {
        text: text.trim_end_matches('\n').to_owned(),
        stale,
        variant,
    })
}

/// `__gpy_segment_language_detect`: a `.git` or a language marker file in
/// `$PWD` or an ancestor (the logical path, walked by string).
fn lang_detect(pwd: &Path) -> bool {
    let markers = crate::language::metadata::marker_file_names();
    if markers.is_empty() {
        return true;
    }
    pwd.ancestors().any(|dir| {
        dir.join(".git").exists() || markers.iter().any(|marker| dir.join(marker).is_file())
    })
}

/// `__gpy_forwarded_venv_into`: `$VIRTUAL_ENV`, else a non-base conda prefix.
fn forwarded_venv() -> Option<String> {
    env("VIRTUAL_ENV").or_else(|| {
        let prefix = env("CONDA_PREFIX")?;
        let name = env("CONDA_DEFAULT_ENV")?;
        (name != "base").then_some(prefix)
    })
}
