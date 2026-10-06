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
//! keys removed on theme switch (#791). Instant-cache comparison joins the
//! harness once it can seed a warm repo (`Agent` exposes no handle to its git
//! cache).

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::str_to_string)]
#![allow(clippy::default_numeric_fallback)]
#![cfg(unix)]

use gpy_agent::agent::Agent;
use gpy_agent::cache::theme_export::write_theme_export_to_dir;
use gpy_agent::config::Config;
use gpy_agent::config::defaults::DEFAULT_THEME_CONTENT;
use gpy_agent::config::loader::load_config_from_file;
use gpy_agent::palette::active_palette;
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
const ACTIVE_PALETTE: &str = "mine";

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

const EDIT_ACTIVE_PALETTE: Mechanism = Mechanism {
    name: "edit the active palette file in place (#772)",
    setup: None,
    apply: edit_active_palette_in_place,
    doorbells: Some(1),
};
const REAPPLY_SAME_PALETTE: Mechanism = Mechanism {
    name: "rewrite config.toml with the same palette name after a palette edit (#772)",
    setup: None,
    apply: reapply_same_palette,
    doorbells: None,
};

const INVALID_THEN_FIXED_CONFIG: Mechanism = Mechanism {
    name: "fix an invalid lower-priority config.toml the agent started without (#788)",
    setup: Some(invalid_home_config_only),
    apply: fix_home_config,
    doorbells: None,
};
const DELETE_CONFIG: Mechanism = Mechanism {
    name: "delete the active config.toml (#788)",
    setup: None,
    apply: delete_active_config,
    doorbells: None,
};
const HIGHER_PRIORITY_CONFIG_APPEARS: Mechanism = Mechanism {
    name: "create a higher-priority config.toml while running on a lower one (#788)",
    setup: Some(config_lives_under_home),
    apply: create_xdg_config,
    doorbells: None,
};
const DELETE_CONFIG_WITH_LOCAL_GPY_TOML: Mechanism = Mechanism {
    name: "delete the active config.toml with a .gpy.toml in the agent's cwd (#733)",
    setup: Some(local_gpy_toml_in_cwd),
    apply: delete_active_config,
    doorbells: None,
};

fn invalid_home_config_only(h: &Harness) {
    fs::remove_file(h.config_path()).expect("remove xdg config");
    let home_config = h.home_config_path();
    fs::create_dir_all(home_config.parent().expect("home config dir")).expect("home config dir");
    fs::write(
        home_config,
        format!("[ui]\ntheme = \"{ACTIVE_THEME}\"\n\n[git]\ntimeout_seconds = 0\n"),
    )
    .expect("write invalid config");
}

fn fix_home_config(h: &Harness) {
    Harness::write_config_at(&h.home_config_path(), OTHER_THEME, true);
}

fn delete_active_config(h: &Harness) {
    fs::remove_file(h.config_path()).expect("delete config");
}

/// A `.gpy.toml` in the directory the agent starts from, naming a theme that
/// differs from both the active config and the defaults. It is not a config
/// source (#733), so it must never become active.
fn local_gpy_toml_in_cwd(h: &Harness) {
    std::env::set_current_dir(&h.home).expect("enter launch dir");
    Harness::write_config_at(&h.home.join(".gpy.toml"), OTHER_THEME, true);
}

fn config_lives_under_home(h: &Harness) {
    let home_config = h.home_config_path();
    fs::create_dir_all(home_config.parent().expect("home config dir")).expect("home config dir");
    fs::rename(h.config_path(), home_config).expect("move config under home");
}

fn create_xdg_config(h: &Harness) {
    h.write_config(OTHER_THEME, true);
}

fn edit_active_palette_in_place(h: &Harness) {
    h.write_palette("#123456");
}

