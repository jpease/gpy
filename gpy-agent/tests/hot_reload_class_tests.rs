//! HOT-RELOAD class test.
//!
//! Invariant: for every way config/theme/palette can change on disk while the
//! agent runs, once the reload settles, what the agent exported
//! (`theme-export.{fish,bash,zsh}`) equals what a FRESH load from disk would
//! produce. The fresh side is computed independently with the public loaders
//! plus the export generator; it never reads back the agent's own state.
//!
//! One `const` row per change mechanism, all driven through one
//! harness (`Agent::new()` against temp `XDG_*` dirs, the production wiring).
//!
//! Extension points (later issues add one row each): symlinked config.toml
//! (#720), palette file edit (#772), poll-thread stop latency (#778),
//! invalid-then-fixed / deleted config.toml (#788), keys removed on theme
//! switch (#791). Instant-cache comparison joins the harness once it can seed
//! a warm repo (`Agent` exposes no handle to its git cache).

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::str_to_string)]
#![allow(clippy::default_numeric_fallback)]
#![cfg(unix)]

use gpy_agent::agent::Agent;
use gpy_agent::cache::theme_export::write_theme_export_to_dir;
use gpy_agent::config::defaults::DEFAULT_THEME_CONTENT;
use gpy_agent::config::loader::load_config_from_file;
use gpy_agent::theme::ThemeManager;
use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, Signal, sigaction};
use serial_test::serial;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tempfile::TempDir;

static DOORBELL_COUNTER: AtomicUsize = AtomicUsize::new(0);

extern "C" fn count_doorbell(_signal: i32) {
    DOORBELL_COUNTER.fetch_add(1, Ordering::Relaxed);
}

const SHELL_EXPORTS: [&str; 3] = ["theme-export.fish", "theme-export.bash", "theme-export.zsh"];
/// Theme debounce is 5 s in production; allow generous headroom under load.
const SETTLE_DEADLINE: Duration = Duration::from_secs(25);
/// Extra time after settling to catch a late stale overwrite.
const QUIET_PERIOD: Duration = Duration::from_millis(2500);

const ACTIVE_THEME: &str = "active";
const OTHER_THEME: &str = "other";

/// One way config/theme/palette can change on disk.
struct Mechanism {
    name: &'static str,
    /// Runs once before the agent starts, to shape the on-disk layout.
    setup: Option<fn(&Harness)>,
    apply: fn(&Harness),
    /// Exact number of reload doorbells expected once settled, if pinned.
    doorbells: Option<usize>,
}

/// The table: one `const` row per mechanism plus one `#[test]` below that runs
/// it. Add a row here (and a test) to cover a new way the disk can change.
const EDIT_ACTIVE_THEME: Mechanism = Mechanism {
    name: "edit active theme file in place (#710)",
    setup: None,
    apply: edit_active_theme_in_place,
    doorbells: Some(1),
};
const SWITCH_THEME_BY_CONFIG: Mechanism = Mechanism {
    name: "switch theme by name via config (regression guard)",
    setup: None,
    apply: switch_theme_by_config,
    doorbells: None,
};
const EDIT_CONFIG_IN_PLACE: Mechanism = Mechanism {
    name: "edit config.toml in place",
    setup: None,
    apply: edit_config_in_place,
    doorbells: None,
};
const EDIT_SYMLINKED_CONFIG_TARGET: Mechanism = Mechanism {
    name: "edit the target of a symlinked config.toml (#720)",
    setup: Some(symlink_config_into_dotfiles),
    apply: edit_symlinked_config_target,
    doorbells: None,
};
const RETARGET_SYMLINKED_CONFIG: Mechanism = Mechanism {
    name: "retarget a symlinked config.toml (#720)",
    setup: Some(symlink_config_into_dotfiles),
    apply: retarget_symlinked_config,
    doorbells: None,
};

fn edit_active_theme_in_place(h: &Harness) {
    h.write_theme(ACTIVE_THEME, "magenta");
}

fn switch_theme_by_config(h: &Harness) {
    h.write_config(OTHER_THEME, true);
}

fn edit_config_in_place(h: &Harness) {
    h.write_config(ACTIVE_THEME, false);
}

