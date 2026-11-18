//! Script segments for `gpy prompt` (#1247): `bash/core/g_segments.bash`
//! (what is due, the cache, the declared environment, the trigger checks,
//! the circuit breaker and the pill) with its state in a per-shell session
//! file instead of shell variables. The unchanged `g_runner.bash` runs the
//! trigger scan and the scripts in one Bash child (`__gpy_g_scan_batch`, as
//! Fish runs it).
//!
//! The declared `env` and `require_env` names are read from this process's
//! environment, so only exported variables reach a script; the in-shell path
//! also forwards unexported ones.

use super::link::now_secs;
use super::pill::{self, Caps, Depth, Place};
use super::vars::ShellVars;
use crate::theme::export::ScriptSegmentExport;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::io::{Read as _, Write as _};
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// What the shell passes for its script segments.
#[derive(Debug, Clone)]
pub struct ScriptArgs {
    /// The Bash that runs `g_runner.bash`: the shell's own `$BASH`.
    pub bash: PathBuf,
    /// `bash/core/g_runner.bash` of the loaded integration.
    pub runner: PathBuf,
    /// A token that changes with every command the shell ran (`\#`).
    pub cmd: String,
    /// The theme was (re)loaded since the last prompt: the run is due and
    /// every circuit breaker is reset (`__gpy_g_sync`).
    pub reload: bool,
}

/// The second timeout or over-cap result in a row disables a segment.
const STRIKES_MAX: u8 = 2;

/// Record, field and section separators of the runner's output.
const RS: char = '\u{1e}';
const US: char = '\u{1f}';
const GS: char = '\u{1d}';

/// How long the runner's output pipe may outlive the runner (a leftover
/// process holding it): its records are written before it exits.
const PIPE_GRACE: Duration = Duration::from_millis(50);

/// The per-shell state the shell path keeps in `__gpy_g_*` variables.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Session {
    /// The command token of the last prompt (`__gpy_g_cmd`).
    cmd: String,
    /// `$PWD` of the last prompt (`__gpy_g_lastpwd`).
    pwd: String,
    tokens: BTreeMap<String, TokenState>,
}

/// One segment's state, by token.
#[derive(Debug, Default, Serialize, Deserialize)]
struct TokenState {
    /// Text drawn this prompt, not escaped (`""` hides).
    text: String,
    /// Colour variant drawn this prompt.
    variant: String,
    cache: Option<Cached>,
    run_strikes: u8,
    scan_strikes: u8,
    /// Set once the circuit breaker tripped.
    off: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct Cached {
    /// The full cache key: the static key, then every declared value.
    key: String,
    /// Epoch seconds when stored.
    at: u64,
    text: String,
    variant: String,
}

/// What a render knows that a script may declare as an input.
pub(super) struct Inputs<'a> {
    /// The logical `$PWD`.
    pub(super) pwd: &'a str,
    /// The last command's duration, when one ran.
    pub(super) duration_ms: Option<u64>,
    /// `__prompt_icons` of the export.
    pub(super) prompt_icons: &'a str,
}

/// The enabled script segments and their state after this prompt's run.
pub(super) struct Scripts {
    segments: Vec<ScriptSegmentExport>,
    session: Session,
}

/// One segment the run asks the runner for.
struct Planned {
    token: String,
    key: String,
    /// Declared names and values, in runner order.
    values: Vec<(String, OsString)>,
    /// Waits for its own trigger scan to hit.
    scanned: bool,
    /// Served from its cache entry (when its scan, if any, hits).
    hit: bool,
}

impl Scripts {
    /// Load this shell's state, run what is due and store the state again.
    pub(super) fn run(
        sh: &str,
        segments: Vec<ScriptSegmentExport>,
        args: &ScriptArgs,
        pid: u32,
        inputs: &Inputs<'_>,
    ) -> Self {
        let path = session_path(pid);
        let loaded = path.as_deref().and_then(load_session);
        let fresh = loaded.is_none();
        let mut scripts = Self {
            segments: dedup(segments),
            session: loaded.unwrap_or_default(),
        };
        let listed: HashSet<&str> = scripts.segments.iter().map(|s| s.token.as_str()).collect();
        scripts
            .session
            .tokens
            .retain(|token, _| listed.contains(token.as_str()));
        for segment in &scripts.segments {
            let state = scripts
                .session
                .tokens
                .entry(segment.token.clone())
                .or_default();
            if args.reload {
                state.run_strikes = 0;
                state.scan_strikes = 0;
                state.off = false;
            }
        }
        let due = fresh
            || args.reload
            || scripts.session.cmd != args.cmd
            || scripts.session.pwd != inputs.pwd;
        if !due {
            return scripts;
        }
        scripts.session.cmd.clone_from(&args.cmd);
        inputs.pwd.clone_into(&mut scripts.session.pwd);
        // Emptied and stored first: a run cut short (Ctrl-C) leaves its
        // segments hidden and is not started again by the redraw.
        for state in scripts.session.tokens.values_mut() {
            state.text.clear();
            state.variant.clear();
        }
        if let Some(file) = &path {
            store_session(file, &scripts.session);
        }
        if sh.starts_with('/') {
            scripts.run_due(sh, args, inputs);
            if let Some(file) = &path {
                store_session(file, &scripts.session);
            }
        }
        scripts
    }