fn reapply_same_palette(h: &Harness) {
    h.write_palette("#123456");
    h.write_config(ACTIVE_THEME, true);
}

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
    home: PathBuf,
    config_home: PathBuf,
    cache_home: PathBuf,
    prev_env: Vec<(&'static str, Option<OsString>)>,
    prev_cwd: PathBuf,
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
            &config_home.join("gpy").join("palettes"),
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
            home: root,
            config_home,
            cache_home,
            prev_env,
            prev_cwd: std::env::current_dir().expect("current dir"),
            prev_sigurg,
        };
        harness.write_theme(ACTIVE_THEME, "white");
        harness.write_theme(OTHER_THEME, "cyan");
        harness.write_palette("red");
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

    /// The lower-priority candidate under `$HOME/.config/gpy`; `XDG_CONFIG_HOME`
    /// points elsewhere, so it is shadowed whenever the XDG file exists.
    fn home_config_path(&self) -> PathBuf {
        self.home.join(".config").join("gpy").join("config.toml")
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

    fn write_palette(&self, red: &str) {
        let path = self
            .gpy_config_dir()
            .join("palettes")
            .join(format!("{ACTIVE_PALETTE}.toml"));
        fs::write(
            path,
            format!(
                "name = \"{ACTIVE_PALETTE}\"\n\n[colors]\nred = \"{red}\"\ngreen = \"green\"\n"
            ),
        )
        .expect("write palette");
    }

    fn write_config(&self, theme: &str, git_enabled: bool) {
        Self::write_config_at(&self.config_path(), theme, git_enabled);
    }

    fn write_config_at(path: &Path, theme: &str, git_enabled: bool) {
        fs::write(
            path,
            format!(
                "[ui]\ntheme = \"{theme}\"\npalette = \"{ACTIVE_PALETTE}\"\n\n[git]\nenabled = {git_enabled}\n"
            ),
        )
        .expect("write config");
    }

    /// The config file the agent resolves: the highest-priority existing
    /// candidate (`XDG_CONFIG_HOME`, then `HOME`), if any.
    fn active_config_file(&self) -> Option<PathBuf> {
        [self.config_path(), self.home_config_path()]
            .into_iter()
            .find(|path| path.exists())
    }

    /// What a fresh load from disk gives: the active file, or built-in
    /// defaults when no candidate exists.
    fn fresh_config(&self) -> Config {
        self.active_config_file()
            .map_or_else(Config::default, |path| {
                load_config_from_file(&path.display().to_string()).expect("fresh config load")
            })
    }

    /// What the agent starts with: like [`Self::fresh_config`], except that a
    /// file which fails to load means built-in defaults (the agent's startup
    /// fallback), so a row may begin on an invalid file.
    fn startup_config(&self) -> Config {
        self.active_config_file()
            .and_then(|path| load_config_from_file(&path.display().to_string()).ok())
            .unwrap_or_default()
    }

    /// What a load of `config` exports, computed with the public loaders.
    fn fresh_exports(config: &Config) -> Vec<String> {
        let theme_manager = ThemeManager::new(config.ui.theme.as_str()).expect("fresh theme load");
        let scratch = TempDir::new().expect("scratch dir");
        write_theme_export_to_dir(scratch.path(), &theme_manager, config).expect("fresh export");
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
        let _ = std::env::set_current_dir(&self.prev_cwd);
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
    let startup_config = harness.startup_config();
    let before = (
        Harness::fresh_exports(&startup_config),
        active_palette(&startup_config),
    );

    let agent = Agent::new().expect("agent starts");
    agent.register_test_client(std::process::id(), harness.config_home.as_path());
    // Let the watchers arm before touching the disk.
    std::thread::sleep(Duration::from_millis(500));
    DOORBELL_COUNTER.store(0, Ordering::Relaxed);

    (mechanism.apply)(&harness);
    let expected_config = harness.fresh_config();
    let expected = (
        Harness::fresh_exports(&expected_config),
        active_palette(&expected_config),
    );
    assert_ne!(
        before, expected,
        "[{}] the change must alter the fresh export or the row is vacuous",
        mechanism.name
    );

    let agent_state = || (harness.agent_exports(), agent.active_palette_for_testing());
    let settled = wait_until(SETTLE_DEADLINE, || agent_state() == expected);
    assert!(
        settled,
        "[{}] exports/palette never matched a fresh load from disk.\nagent fish export:\n{}\nfresh fish export:\n{}\nagent palette: {:?}\nfresh palette: {:?}",
        mechanism.name,
        harness.agent_exports().first().cloned().unwrap_or_default(),
        expected.0.first().cloned().unwrap_or_default(),
        agent.active_palette_for_testing(),
        expected.1
    );

    // A late reload from a second watcher must not move the exports back.
    std::thread::sleep(QUIET_PERIOD);
    assert_eq!(
        agent_state(),
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

#[test]
#[serial]
fn hot_reload_edit_active_palette_in_place() {
    run_mechanism(&EDIT_ACTIVE_PALETTE);
}

/// #778: with the poll fallback in use, a theme switch by name must not wait
/// out the poll interval (the CLI gives the agent 2 s to acknowledge a reload).
/// Drives `ThemeManager` directly: `switch_theme` is what the `config_reload`
/// request runs synchronously, and it re-arms the watcher via `HotReloadSlot`.
#[test]
#[serial]
fn hot_reload_switch_theme_by_name_with_poll_fallback_is_prompt() {
    const POLL_VAR: &str = "GPY_THEME_WATCH_POLL_MS";
    let harness = Harness::new();
    let prev = std::env::var_os(POLL_VAR);
    unsafe { std::env::set_var(POLL_VAR, "250") };

    let manager = ThemeManager::new(ACTIVE_THEME).expect("theme manager");
    manager
        .start_watching(Duration::from_secs(5), None)
        .expect("start watching");
    // Let the poll thread enter its between-ticks sleep.
    std::thread::sleep(Duration::from_millis(500));

    let started = Instant::now();
    let switched = manager.switch_theme(OTHER_THEME);
    let elapsed = started.elapsed();

    unsafe {
        match prev {
            Some(v) => std::env::set_var(POLL_VAR, v),
            None => std::env::remove_var(POLL_VAR),
        }
    }
    drop(manager);
    drop(harness);

    switched.expect("switch theme");
    assert!(
        elapsed < Duration::from_millis(1000),
        "switch_theme took {elapsed:?} with the poll fallback; it must not wait out the 5 s poll interval"
    );
}

#[test]
#[serial]
fn hot_reload_reapply_same_palette() {
    run_mechanism(&REAPPLY_SAME_PALETTE);
}

/// #788: the agent starts on an unparsable lower-priority config (falling
/// back to defaults), and fixing that file must apply it.
#[test]
#[serial]
fn hot_reload_invalid_config_then_fixed() {
    run_mechanism(&INVALID_THEN_FIXED_CONFIG);
}

/// #788: deleting the active config.toml reverts the agent to defaults.
#[test]
#[serial]
fn hot_reload_delete_config() {
    run_mechanism(&DELETE_CONFIG);
}

/// #788: a higher-priority candidate appearing switches the agent to it.
#[test]
#[serial]
fn hot_reload_higher_priority_config_appears() {
    run_mechanism(&HIGHER_PRIORITY_CONFIG_APPEARS);
}

/// #733: with the active config.toml gone, the agent falls back to defaults,
/// never to a `.gpy.toml` in the directory it was launched from.
#[test]
#[serial]
fn hot_reload_delete_config_ignores_local_gpy_toml() {
    run_mechanism(&DELETE_CONFIG_WITH_LOCAL_GPY_TOML);
}
