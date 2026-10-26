//! Reload regressions against a real `gpy-agent start` daemon on an isolated
//! HOME/XDG tree: files change on disk, a `config_reload` request applies them,
//! and rendered `directory` responses show what the daemon now uses.

#![cfg(unix)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]

mod common;

use common::child_guard::AgentStopGuard;
use common::cli_harness::CliTestEnv;
use common::sigurg::SigurgCounter;
use common::wait::wait_until;
use gpy_agent::ipc::protocol::CONFIG_RELOAD_JSON;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// A running daemon on its own config tree, stopped on drop.
struct Daemon {
    // First: fields drop in order, and `stop` needs the tree `env` removes.
    _stop: AgentStopGuard,
    env: CliTestEnv,
    socket: PathBuf,
    cwd: PathBuf,
}

impl Daemon {
    /// Start a daemon on `env`, whose files must already be in place.
    fn start(env: CliTestEnv) -> Self {
        let socket = env.xdg_runtime_dir().join("gpy").join("gpy.sock");
        // Armed before `start` so the daemon stops even if a later step fails.
        let stop = AgentStopGuard::new(env.gpy_agent_command(), socket.clone());
        env.run_gpy_agent(&["start"])
            .expect("run gpy-agent start")
            .assert_success("gpy-agent start");
        assert!(
            wait_until(Duration::from_secs(10), Duration::from_millis(25), || {
                UnixStream::connect(&socket).is_ok()
            }),
            "agent never accepted a connection on {}",
            socket.display()
        );
        let cwd = env.root().join("fixture");
        fs::create_dir_all(&cwd).expect("create fixture dir");
        Self {
            _stop: stop,
            env,
            socket,
            cwd,
        }
    }

    fn request(&self, json: &str) -> String {
        let mut stream = UnixStream::connect(&self.socket).expect("connect to agent");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("set read timeout");
        stream
            .write_all(format!("{json}\n").as_bytes())
            .expect("send request");
        let mut response = String::new();
        BufReader::new(stream)
            .read_line(&mut response)
            .expect("read response");
        response
    }

    /// The ANSI-rendered directory segment for the fixture directory, as the
    /// last segment (the oneshot default).
    fn directory(&self) -> String {
        let request = serde_json::json!({
            "op": "directory",
            "cwd": self.cwd,
            "format": "ansi",
            "is_last": true,
        });
        self.request(&request.to_string())
    }

    /// What a fresh `gpy-agent oneshot` renders for the same directory request.
    fn oneshot_directory(&self) -> String {
        let cwd = self.cwd.to_str().expect("utf-8 fixture path");
        let result = self
            .env
            .run_gpy_agent(&["oneshot", "directory", "--cwd", cwd, "--format", "ansi"])
            .expect("run oneshot");
        result.assert_success("gpy-agent oneshot directory");
        result.stdout
    }

    /// The daemon's directory render, asserted equal to a fresh oneshot's.
    fn directory_matching_oneshot(&self, context: &str) -> String {
        let daemon = self.directory();
        assert_eq!(
            daemon.trim_end(),
            self.oneshot_directory().trim_end(),
            "{context}: the daemon disagrees with a fresh oneshot"
        );
        daemon
    }