fn symlink_config_into_dotfiles(h: &Harness) {
    let dotfiles = h.dotfiles_dir();
    fs::create_dir_all(&dotfiles).expect("create dotfiles dir");
    let target = dotfiles.join("gpy-config.toml");
    fs::rename(h.config_path(), &target).expect("move config into dotfiles");
    std::os::unix::fs::symlink(&target, h.config_path()).expect("symlink config");
}

fn edit_symlinked_config_target(h: &Harness) {
    fs::write(
        h.dotfiles_dir().join("gpy-config.toml"),
        format!("[ui]\ntheme = \"{ACTIVE_THEME}\"\n\n[git]\nenabled = false\n"),
    )
    .expect("write symlink target");
}

fn retarget_symlinked_config(h: &Harness) {
    let dotfiles = h.dotfiles_dir();
    let other = dotfiles.join("other-config.toml");
    fs::write(
        &other,
        format!("[ui]\ntheme = \"{ACTIVE_THEME}\"\n\n[git]\nenabled = false\n"),
    )
    .expect("write second target");
    let staged = dotfiles.join("staged-link");
    std::os::unix::fs::symlink(&other, &staged).expect("stage link");
    fs::rename(&staged, h.config_path()).expect("retarget link");
}

struct Harness {
    _temp: TempDir,
    config_home: PathBuf,
    cache_home: PathBuf,
    prev_env: Vec<(&'static str, Option<OsString>)>,
    prev_sigurg: SigAction,
}

impl Harness {
    fn new() -> Self {
        let temp = TempDir::new().expect("tempdir");
        let root = temp.path().to_path_buf();
        let config_home = root.join("config");
        let cache_home = root.join("cache");
        let runtime = root.join("run");
        for dir in [
            &config_home.join("gpy").join("themes"),
            &cache_home.join("gpy"),
            &runtime,
        ] {
            fs::create_dir_all(dir).expect("create dir");
        }
        let vars: [(&'static str, Option<&Path>); 6] = [
            ("HOME", Some(&root)),
            ("XDG_CONFIG_HOME", Some(&config_home)),
            ("XDG_CACHE_HOME", Some(&cache_home)),
            ("XDG_RUNTIME_DIR", Some(&runtime)),
            ("GPY_CONFIG_PATH", None),
            ("GPY_DISABLE_WATCHER", None),
        ];
        let mut prev_env = Vec::new();
        for (key, value) in vars {
            prev_env.push((key, std::env::var_os(key)));
            unsafe {
                match value {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
        }
        DOORBELL_COUNTER.store(0, Ordering::Relaxed);
        let handler = SigAction::new(
            SigHandler::Handler(count_doorbell),
            SaFlags::empty(),
            SigSet::empty(),
        );
        let prev_sigurg = unsafe { sigaction(Signal::SIGURG, &handler) }.expect("install SIGURG");
        let harness = Self {
            _temp: temp,
            config_home,
            cache_home,
            prev_env,
            prev_sigurg,
        };
        harness.write_theme(ACTIVE_THEME, "white");
        harness.write_theme(OTHER_THEME, "cyan");
        harness.write_config(ACTIVE_THEME, true);
        harness
    }

    fn gpy_config_dir(&self) -> PathBuf {
        self.config_home.join("gpy")
    }

    /// A directory outside the XDG tree, standing in for a dotfiles repo.
    fn dotfiles_dir(&self) -> PathBuf {
        self.config_home.with_file_name("dotfiles")
    }

    fn config_path(&self) -> PathBuf {
        self.gpy_config_dir().join("config.toml")
    }

    /// A full theme whose only distinguishing field is the clock text colour,
    /// which `theme export` emits as `__color_clock_fg`.
    fn write_theme(&self, name: &str, clock_text_color: &str) {
        let content = DEFAULT_THEME_CONTENT.replacen(
            "text_color = \"white\"\nbg_color = \"black\"\ntime_format",
            &format!("text_color = \"{clock_text_color}\"\nbg_color = \"black\"\ntime_format"),
            1,
        );
        assert!(
            content.contains(&format!(
                "text_color = \"{clock_text_color}\"\nbg_color = \"black\"\ntime_format"
            )),
            "theme anchor must match the default theme"
        );
        let path = self
            .gpy_config_dir()
            .join("themes")
            .join(format!("{name}.toml"));
        fs::write(path, content).expect("write theme");
    }

    fn write_config(&self, theme: &str, git_enabled: bool) {
        fs::write(
            self.config_path(),
            format!("[ui]\ntheme = \"{theme}\"\n\n[git]\nenabled = {git_enabled}\n"),
        )
        .expect("write config");
    }

    /// What a fresh load from disk exports, computed with the public loaders.
    fn fresh_exports(&self) -> Vec<String> {
        let config = load_config_from_file(&self.config_path().display().to_string())
            .expect("fresh config load");
        let theme_manager = ThemeManager::new(config.ui.theme.as_str()).expect("fresh theme load");
        let scratch = TempDir::new().expect("scratch dir");
        write_theme_export_to_dir(scratch.path(), &theme_manager, &config).expect("fresh export");
        SHELL_EXPORTS
            .iter()
            .map(|file| fs::read_to_string(scratch.path().join(file)).expect("read fresh export"))
            .collect()
    }

    /// What the agent has exported to its cache dir (empty string if absent).
    fn agent_exports(&self) -> Vec<String> {
        SHELL_EXPORTS
            .iter()
            .map(|file| {
                fs::read_to_string(self.cache_home.join("gpy").join(file)).unwrap_or_default()
            })
            .collect()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        unsafe {
            let _ = sigaction(Signal::SIGURG, &self.prev_sigurg);
            for (key, value) in self.prev_env.drain(..) {
                match value {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
}

fn wait_until(deadline: Duration, predicate: impl Fn() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if predicate() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    predicate()
}

fn run_mechanism(mechanism: &Mechanism) {
    let harness = Harness::new();
    if let Some(setup) = mechanism.setup {
        setup(&harness);
    }
    let before = harness.fresh_exports();

    let agent = Agent::new().expect("agent starts");
    agent.register_test_client(std::process::id(), harness.config_home.as_path());
    // Let the watchers arm before touching the disk.
    std::thread::sleep(Duration::from_millis(500));
    DOORBELL_COUNTER.store(0, Ordering::Relaxed);

    (mechanism.apply)(&harness);
    let expected = harness.fresh_exports();
    assert_ne!(
        before, expected,
        "[{}] the change must alter the fresh export or the row is vacuous",
        mechanism.name
    );

    let settled = wait_until(SETTLE_DEADLINE, || harness.agent_exports() == expected);
    assert!(
        settled,
        "[{}] exports never matched a fresh load from disk.\nagent fish export:\n{}\nfresh fish export:\n{}",
        mechanism.name,
        harness.agent_exports().first().cloned().unwrap_or_default(),
        expected.first().cloned().unwrap_or_default()
    );

    // A late reload from a second watcher must not move the exports back.
    std::thread::sleep(QUIET_PERIOD);
    assert_eq!(
        harness.agent_exports(),
        expected,
        "[{}] exports drifted from a fresh load after settling",
        mechanism.name
    );

    if let Some(rings) = mechanism.doorbells {
        assert_eq!(
            DOORBELL_COUNTER.load(Ordering::Relaxed),
            rings,
            "[{}] reload doorbell count",
            mechanism.name
        );
    }
    drop(agent);
}

#[test]
#[serial]
fn hot_reload_edit_active_theme_in_place() {
    run_mechanism(&EDIT_ACTIVE_THEME);
}

#[test]
#[serial]
fn hot_reload_switch_theme_by_config() {
    run_mechanism(&SWITCH_THEME_BY_CONFIG);
}

#[test]
#[serial]
fn hot_reload_edit_config_in_place() {
    run_mechanism(&EDIT_CONFIG_IN_PLACE);
}

#[test]
#[serial]
fn hot_reload_edit_symlinked_config_target() {
    run_mechanism(&EDIT_SYMLINKED_CONFIG_TARGET);
}

#[test]
#[serial]
fn hot_reload_retarget_symlinked_config() {
    run_mechanism(&RETARGET_SYMLINKED_CONFIG);
}