    /// `__gpy_g_run_due`: triggers, the cache, then one runner child for
    /// the scan and every miss.
    fn run_due(&mut self, sh: &str, args: &ScriptArgs, inputs: &Inputs<'_>) {
        let now = now_secs();
        let mut planned = Vec::new();
        let mut scan_words: Vec<String> = Vec::new();
        for segment in &self.segments {
            let Some(state) = self.session.tokens.get_mut(&segment.token) else {
                continue;
            };
            if state.off {
                continue;
            }
            let env_ok = segment
                .require_env
                .iter()
                .all(|name| user_env(name).is_some_and(|value| !value.is_empty()));
            let names_ok = segment
                .require_files
                .iter()
                .chain(&segment.require_folders)
                .chain(&segment.require_extensions)
                .all(|name| !name.is_empty() && !name.contains('/') && name != "." && name != "..");
            if !env_ok || !names_ok {
                state.cache = None;
                continue;
            }
            let Some((key, values)) = key_and_values(segment, inputs) else {
                continue;
            };
            let triggers = [
                &segment.require_files,
                &segment.require_folders,
                &segment.require_extensions,
            ];
            let scanned = triggers.iter().any(|list| !list.is_empty());
            if scanned {
                scan_words.push(segment.token.clone());
                scan_words.extend(triggers.iter().map(|list| list.len().to_string()));
                scan_words.extend(triggers.iter().flat_map(|list| list.iter().cloned()));
            }
            let hit = segment.ttl_secs > 0
                && state.cache.as_ref().is_some_and(|cached| {
                    cached.key == key
                        && cached.at <= now
                        && now.saturating_sub(cached.at) < u64::from(segment.ttl_secs)
                });
            planned.push(Planned {
                token: segment.token.clone(),
                key,
                values,
                scanned,
                hit,
            });
        }
        let misses: Vec<&Planned> = planned.iter().filter(|plan| !plan.hit).collect();
        if scan_words.is_empty() && misses.is_empty() {
            self.show_hits(&planned, &HashSet::new());
            return;
        }
        let output = self.spawn_runner(sh, args, &scan_words, &misses);
        let (scan, batch) = output.split_once(GS).unwrap_or((output.as_str(), ""));
        let hits = self.read_scan(scan, &planned);
        self.show_hits(&planned, &hits);
        self.read_batch(batch, &planned, &hits, now);
    }

