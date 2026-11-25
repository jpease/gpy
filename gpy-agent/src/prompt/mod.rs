//! `gpy prompt`: the whole Bash, Zsh or Fish prompt from one process, so
//! the shell hook is one `gpy prompt <shell>` call and the shell keeps no
//! prompt state (#1247). Script segments run through [`script`]; the
//! shell's lifecycle (agent start, registration, supervisor, exit) lives in
//! [`session`].

mod dialect;
mod link;
mod pill;
mod script;
mod session;
mod vars;

#[cfg(test)]
mod tests;

pub use dialect::Dialect;

use crate::Result;
use crate::agent::oneshot::{OneshotKind, OneshotRequest, handle_oneshot_request};
use crate::cache::InstantPromptCache;
use crate::config::{Config, loader::load_config};
use crate::env_config::{
    AGENT_ENABLED, GIT_INSTANT_CACHE_TTL_SECONDS, LANGUAGE_CACHE_TTL_SECONDS, SHOW_STATUS,
};
use crate::formatter::{
    ColorDepth, DirectoryEnv, IsFirst, IsLast, RenderContext, SegmentPosition, create_formatter,
};
use crate::ipc::Response;
use crate::template::{Palette, parse_color};
use crate::theme::export::{ScriptSegmentExports, collect_script_segments};
use crate::theme::{ThemeConfig, load::load_theme};
use link::{Link, Reply};
use pill::{Depth, Paint, Place};
use script::{Inputs, ScriptArgs, Scripts};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use vars::ShellVars;

/// What the shell knows about the command that just finished.
#[derive(Debug, Clone)]
pub struct PromptArgs {
    /// Exit status of the last command.
    pub status: i32,
    /// Its duration in milliseconds; `None` when this prompt follows no
    /// command (the first prompt, an empty Enter).
    pub duration_ms: Option<u64>,
    /// The shell's PID: names its session, refresh-throttle stamps and
    /// late window.
    pub pid: u32,
    /// A token that changes with every command the shell ran: script
    /// segments run again when it does.
    pub cmd: String,
    /// A token the shell integration draws once when it loads; empty from
    /// one that predates it. Another token at the same PID is another shell
    /// (an `exec`, or a recycled PID): it starts a fresh session.
    pub instance: String,
}

/// The built-in segments `gpy prompt` draws; any other enabled name is a
/// script segment, or skipped.
const BUILTIN_SEGMENTS: [&str; 6] = [
    "clock",
    "duration",
    "language",
    "directory",
    "git",
    "status",
];

/// A rendered prompt.
#[derive(Debug)]
pub struct Rendered {
    /// `PS1`, `PROMPT` or Fish's prompt.
    pub prompt: String,
    /// Zsh: the prompt symbol alone, without the status icon (the
    /// transient prompt). Fish: the whole transient line. Empty for Bash.
    pub tail: String,
}

/// The shell is exiting: unregister it from the agent and remove the files
/// kept for it.
pub fn exit(pid: u32) {
    let config = load_config().unwrap_or_default();
    session::exit(&Link::new(agent_enabled(&config), pid), pid);
}

