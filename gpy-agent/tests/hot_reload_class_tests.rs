//! Hot-reload consistency tests.
//!
//! Invariant: for every way config/theme/palette can change on disk while the
//! agent runs, once the reload settles, what the agent exported
//! (`theme-export.{fish,bash,zsh}`) equals what a FRESH load from disk would
//! produce. The fresh side is computed independently with the public loaders
//! plus the export generator; it never reads back the agent's own state.
//!
//! One `const` row per change mechanism, all driven through one
//! harness (`Agent::new()` against temp `XDG_*` dirs, the production wiring).
//! The instant cache is not compared: `Agent` exposes no handle to its git
//! cache, so the harness cannot seed a warm repo.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]
#![cfg(unix)]

#[path = "common/env_guard.rs"]
mod env_guard;
#[path = "common/sigurg.rs"]
mod sigurg;
#[path = "common/wait.rs"]
mod wait;

use env_guard::EnvGuard;
use gpy_agent::agent::Agent;
use gpy_agent::cache::theme_export::write_theme_export_to_dir;
use gpy_agent::config::Config;
use gpy_agent::config::defaults::DEFAULT_THEME_CONTENT;
use gpy_agent::config::loader::load_config_from_file;
use gpy_agent::palette::active_palette;
use gpy_agent::theme::ThemeManager;
use serial_test::serial;
use sigurg::SigurgCounter;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tempfile::TempDir;
use wait::wait_until;

const SHELL_EXPORTS: [&str; 3] = ["theme-export.fish", "theme-export.bash", "theme-export.zsh"];
/// Generous headroom over the 1 s watcher debounce under load.
const SETTLE_DEADLINE: Duration = Duration::from_secs(25);
/// Extra time after settling to catch a late stale overwrite.
const QUIET_PERIOD: Duration = Duration::from_millis(2500);
/// Doorbells are counted this long after the change, so a late ring from
/// either 1 s watcher, whose flush deadline drains a change within 2 s, lands.
const DOORBELL_WINDOW: Duration = Duration::from_secs(5);

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
    name: "edit active theme file in place",
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
    doorbells: Some(1),
};
const EDIT_SYMLINKED_CONFIG_TARGET: Mechanism = Mechanism {
    name: "edit the target of a symlinked config.toml",
    setup: Some(symlink_config_into_dotfiles),
    apply: edit_symlinked_config_target,
    doorbells: None,
};
const RETARGET_SYMLINKED_CONFIG: Mechanism = Mechanism {
    name: "retarget a symlinked config.toml",
    setup: Some(symlink_config_into_dotfiles),
    apply: retarget_symlinked_config,
    doorbells: None,
};

const EDIT_ACTIVE_PALETTE: Mechanism = Mechanism {
    name: "edit the active palette file in place",
    setup: None,
    apply: edit_active_palette_in_place,
    doorbells: Some(1),
};
const REAPPLY_SAME_PALETTE: Mechanism = Mechanism {
    name: "rewrite config.toml with the same palette name after a palette edit",
    setup: None,
    apply: reapply_same_palette,
    doorbells: None,
};

const INVALID_THEN_FIXED_CONFIG: Mechanism = Mechanism {
    name: "fix an invalid lower-priority config.toml the agent started without",
    setup: Some(invalid_home_config_only),
    apply: fix_home_config,
    doorbells: None,
};
const DELETE_CONFIG: Mechanism = Mechanism {
    name: "delete the active config.toml",
    setup: None,
    apply: delete_active_config,
    doorbells: None,
};
const HIGHER_PRIORITY_CONFIG_APPEARS: Mechanism = Mechanism {
    name: "create a higher-priority config.toml while running on a lower one",
    setup: Some(config_lives_under_home),
    apply: create_xdg_config,
    doorbells: None,
};
const DELETE_CONFIG_WITH_LOCAL_GPY_TOML: Mechanism = Mechanism {
    name: "delete the active config.toml with a .gpy.toml in the agent's cwd",
    setup: Some(local_gpy_toml_in_cwd),
    apply: delete_active_config,
    doorbells: None,
};

// Supervisor settings render nothing; shells learn them only from the export,
// so changing just one must still ring the reload doorbell exactly once.
const DISABLE_SUPERVISOR: Mechanism = Mechanism {
    name: "disable the supervisor in config.toml",
    setup: None,
    apply: disable_supervisor,
    doorbells: Some(1),
};
const CHANGE_SUPERVISOR_INTERVAL: Mechanism = Mechanism {
    name: "change the supervisor check interval in config.toml",
    setup: None,
    apply: change_supervisor_interval,
    doorbells: Some(1),
};
const CHANGE_SUPERVISOR_ATTEMPTS: Mechanism = Mechanism {
    name: "change the supervisor restart-attempt limit in config.toml",
    setup: None,
    apply: change_supervisor_attempts,
    doorbells: Some(1),
};

fn disable_supervisor(h: &Harness) {
    h.write_config_with("\n[agent.supervisor]\nenabled = false\n");
}

fn change_supervisor_interval(h: &Harness) {
    h.write_config_with("\n[agent.supervisor]\ncheck_interval_seconds = 45\n");
}

