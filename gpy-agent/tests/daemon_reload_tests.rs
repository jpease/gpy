//! Reload regressions against a real `gpy-agent start` daemon on an isolated
//! HOME/XDG tree: files change on disk, a `config_reload` request applies them,
//! and rendered `directory` responses show what the daemon now uses.

#![cfg(unix)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]

mod common;

use common::{AgentStopGuard, CliTestEnv, wait_until};
use gpy_agent::ipc::protocol::CONFIG_RELOAD_JSON;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

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

    /// The ANSI-rendered directory segment for the fixture directory.
    fn directory(&self) -> String {
        let request = serde_json::json!({
            "op": "directory",
            "cwd": self.cwd,
            "format": "ansi",
        });
        self.request(&request.to_string())
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