    /// One `__gpy_g_scan_batch` child; its standard output.
    fn spawn_runner(
        &self,
        sh: &str,
        args: &ScriptArgs,
        scan_words: &[String],
        misses: &[&Planned],
    ) -> String {
        let Ok(source) = std::fs::read_to_string(&args.runner) else {
            return String::new();
        };
        let mut command = Command::new(&args.bash);
        command
            .arg("-p")
            .arg("-c")
            .arg(format!("{source}\n__gpy_g_scan_batch \"$@\""))
            .arg("gpy")
            .arg(sh)
            .arg(scan_words.len().to_string())
            .args(scan_words);
        // Only the values set here reach the runner's `__gpy_g_v<K>_<NAME>`.
        for (name, _) in std::env::vars_os() {
            if name.as_bytes().starts_with(b"__gpy_g_") {
                command.env_remove(name);
            }
        }
        for (index, plan) in misses.iter().enumerate() {
            let Some(segment) = self.segment(&plan.token) else {
                continue;
            };
            // The inherited working directory for a folder-scoped segment,
            // so `$PWD` is never in argv.
            let cwd = if segment.scope == "folder" {
                ""
            } else {
                segment.root.as_str()
            };
            command
                .arg(if plan.scanned {
                    plan.token.as_str()
                } else {
                    ""
                })
                .arg(&plan.token)
                .arg(segment.timeout_ms.to_string())
                .arg(cwd)
                .arg(&segment.command)
                .arg(&segment.require_command)
                .arg(plan.values.len().to_string());
            for (name, value) in &plan.values {
                command.arg(name);
                command.env(format!("__gpy_g_v{index}_{name}"), value);
            }
        }
        command
            .stdin(Stdio::inherit())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let Ok(mut child) = command.spawn() else {
            return String::new();
        };
        // Read while the runner runs (its output can outgrow the pipe), but
        // return once it exited: a leftover holding the pipe cannot hold the
        // prompt.
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        if let Some(mut stdout) = child.stdout.take() {
            let sink = Arc::clone(&buffer);
            std::thread::spawn(move || {
                let mut chunk = [0_u8; 8192];
                while let Ok(read) = stdout.read(&mut chunk) {
                    let Some(bytes) = chunk.get(..read).filter(|bytes| !bytes.is_empty()) else {
                        break;
                    };
                    sink.lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .extend_from_slice(bytes);
                }
                let _ = done_tx.send(());
            });
        }
        let _ = child.wait();
        let _ = done_rx.recv_timeout(PIPE_GRACE);
        let bytes = std::mem::take(&mut *buffer.lock().unwrap_or_else(PoisonError::into_inner));
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// The scan records: a hit resets the scan strikes and lets the segment
    /// show; a timeout is a strike. A scanned segment that did not hit
    /// loses its cache entry.
    fn read_scan(&mut self, scan: &str, planned: &[Planned]) -> HashSet<String> {
        let mut hits = HashSet::new();
        for record in scan.split(RS).filter(|record| !record.is_empty()) {
            let Some((token, status)) = record.split_once(US) else {
                continue;
            };
            if !planned
                .iter()
                .any(|plan| plan.scanned && plan.token == token)
            {
                continue;
            }
            match status {
                "hit" => {
                    hits.insert(token.to_owned());
                    self.with_state(token, |state| state.scan_strikes = 0);
                }
                "miss" => self.with_state(token, |state| state.scan_strikes = 0),
                _ => self.strike(token, Strike::Scan),
            }
        }
        for plan in planned.iter().filter(|plan| plan.scanned) {
            if !hits.contains(&plan.token) {
                self.with_state(&plan.token, |state| state.cache = None);
            }
        }
        hits
    }

    /// Cache hits show their stored text.
    fn show_hits(&mut self, planned: &[Planned], hits: &HashSet<String>) {
        for plan in planned {
            if !plan.hit || (plan.scanned && !hits.contains(&plan.token)) {
                continue;
            }
            self.with_state(&plan.token, |state| {
                if let Some(cached) = &state.cache {
                    state.text.clone_from(&cached.text);
                    state.variant.clone_from(&cached.variant);
                }
            });
        }
    }

    /// The batch records, matched by token: `ok` shows and caches the text,
    /// `exit`/`req`/`c1` cache an empty one, `timeout`/`cap` are a strike.
    fn read_batch(&mut self, batch: &str, planned: &[Planned], hits: &HashSet<String>, now: u64) {
        for record in batch.split(RS).filter(|record| !record.is_empty()) {
            let mut fields = record.split(US);
            let (Some(token), Some(status)) = (fields.next(), fields.next()) else {
                continue;
            };
            let Some(plan) = planned.iter().find(|plan| {
                !plan.hit && plan.token == token && (!plan.scanned || hits.contains(token))
            }) else {
                continue;
            };
            let (text, variant) = match status {
                "ok" => {
                    let text = fields.next().unwrap_or_default();
                    let variant = fields
                        .next()
                        .filter(|v| valid_variant(v))
                        .unwrap_or_default();
                    self.with_state(token, |state| state.run_strikes = 0);
                    (text, variant)
                }
                "exit" | "req" | "c1" => ("", ""),
                "timeout" | "cap" => {
                    self.strike(token, Strike::Run);
                    continue;
                }
                _ => continue,
            };
            let ttl = self.segment(token).map_or(0, |segment| segment.ttl_secs);
            self.with_state(token, |state| {
                text.clone_into(&mut state.text);
                variant.clone_into(&mut state.variant);
                if ttl > 0 {
                    state.cache = Some(Cached {
                        key: plan.key.clone(),
                        at: now,
                        text: text.to_owned(),
                        variant: variant.to_owned(),
                    });
                }
            });
        }
    }

    /// `__gpy_g_strike`: the second strike in a row of one kind disables
    /// the segment for this shell, with one line on stderr.
    fn strike(&mut self, token: &str, kind: Strike) {
        let mut tripped = false;
        self.with_state(token, |state| {
            let count = match kind {
                Strike::Run => &mut state.run_strikes,
                Strike::Scan => &mut state.scan_strikes,
            };
            *count = count.saturating_add(1);
            if *count >= STRIKES_MAX {
                tripped = !state.off;
                state.off = true;
                state.text.clear();
            }
        });
        if let (true, Some(segment)) = (tripped, self.segment(token)) {
            // Not `eprintln!`: `clippy::print_stderr` is denied.
            let _ = writeln!(
                std::io::stderr(),
                "gpy: script segment {} disabled for this shell after {STRIKES_MAX} failures (timeout or output over 4096 bytes)",
                segment.name
            );
        }
    }

    fn with_state(&mut self, token: &str, update: impl FnOnce(&mut TokenState)) {
        if let Some(state) = self.session.tokens.get_mut(token) {
            update(state);
        }
    }

    fn segment(&self, token: &str) -> Option<&ScriptSegmentExport> {
        self.segments.iter().find(|segment| segment.token == token)
    }

    /// The segment an enabled-segment word names (case-insensitively, as
    /// the export matches them) and its state.
    fn lookup(&self, word: &str) -> Option<(&ScriptSegmentExport, &TokenState)> {
        let segment = self
            .segments
            .iter()
            .find(|segment| segment.name.eq_ignore_ascii_case(word))?;
        Some((segment, self.session.tokens.get(&segment.token)?))
    }

    /// `__gpy_g_visible`: text drawn this prompt and the breaker not tripped.
    pub(super) fn visible(&self, word: &str) -> bool {
        self.lookup(word)
            .is_some_and(|(_, state)| !state.text.is_empty() && !state.off)
    }

    /// The pill's background and foreground: `__color_<tok>_{bg,fg}`, each
    /// replaced by the variant's when set and non-empty.
    fn colors<'v>(vars: &'v ShellVars, token: &str, variant: &str) -> [&'v str; 2] {
        ["bg", "fg"].map(|side| {
            let base = vars.or(&format!("__color_{token}_{side}"), "");
            if valid_variant(variant) && !variant.is_empty() {
                vars.or_nonempty(&format!("__color_{token}_{variant}_{side}"), base)
            } else {
                base
            }
        })
    }