fn change_supervisor_attempts(h: &Harness) {
    h.write_config_with("\n[agent.supervisor]\nmax_restart_attempts = 2\n");
}

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
/// source, so it must never become active.
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
    _env: EnvGuard,
    prev_cwd: PathBuf,
    doorbells: SigurgCounter,
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
        let mut env = EnvGuard::cleared(&["GPY_CONFIG_PATH", "GPY_DISABLE_WATCHER"]);
        env.set("HOME", &root);
        env.set("XDG_CONFIG_HOME", &config_home);
        env.set("XDG_CACHE_HOME", &cache_home);
        env.set("XDG_RUNTIME_DIR", &runtime);
        let doorbells = SigurgCounter::install();
        let harness = Self {
            _temp: temp,
            home: root,
            config_home,
            cache_home,
            _env: env,
            prev_cwd: std::env::current_dir().expect("current dir"),
            doorbells,
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
        fs::write(path, Self::config_toml(theme, git_enabled)).expect("write config");
    }

    /// The active config (theme `active`, git on) followed by `extra` TOML.
    fn write_config_with(&self, extra: &str) {
        fs::write(
            self.config_path(),
            format!("{}{extra}", Self::config_toml(ACTIVE_THEME, true)),
        )
        .expect("write config");
    }

    fn config_toml(theme: &str, git_enabled: bool) -> String {
        format!(
            "[ui]\ntheme = \"{theme}\"\npalette = \"{ACTIVE_PALETTE}\"\n\n[git]\nenabled = {git_enabled}\n"
        )
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
                load_config_from_file(&path).expect("fresh config load")
            })
    }

    /// What the agent starts with: like [`Self::fresh_config`], except that a
    /// file which fails to load means built-in defaults (the agent's startup
    /// fallback), so a row may begin on an invalid file.
    fn startup_config(&self) -> Config {
        self.active_config_file()
            .and_then(|path| load_config_from_file(&path).ok())
            .unwrap_or_default()
    }

    /// What a load of `config` exports, computed with the public loaders.
    fn fresh_exports(config: &Config) -> Vec<String> {
        let theme_manager = ThemeManager::new(config.ui.theme.as_str()).expect("fresh theme load");
        let scratch = TempDir::new().expect("scratch dir");
        write_theme_export_to_dir(
            scratch.path(),
            &theme_manager,
            config,
            &active_palette(config),
        )
        .expect("fresh export");
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
    }
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
    // `Agent::start_background` warms the export at startup; the harness only
    // runs `Agent::new`, so warm it here, or the first reload would create it
    // and ring for that alone.
    for (file, content) in SHELL_EXPORTS.iter().zip(&before.0) {
        fs::write(harness.cache_home.join("gpy").join(file), content).expect("warm export");
    }

    let agent = Agent::new().expect("agent starts");
    agent.register_test_client(std::process::id(), harness.config_home.as_path());
    // Let the watchers arm before touching the disk.
    std::thread::sleep(Duration::from_millis(500));
    let rang_before = harness.doorbells.count();

    (mechanism.apply)(&harness);
    let applied_at = Instant::now();
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
    let settled = wait_until(SETTLE_DEADLINE, Duration::from_millis(50), || {
        agent_state() == expected
    });
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
        std::thread::sleep(DOORBELL_WINDOW.saturating_sub(applied_at.elapsed()));
        assert_eq!(
            harness.doorbells.count().saturating_sub(rang_before),
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

/// With the poll fallback in use, a theme switch by name must not wait
/// out the poll interval (the CLI gives the agent 2 s to acknowledge a reload).
/// Drives `ThemeManager` directly: `switch_theme` is what the `config_reload`
/// request runs synchronously, and it re-arms the watcher via `HotReloadSlot`.
#[test]
#[serial]
fn hot_reload_switch_theme_by_name_with_poll_fallback_is_prompt() {
    const POLL_VAR: &str = "GPY_THEME_WATCH_POLL_MS";
    let harness = Harness::new();
    let poll_env = EnvGuard::set_one(POLL_VAR, "250");

    let manager = ThemeManager::new(ACTIVE_THEME).expect("theme manager");
    manager
        .start_watching(Duration::from_secs(5))
        .expect("start watching");
    // Let the poll thread enter its between-ticks sleep.
    std::thread::sleep(Duration::from_millis(500));

    let started = Instant::now();
    let switched = manager.switch_theme(OTHER_THEME);
    let elapsed = started.elapsed();

    drop(poll_env);
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

/// The agent starts on an unparsable lower-priority config (falling
/// back to defaults), and fixing that file must apply it.
#[test]
#[serial]
fn hot_reload_invalid_config_then_fixed() {
    run_mechanism(&INVALID_THEN_FIXED_CONFIG);
}

/// Deleting the active config.toml reverts the agent to defaults.
#[test]
#[serial]
fn hot_reload_delete_config() {
    run_mechanism(&DELETE_CONFIG);
}

/// A higher-priority candidate appearing switches the agent to it.
#[test]
#[serial]
fn hot_reload_higher_priority_config_appears() {
    run_mechanism(&HIGHER_PRIORITY_CONFIG_APPEARS);
}

/// With the active config.toml gone, the agent falls back to defaults,
/// never to a `.gpy.toml` in the directory it was launched from.
#[test]
#[serial]
fn hot_reload_delete_config_ignores_local_gpy_toml() {
    run_mechanism(&DELETE_CONFIG_WITH_LOCAL_GPY_TOML);
}

#[test]
#[serial]
fn hot_reload_disable_supervisor() {
    run_mechanism(&DISABLE_SUPERVISOR);
}

#[test]
#[serial]
fn hot_reload_change_supervisor_interval() {
    run_mechanism(&CHANGE_SUPERVISOR_INTERVAL);
}

#[test]
#[serial]
fn hot_reload_change_supervisor_attempts() {
    run_mechanism(&CHANGE_SUPERVISOR_ATTEMPTS);
}