    /// Send `config_reload` and return whether the daemon acknowledged it.
    fn reload(&self) -> bool {
        self.request(CONFIG_RELOAD_JSON)
            .contains(r#""status":"ok""#)
    }

    fn write(&self, relative: &str, content: &str) {
        let path = self.env.config_dir().join(relative);
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        fs::write(path, content).expect("write file");
    }

    fn remove(&self, relative: &str) {
        fs::remove_file(self.env.config_dir().join(relative)).expect("remove file");
    }
}

/// Register this test process with the daemon as a shell and count each
/// `SIGURG` doorbell it rings.
fn register_shell(daemon: &Daemon) -> SigurgCounter {
    let doorbells = SigurgCounter::install();
    let request = serde_json::json!({
        "op": "register",
        "pid": std::process::id(),
        "cwd": daemon.cwd,
    });
    let response = daemon.request(&request.to_string());
    assert!(
        !response.contains("error"),
        "registration failed: {response}"
    );
    doorbells
}

/// Theme `audit` colors the directory with the `signal` palette role.
fn palette_env(signal: &str) -> CliTestEnv {
    let env = CliTestEnv::with_config("[ui]\ntheme = \"audit\"\npalette = \"audit\"\n")
        .expect("create env");
    fs::write(
        env.themes_dir().join("audit.toml"),
        "[ui]\n[segments.directory]\nformat = \"[$path](fg:signal)\"\n",
    )
    .expect("write theme");
    let palettes = env.config_dir().join("palettes");
    fs::create_dir_all(&palettes).expect("create palettes dir");
    fs::write(palettes.join("audit.toml"), palette_toml(signal)).expect("write palette");
    env
}

fn palette_toml(signal: &str) -> String {
    format!("[colors]\nsignal = \"{signal}\"\n")
}

/// SGR foreground for `#123456` and `#654321`.
const FIRST_FG: &str = "38;2;18;52;86m";
const SECOND_FG: &str = "38;2;101;67;33m";

#[test]
fn unusable_palette_edit_keeps_last_good_palette_until_fixed() {
    let daemon = Daemon::start(palette_env("#123456"));
    let first = daemon.directory();
    assert!(first.contains(FIRST_FG), "initial render: {first:?}");

    for (case, content) in [
        (
            "reference cycle",
            "[colors]\nsignal = \"loop\"\nloop = \"signal\"\n",
        ),
        ("malformed TOML", "[colors\nsignal = "),
    ] {
        daemon.write("palettes/audit.toml", content);
        assert!(daemon.reload(), "{case}: reload of unchanged config acks");
        assert_eq!(
            daemon.directory(),
            first,
            "{case}: the last good palette must keep rendering"
        );
    }

    daemon.write("palettes/audit.toml", &palette_toml("#654321"));
    assert!(daemon.reload(), "valid palette reload acks");
    let fixed = daemon.directory();
    assert!(
        fixed.contains(SECOND_FG),
        "a valid save must apply without restart: {fixed:?}"
    );
}

#[test]
fn switching_to_unusable_palette_rejects_the_reload() {
    let daemon = Daemon::start(palette_env("#123456"));
    let first = daemon.directory();
    assert!(first.contains(FIRST_FG), "initial render: {first:?}");

    daemon.write(
        "palettes/broken.toml",
        "[colors]\nsignal = \"loop\"\nloop = \"signal\"\n",
    );
    daemon.write(
        "config.toml",
        "[ui]\ntheme = \"audit\"\npalette = \"broken\"\n",
    );
    assert!(
        !daemon.reload(),
        "selecting a palette that cannot load must reject the reload"
    );
    assert_eq!(
        daemon.directory(),
        first,
        "the rejected reload changed rendering"
    );

    daemon.write("palettes/broken.toml", &palette_toml("#654321"));
    assert!(daemon.reload(), "the fixed palette reload acks");
    let fixed = daemon.directory();
    assert!(
        fixed.contains(SECOND_FG),
        "fixed palette applies: {fixed:?}"
    );
}

const USER_OVERRIDE: &str = "[ui]\n[segments.directory]\nformat = \"USER_THEME:$path\"\n";

/// Add then remove a user override for the active theme `name`, applying each
/// with `config_reload`: the daemon must follow both and agree with a fresh
/// oneshot each time, without a restart.
fn assert_override_followed(daemon: &Daemon, name: &str) {
    let override_file = format!("themes/{name}.toml");
    let before = daemon.directory_matching_oneshot("before the override");
    assert!(!before.contains("USER_THEME:"), "{name}: {before:?}");

    daemon.write(&override_file, USER_OVERRIDE);
    assert!(
        daemon.reload(),
        "{name}: reload after adding the override acks"
    );
    let overridden = daemon.directory_matching_oneshot("with the override");
    assert!(
        overridden.contains("USER_THEME:"),
        "{name}: the user override must win: {overridden:?}"
    );

    daemon.remove(&override_file);
    assert!(
        daemon.reload(),
        "{name}: reload after removing the override acks"
    );
    assert_eq!(
        daemon.directory_matching_oneshot("after removing the override"),
        before,
        "{name}: removing the override must restore the lower-priority theme"
    );
}

#[test]
fn config_reload_follows_a_user_override_above_the_builtin_theme() {
    let env = CliTestEnv::with_config("[ui]\ntheme = \"default\"\n").expect("create env");
    // The harness seeds a user copy of `default`; this test needs the builtin.
    fs::remove_file(env.themes_dir().join("default.toml")).expect("remove seeded theme");
    let daemon = Daemon::start(env);
    assert_override_followed(&daemon, "default");
}

#[test]
fn config_reload_follows_a_user_override_above_a_plugin_theme() {
    let env = CliTestEnv::with_config("[ui]\ntheme = \"community\"\n").expect("create env");
    let plugin = env.plugins_dir().join("acme");
    fs::create_dir_all(plugin.join("themes")).expect("create plugin dir");
    fs::write(
        plugin.join("plugin.toml"),
        "id = \"acme\"\nname = \"Acme\"\nversion = \"0.1.0\"\napi_version = \"v1\"\n\
         entry_type = \"file\"\nprovided_segments = [\"acme_seg\"]\n",
    )
    .expect("write plugin manifest");
    fs::write(
        plugin.join("themes").join("community.toml"),
        "[ui]\n[segments.directory]\nformat = \"PLUGIN_THEME:$path\"\n",
    )
    .expect("write plugin theme");
    let daemon = Daemon::start(env);
    assert!(daemon.directory().contains("PLUGIN_THEME:"));
    assert_override_followed(&daemon, "community");
}

/// An explicit `config_reload` applies an in-place edit of the active theme
/// file at once, without waiting for the theme watcher's debounce.
#[test]
fn config_reload_applies_an_in_place_theme_edit() {
    let daemon = Daemon::start(palette_env("#123456"));
    daemon.directory_matching_oneshot("before the edit");

    daemon.write(
        "themes/audit.toml",
        "[ui]\n[segments.directory]\nformat = \"EDITED:$path\"\n",
    );
    assert!(daemon.reload(), "reload after the edit acks");
    let edited = daemon.directory_matching_oneshot("after the edit");
    assert!(
        edited.contains("EDITED:"),
        "the edit must apply: {edited:?}"
    );
}

/// Bound on a watched in-place theme save reaching a render: the 1 s debounce,
/// up to 1 s more for the flush deadline, plus slack for a loaded machine. Still
/// below the 5 s theme debounce this replaced.
const THEME_EDIT_LATENCY: Duration = Duration::from_secs(4);
/// Long enough after an edit for every watcher to have flushed it.
const DOORBELL_WINDOW: Duration = Duration::from_secs(6);

/// With no explicit reload, an in-place save of the active theme reaches a
/// render promptly and rings shells exactly once.
#[test]
fn watched_theme_edit_renders_promptly_and_rings_once() {
    let env = palette_env("#123456");
    // No clock segment: its minute repaint also rings `SIGURG`.
    fs::write(
        env.config_dir().join("config.toml"),
        "[ui]\ntheme = \"audit\"\npalette = \"audit\"\nenabled_segments = [\"directory\"]\n",
    )
    .expect("write config");
    let daemon = Daemon::start(env);
    let shell = register_shell(&daemon);
    daemon.directory_matching_oneshot("before the edit");
    // Let registration settle and the watchers arm before editing.
    std::thread::sleep(Duration::from_millis(500));
    let rang_before = shell.count();

    daemon.write(
        "themes/audit.toml",
        "[ui]\n[segments.directory]\nformat = \"EDITED:$path\"\n",
    );
    let edited_at = Instant::now();
    let rendered = wait_until(Duration::from_secs(15), Duration::from_millis(25), || {
        daemon.directory().contains("EDITED:")
    });
    let latency = edited_at.elapsed();
    assert!(rendered, "the theme edit never rendered");
    assert!(
        latency <= THEME_EDIT_LATENCY,
        "the theme edit took {latency:?} to render"
    );

    std::thread::sleep(DOORBELL_WINDOW.saturating_sub(edited_at.elapsed()));
    assert_eq!(
        shell.count().saturating_sub(rang_before),
        1,
        "reload doorbells for one theme edit"
    );
}