/// Render the prompt for the current directory (`$PWD`).
///
/// # Errors
/// Returns an error if neither the configured nor the `default` theme loads.
pub fn render(dialect: Dialect, args: PromptArgs, shell_version: &str) -> Result<Rendered> {
    let config = load_config().unwrap_or_else(|e| {
        crate::debug::warn_fallback("Config loading", "using defaults", &e);
        Config::default()
    });
    // Plugin discovery reads every manifest: only when a segment needs it.
    let exports = if config
        .effective_segments()
        .all(|segment| BUILTIN_SEGMENTS.contains(&segment))
    {
        ScriptSegmentExports::default()
    } else {
        collect_script_segments(&crate::plugin::discover_plugins(), &config)
    };
    let enabled = agent_enabled(&config);
    let mut session = session::Session::open(args.pid, &args.instance, enabled);
    let script_duration = session.script_duration(args.duration_ms);
    let theme = load_theme(config.ui.theme.as_str()).or_else(|_| load_theme("default"))?;
    let palette = crate::palette::active_palette(&config);
    let vars = ShellVars::new(crate::theme::export::prompt_shell_values(
        &theme, &config, &palette,
    ));
    let pwd = logical_pwd();
    let scripts = if exports.segments.is_empty() {
        None
    } else {
        session.bash().map(|bash| {
            Scripts::run(
                &exports.sh,
                exports.segments,
                &ScriptArgs {
                    bash,
                    cmd: &args.cmd,
                    reload: session.reload,
                },
                args.pid,
                &Inputs {
                    pwd: &pwd.to_string_lossy(),
                    duration_ms: script_duration,
                    prompt_icons: vars.or("__prompt_icons", ""),
                },
            )
        })
    };
    let canonical = std::fs::canonicalize(&pwd).ok();
    let git_root = canonical.as_deref().and_then(find_git_root);
    let mut render = Render {
        dialect,
        paint: Paint {
            depth: Depth::from(ColorDepth::detect()),
            dialect,
        },
        link: Link::new(enabled, args.pid),
        config: &config,
        theme: &theme,
        palette,
        vars,
        args,
        pwd,
        canonical,
        git_root,
        scripts,
        oneshot_used: false,
        is_root: nix::unistd::geteuid().is_root(),
    };
    let rendered = render.prompt();
    // After the render, so a cold start never delays this prompt.
    session.finish(
        &render.link,
        &config,
        &session::Shell {
            pid: render.args.pid,
            name: dialect.request_shell(),
            version: shell_version,
        },
        &render.pwd.to_string_lossy(),
    );
    Ok(rendered)
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

/// `__gpy_agent_enabled`: `GPY_AGENT_ENABLED`, else `agent.enabled`.
fn agent_enabled(config: &Config) -> bool {
    AGENT_ENABLED
        .flag_override()
        .unwrap_or(config.agent.enabled)
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
    dialect: Dialect,
    paint: Paint,
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
    /// The script segments after this prompt's run.
    scripts: Option<Scripts>,
    /// One in-process oneshot per render (`__gpy_oneshot_claim`).
    oneshot_used: bool,
    /// Effective UID 0: the root symbol instead of the `character` segment.
    is_root: bool,
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
    fn prompt(&mut self) -> Rendered {
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
        let root = self.is_root;
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
        let prompt_tail = |show_status: bool| {
            pill::prompt_tail(&self.vars, self.paint, self.args.status, root, show_status)
        };
        let (symbol, symbol_alone) = if character.is_empty() {
            (prompt_tail(SHOW_STATUS.flag()), prompt_tail(false))
        } else {
            (character.clone(), character)
        };
        out.push_str(&symbol);
        let has_clock = self
            .config
            .effective_segments()
            .any(|segment| segment == "clock");
        // What stays in scrollback once the line is submitted: transient on,
        // the clock time and the symbol; off with the clock, the whole prompt
        // (the time it was submitted); off without it, nothing to swap in.
        let tail = match self.dialect {
            Dialect::Zsh | Dialect::Fish if self.config.ui.transient_prompt => {
                self.transient_line(&symbol_alone, has_clock)
            }
            Dialect::Zsh | Dialect::Fish if has_clock => out.clone(),
            Dialect::Bash | Dialect::Zsh | Dialect::Fish => String::new(),
        };
        Rendered { prompt: out, tail }
    }

    /// The transient line: the clock time (when the clock is enabled) and
    /// the prompt symbol.
    fn transient_line(&self, symbol: &str, has_clock: bool) -> String {
        let mut line = String::new();
        if self.vars.or_nonempty("__gpy_add_newline", "1") != "0" {
            line.push('\n');
        }
        if has_clock {
            line.push_str(&pill::fg_text(
                self.paint,
                self.vars.or("__color_clock_fg", ""),
                &format!(
                    "{}{}",
                    self.dialect.escape(self.vars.or("__prompt_time_pad", " ")),
                    self.dialect.clock_token(&pill::clock_time_spec(&self.vars))
                ),
            ));
            // Themes whose symbol has no leading space (starship) still need
            // one between the time and the symbol.
            let shown = pill::strip_sgr(symbol).replace("%{", "").replace("%}", "");
            if !shown.starts_with(' ') {
                line.push(' ');
            }
        }
        line.push_str(symbol);
        line
    }

    /// `__gpy_segment_would_render`: detect, then omit.
    fn would_render(&self, segment: &str) -> bool {
        match segment {
            "directory" | "clock" | "status" => true,
            "duration" => self
                .args
                .duration_ms
                .is_some_and(|ms| ms >= self.duration_threshold()),
            // Drawn from the cache, the agent or, when it is off, down or
            // refuses, the in-process oneshot.
            "git" => self.git_root.is_some(),
            "language" => {
                if !lang_detect(&self.pwd) {
                    return false;
                }
                let present = self
                    .lang_key()
                    .is_some_and(|key| marker_present(&key, "lang", self.dialect));
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
            _ => self
                .scripts
                .as_ref()
                .is_some_and(|scripts| scripts.visible(segment)),
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
            _ => Drawn::plain(self.scripts.as_ref().map_or_else(String::new, |scripts| {
                scripts.pill(&self.vars, self.paint, segment, place)
            })),
        }
    }

    /// `__gpy_segment_clock`: the theme's clock template, else the local pill
    /// with the shell's live `\D{…}` or `%D{…}`, or Fish's time now.
    fn clock(&self, place: Place, prev_bg: &str) -> String {
        let spec = pill::clock_time_spec(&self.vars);
        if self.vars.is_set("__clock_format") {
            let rendered = self.render_response(
                &Response::Clock {
                    shell: self.dialect.shell(),
                },
                place,
                prev_bg,
            );
            if !rendered.is_empty() {
                // Fish's pill carries the bare spec where the time goes.
                return if self.dialect == Dialect::Fish {
                    rendered.replacen(&spec, &self.dialect.clock_token(&spec), 1)
                } else {
                    rendered
                };
            }
        }
        let content = format!(
            "{}{}",
            self.paint.escape(self.vars.or("__prompt_time_pad", " ")),
            self.dialect.clock_token(&spec)
        );
        pill::section_standalone(
            &self.vars,
            self.paint,
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
        let icon = self
            .paint
            .escape(&format!(" {}", pill::status_icon(&self.vars, success)));
        pill::section_standalone(&self.vars, self.paint, bg, fg, &icon, place)
    }

    /// `__gpy_segment_git`: serve the instant cache, ask the agent on a cold
    /// or wrong-variant miss, refresh a stale entry, and fall back to an
    /// in-process oneshot when the agent is off, down or refuses.
    fn git(&mut self, place: Place, prev_bg: &str) -> Drawn {
        let suffix = variant_suffix("git", place);
        let Some(root) = self.git_root.clone() else {
            return Drawn::plain(String::new());
        };
        let Some(entry) = read_entry(&root, &suffix, prev_bg, "git", self.dialect) else {
            if !self.link.enabled() {
                return self.oneshot_git(place);
            }
            let request = self.data_request("git", place, prev_bg, true);
            return match self.link.send(&request, Some(link::cold_timeout_ms())) {
                Reply::Text(text) => Drawn::plain(text),
                // Nothing listens, or a refusal for good (a denied cwd) that
                // a refresh would get too: draw it agent-free.
                Reply::Unreachable | Reply::Refused(_) => self.oneshot_git(place),
                // A live agent that is late has the request: it fills the
                // cache, and the next prompt draws it.
                Reply::Late => {
                    self.refresh("git", &suffix, "", place, prev_bg);
                    Drawn::plain(String::new())
                }
            };
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
        if self.refresh("git", &suffix, "", place, prev_bg) {
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
            .and_then(|key| read_entry(&key, &suffix, prev_bg, "lang", self.dialect))
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
            format: self.dialect.format(),
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
            "format": self.dialect.request_format(),
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

    /// Ask the agent to refresh `op`'s entry in the background; `false`
    /// when no agent listens to do it.
    fn refresh(&self, op: &str, suffix: &str, extra: &str, place: Place, prev_bg: &str) -> bool {
        let request = self.data_request(op, place, prev_bg, false);
        self.link
            .maybe_refresh(&request, [op, suffix, extra], &self.pwd)
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
                            self.dialect.prompt_dialect(),
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
            _ => self
                .scripts
                .as_ref()
                .map_or_else(String::new, |scripts| scripts.bg(&self.vars, segment)),
        }
    }

    /// Render `response` at `place` after a segment with background
    /// `prev_bg`, as the agent's `render_response` does; empty on failure.
    fn render_response(&self, response: &Response, place: Place, prev_bg: &str) -> String {
        let format = self.dialect.format();
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
fn marker_present(key: &Path, base: &str, dialect: Dialect) -> bool {
    InstantPromptCache::presence_marker_for_dir(key, base, dialect.prompt_dialect())
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
fn read_entry(
    key: &Path,
    suffix: &str,
    prev_bg: &str,
    base: &str,
    shell: Dialect,
) -> Option<Entry> {
    let dialect = shell.prompt_dialect();
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
    let ttl = if base == "git" {
        GIT_INSTANT_CACHE_TTL_SECONDS.count()
    } else {
        LANGUAGE_CACHE_TTL_SECONDS.count()
    };
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