    /// The background the next chevron starts from after `word`.
    pub(super) fn bg(&self, vars: &ShellVars, word: &str) -> String {
        self.lookup(word)
            .map_or_else(String::new, |(segment, state)| {
                let [bg, _] = Self::colors(vars, &segment.token, &state.variant);
                bg.to_owned()
            })
    }

    /// `__gpy_g_render`: the pill of `word` at `place`.
    pub(super) fn pill(&self, vars: &ShellVars, depth: Depth, word: &str, place: Place) -> String {
        let Some((segment, state)) = self.lookup(word) else {
            return String::new();
        };
        let token = segment.token.as_str();
        let colors = Self::colors(vars, token, &state.variant);
        let mut icon = vars.or(&format!("__icon_{token}"), "");
        if valid_variant(&state.variant) && !state.variant.is_empty() {
            icon = vars.or_nonempty(&format!("__icon_{token}_{}", state.variant), icon);
        }
        let text = pill::prompt_escape(&state.text);
        let label = if icon.is_empty() {
            format!(" {text}")
        } else {
            format!(" {} {text}", pill::prompt_escape(icon))
        };
        let cap = |side: &str| {
            let prefix = format!("__gpy_segment_{token}_{side}");
            vars.get(&format!("{prefix}_icon")).map(|glyph| {
                [
                    glyph,
                    vars.or(&format!("{prefix}_icon_color"), ""),
                    vars.or(&format!("{prefix}_bg_color"), ""),
                ]
            })
        };
        let caps = Caps {
            open: cap("open"),
            close: cap("close"),
        };
        pill::section_with_caps(vars, depth, colors, &label, place, &caps)
    }
}

#[derive(Debug, Clone, Copy)]
enum Strike {
    Run,
    Scan,
}

/// Each token once, in export order.
fn dedup(segments: Vec<ScriptSegmentExport>) -> Vec<ScriptSegmentExport> {
    let mut seen = HashSet::new();
    segments
        .into_iter()
        .filter(|segment| seen.insert(segment.token.clone()))
        .collect()
}

/// `<runtime root>/refresh-throttle/<pid>.scripts`: the shell's exit
/// handler sweeps every `<pid>.*` there.
fn session_path(pid: u32) -> Option<PathBuf> {
    crate::paths::runtime_dir()
        .ok()
        .map(|root| root.join("refresh-throttle").join(format!("{pid}.scripts")))
}

fn load_session(path: &Path) -> Option<Session> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Write through a temporary file, so a reader never sees half a session.
fn store_session(path: &Path, session: &Session) {
    let Ok(json) = serde_json::to_vec(session) else {
        return;
    };
    let mut name = path.as_os_str().to_owned();
    name.push(".tmp");
    let temporary = PathBuf::from(name);
    let written = std::fs::write(&temporary, &json).is_ok()
        || (path
            .parent()
            .is_some_and(|dir| std::fs::create_dir_all(dir).is_ok())
            && std::fs::write(&temporary, &json).is_ok());
    if written && std::fs::rename(&temporary, path).is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
}

/// `[A-Za-z_][A-Za-z0-9_]*`.
fn is_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// A variant line the runner may report: `[a-z0-9_]{0,32}`.
fn valid_variant(variant: &str) -> bool {
    variant.len() <= 32
        && variant
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// A declared variable as the user's environment holds it. A `__gpy_*`
/// name is gpy state, never the user, and counts as unset.
fn user_env(name: &str) -> Option<OsString> {
    if !is_name(name) || name.starts_with("__gpy_") {
        return None;
    }
    std::env::var_os(name)
}

/// A non-empty environment value.
fn env_set(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.is_empty())
}

/// Append `|<tag><len>:<name><len>:<value>` to `key`: no field can shift
/// into its neighbour.
fn frame(key: &mut String, tag: char, name: &str, value: Option<&OsStr>) {
    use std::fmt::Write as _;
    let _ = write!(key, "|{tag}{}:{name}", name.len());
    if let Some(text) = value {
        let _ = write!(key, "{}:{}", text.as_bytes().len(), text.to_string_lossy());
    }
}

/// `__gpy_g_key`: the full cache key and the names and values the script
/// gets (declared env, inputs, then settings); `None` on an unknown input
/// or a bad name.
fn key_and_values(
    segment: &ScriptSegmentExport,
    inputs: &Inputs<'_>,
) -> Option<(String, Vec<(String, OsString)>)> {
    let mut key = segment.key.clone();
    let mut values = Vec::new();
    for name in &segment.env {
        if !is_name(name) || name.starts_with("GPY_") {
            return None;
        }
        let found = user_env(name);
        frame(
            &mut key,
            if found.is_some() { 'e' } else { 'u' },
            name,
            found.as_deref(),
        );
        if let Some(value) = found {
            values.push((name.clone(), value));
        }
    }
    let flag = |set: bool| if set { "1" } else { "0" };
    for &input in &segment.inputs {
        let (name, value) = match input {
            "pwd" => ("GPY_PWD", inputs.pwd.to_owned()),
            "is_root" => (
                "GPY_IS_ROOT",
                flag(nix::unistd::geteuid().is_root()).to_owned(),
            ),
            "is_sudo" => ("GPY_IS_SUDO", flag(env_set("SUDO_USER")).to_owned()),
            "is_ssh" => (
                "GPY_IS_SSH",
                flag(
                    ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
                        .into_iter()
                        .any(env_set),
                )
                .to_owned(),
            ),
            "cmd_duration" => (
                "GPY_CMD_DURATION",
                inputs
                    .duration_ms
                    .map_or_else(String::new, |ms| ms.to_string()),
            ),
            "prompt_icons" => ("GPY_PROMPT_ICONS", inputs.prompt_icons.to_owned()),
            _ => return None,
        };
        frame(&mut key, 'i', input, Some(OsStr::new(&value)));
        values.push((name.to_owned(), OsString::from(value)));
    }
    // Settings are in the static key already; the runner sets them last, so
    // they override any inherited GPY_OPT_*.
    for entry in &segment.opts {
        let (name, value) = entry.split_once('=')?;
        if !name.starts_with("GPY_OPT_") || !is_name(name) || name.len() > 72 {
            return None;
        }
        values.push((name.to_owned(), OsString::from(value)));
    }
    Some((key, values))
}
