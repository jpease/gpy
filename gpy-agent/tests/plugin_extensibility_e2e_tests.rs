//! End-to-end plugin tests: discovery, theme export, and Fish runtime loading.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test helpers outside #[test] fns panic on failure"
)]

use gpy_agent::config::Config;
use gpy_agent::config::discovery::Source;
use gpy_agent::shell::Shell;
use gpy_agent::theme::ThemeManager;
use serial_test::serial;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

#[path = "common/env_guard.rs"]
mod env_guard;
#[path = "common/skip.rs"]
mod skip;

use env_guard::EnvGuard;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf()
}

fn write_config(root: &std::path::Path, theme: &str, segments: &[&str]) {
    let cfg_dir = root.join("gpy");
    fs::create_dir_all(&cfg_dir).expect("create config dir");
    let list = segments
        .iter()
        .map(|segment| format!("\"{segment}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let content = format!(
        r#"[agent]
enabled = true

[git]

[language]

[ui]
theme = "{theme}"
enabled_segments = [{list}]
"#
    );
    fs::write(cfg_dir.join("config.toml"), content).expect("write config");
}

fn install_plugin(root: &std::path::Path, with_segment_file: bool) {
    let plugin_dir = root.join("gpy").join("plugins").join("k8s-tools");
    fs::create_dir_all(plugin_dir.join("segments")).expect("create plugin dirs");
    fs::create_dir_all(plugin_dir.join("themes")).expect("create plugin themes dir");

    fs::write(
        plugin_dir.join("plugin.toml"),
        r#"id = "k8s-tools"
name = "K8s Tools"
version = "0.1.0"
api_version = "v1"
entry_type = "file"
provided_segments = ["k8s_tools"]
"#,
    )
    .expect("write plugin manifest");

    if with_segment_file {
        fs::write(
            plugin_dir.join("segments").join("k8s_tools.fish"),
            r#"function segment_k8s_tools_detect
    return 0
end

function segment_k8s_tools_render --argument-names is_last
    echo -n "PLUGIN_SEGMENT"
end
"#,
        )
        .expect("write plugin segment");
    }

    fs::write(
        plugin_dir.join("themes").join("community.toml"),
        r##"[ui]

[segments.character]
success_symbol = ">"

[segments.clock]
bg_color = "black"
text_color = "white"

[segments.k8s_tools]
bg_color = "#073642"
text_color = "white"
icon = "⎈"
"##,
    )
    .expect("write plugin theme");
}

/// `true` only for Fish 4.x+ on `PATH`; the inline test script uses Fish-4-only syntax.
fn has_fish_4() -> bool {
    let Ok(output) = Command::new("fish").arg("--version").output() else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    // Output looks like: "fish, version 4.8.0" — the version is the last token.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some(version) = stdout.split_whitespace().last() else {
        return false;
    };
    let Some(major) = version.split('.').next() else {
        return false;
    };
    major
        .parse::<u32>()
        .is_ok_and(|major_version| major_version >= 4)
}

fn prepare_gpy_agent_command_path() -> (TempDir, PathBuf) {
    let gpy_agent_bin = PathBuf::from(env!("CARGO_BIN_EXE_gpy-agent"));
    let bin_dir_temp = TempDir::new().expect("bin tempdir");
    let gpy_agent_cmd_path = bin_dir_temp.path().join("gpy-agent");
    fs::copy(&gpy_agent_bin, &gpy_agent_cmd_path).expect("copy gpy-agent binary");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&gpy_agent_cmd_path)
            .expect("metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&gpy_agent_cmd_path, permissions).expect("set executable bit");
    }
    (bin_dir_temp, gpy_agent_cmd_path)
}

fn build_path_with_bin(bin_dir: &std::path::Path) -> String {
    format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    )
}

fn verify_theme_export_contains_plugin_file_var(
    gpy_agent_cmd_path: &std::path::Path,
    repo: &std::path::Path,
    path: &str,
    xdg: &std::path::Path,
) {
    let export_output = Command::new(gpy_agent_cmd_path)
        .args(["theme", "export", "--shell", "fish"])
        .current_dir(repo)
        .env("PATH", path)
        .env("XDG_CONFIG_HOME", xdg)
        .output()
        .expect("run gpy-agent theme export");
    assert!(
        export_output.status.success(),
        "theme export failed.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&export_output.stdout),
        String::from_utf8_lossy(&export_output.stderr)
    );
    let export_stdout = String::from_utf8_lossy(&export_output.stdout);
    assert!(
        export_stdout.contains("__gpy_plugin_segment_file_k8s_tools"),
        "theme export missing plugin file variable.\nstdout:\n{export_stdout}"
    );
}

fn verify_plugin_segment_loads_in_fish(repo: &std::path::Path, path: &str, xdg: &std::path::Path) {
    let fish_script = "source fish/core/init.fish; gpy-agent theme export --shell fish 2>/dev/null | source; if set -q __gpy_plugin_segment_file_k8s_tools; echo FILE_VAR_OK; else echo FILE_VAR_MISSING; end; __gpy_load_segments k8s_tools; if functions -q segment_k8s_tools_render; echo SEGMENT_OK; segment_k8s_tools_render 1; echo; else echo SEGMENT_MISSING; end";

    let output = Command::new("fish")
        .args(["-c", fish_script])
        .current_dir(repo)
        .env("PATH", path)
        .env("XDG_CONFIG_HOME", xdg)
        .output()
        .expect("run fish");

    assert!(
        output.status.success(),
        "fish init failed.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("FILE_VAR_OK"),
        "plugin file var missing.\nstdout:\n{}\nstderr:\n{}",
        stdout,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("SEGMENT_OK"),
        "segment function missing.\nstdout:\n{}\nstderr:\n{}",
        stdout,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("PLUGIN_SEGMENT"));
}

fn verify_missing_plugin_file_is_graceful(
    repo: &std::path::Path,
    path: &str,
    broken_xdg: &std::path::Path,
) {
    let output_broken = Command::new("fish")
        .args(["-c", "source fish/core/init.fish; echo INIT_OK"])
        .current_dir(repo)
        .env("PATH", path)
        .env("XDG_CONFIG_HOME", broken_xdg)
        .output()
        .expect("run fish with broken plugin");

    assert!(output_broken.status.success());
    let stdout_broken = String::from_utf8_lossy(&output_broken.stdout);
    assert!(stdout_broken.contains("INIT_OK"));
}

#[test]
#[serial]
fn e2e_plugin_segment_and_theme_discovery_and_export() {
    let xdg = TempDir::new().expect("xdg tempdir");
    install_plugin(xdg.path(), true);
    write_config(xdg.path(), "community", &["clock", "k8s_tools"]);

    let mut env = EnvGuard::set_one("XDG_CONFIG_HOME", xdg.path());
    env.set("XDG_DATA_HOME", xdg.path());
    env.remove("GPY_BUNDLED_PLUGIN_DIR");

    let themes = ThemeManager::discover_available_themes();
    let community = themes
        .iter()
        .find(|theme| theme.name == "community")
        .expect("plugin theme should be discovered");
    assert!(matches!(community.source, Source::Plugin { .. }));
    let manager = ThemeManager::new("community").expect("load plugin theme");
    let mut config = Config::default();
    config.ui.enabled_segments = vec!["clock".to_owned(), "k8s_tools".to_owned()];
    let fish_export = manager.export(
        Shell::Fish,
        &config,
        &gpy_agent::template::Palette::default(),
    );

    assert!(fish_export.contains("set -g __enabled_segments clock k8s_tools"));
    assert!(fish_export.contains("set -g __gpy_segment_k8s_tools_bg_color \"#073642\""));
    assert!(fish_export.contains("set -g __gpy_plugin_segment_file_k8s_tools"));
}

#[test]
#[serial]
fn e2e_plugin_runtime_loading_and_missing_file_graceful() {
    // Fish-4-only script; skip on older fish (e.g. Ubuntu CI's 3.7).
    if !has_fish_4() {
        skip::skip_test("fish 4 is not installed; the inline script uses Fish-4-only syntax");
        return;
    }

    let xdg = TempDir::new().expect("xdg tempdir");
    let repo = repo_root();

    // Valid plugin should load and register render function in fish runtime.
    install_plugin(xdg.path(), true);
    write_config(xdg.path(), "default", &["clock", "k8s_tools"]);

    let (bin_dir_temp, gpy_agent_cmd_path) = prepare_gpy_agent_command_path();
    let path = build_path_with_bin(bin_dir_temp.path());
    verify_theme_export_contains_plugin_file_var(&gpy_agent_cmd_path, &repo, &path, xdg.path());
    verify_plugin_segment_loads_in_fish(&repo, &path, xdg.path());

    // Missing file should not crash init; fish should still exit successfully.
    let broken_xdg = TempDir::new().expect("broken xdg tempdir");
    install_plugin(broken_xdg.path(), false);
    write_config(broken_xdg.path(), "default", &["clock", "k8s_tools"]);
    verify_missing_plugin_file_is_graceful(&repo, &path, broken_xdg.path());
}

/// Isolated `HOME`/XDG roots plus a `PATH` whose `gpy-agent` is this build.
struct FishSandbox {
    root: TempDir,
    _bin_dir: TempDir,
    path: String,
}

impl FishSandbox {
    fn new() -> Self {
        let (bin_dir, _) = prepare_gpy_agent_command_path();
        let path = build_path_with_bin(bin_dir.path());
        Self {
            root: TempDir::new().expect("sandbox tempdir"),
            _bin_dir: bin_dir,
            path,
        }
    }

    fn xdg_config(&self) -> PathBuf {
        self.root.path().join("config")
    }

    fn apply_env<'a>(&self, command: &'a mut Command) -> &'a mut Command {
        let root = self.root.path();
        command
            .env("HOME", root)
            .env("XDG_CONFIG_HOME", self.xdg_config())
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("XDG_RUNTIME_DIR", root.join("runtime"))
            .env("PATH", &self.path)
    }

    /// Install a v1 `.fish` plugin `id` providing `segment`, shaped like the
    /// scaffold `gpy plugin new` used to write.
    fn install_fish_plugin(&self, id: &str, segment: &str) {
        let root = self.xdg_config().join("gpy").join("plugins").join(id);
        fs::create_dir_all(root.join("segments")).expect("create plugin dirs");
        fs::write(
            root.join("plugin.toml"),
            format!(
                "id = \"{id}\"\nname = \"{id}\"\nversion = \"0.1.0\"\napi_version = \"v1\"\n\
                 entry_type = \"file\"\nprovided_segments = [\"{segment}\"]\n"
            ),
        )
        .expect("write manifest");
        fs::write(
            root.join("segments").join(format!("{segment}.fish")),
            format!(
                "function segment_{segment}_detect\n    return 0\nend\n\n\
                 function segment_{segment}_render --argument-names is_last\n    \
                 gpy_section_standalone blue white \"* {segment}\" $is_last\nend\n"
            ),
        )
        .expect("write segment");
    }

    /// Run the real `gpy` CLI in the sandbox.
    fn gpy(&self, args: &[&str]) -> std::process::Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_gpy"));
        self.apply_env(&mut command)
            .args(args)
            .output()
            .expect("run gpy")
    }

    /// Run `script` in Fish from the repo root after sourcing the real `init.fish`.
    fn fish(&self, script: &str, segment: &str) -> std::process::Output {
        let mut command = Command::new("fish");
        self.apply_env(&mut command)
            .args([
                "--no-config",
                "-c",
                &format!("source fish/core/init.fish; {script}"),
            ])
            .env("SEGMENT", segment)
            .current_dir(repo_root())
            .output()
            .expect("run fish")
    }
}

fn assert_success(output: &std::process::Output, context: &str) {
    assert!(
        output.status.success(),
        "{context} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// After init (and, for `reload`, after `__gpy_reload_theme` picks up the
/// segment), the scaffolded render function exists and draws the segment.
const FISH_RENDER_PROBE: &str = "if functions -q segment_{$SEGMENT}_render; \
     echo LOADED; segment_{$SEGMENT}_render last; echo; else; echo MISSING; end";

#[test]
#[serial]
fn e2e_scaffolded_segments_with_repeated_or_edge_separators_load_in_fish() {
    if !has_fish_4() {
        skip::skip_test("fish 4 is not installed; init.fish sourcing needs Fish 4");
        return;
    }
    for segment in ["my-seg", "my--seg", "my__seg", "seg_"] {
        let sandbox = FishSandbox::new();
        sandbox.install_fish_plugin("audit-plugin", segment);

        write_config(&sandbox.xdg_config(), "default", &[segment]);
        let initial = sandbox.fish(FISH_RENDER_PROBE, segment);
        let stdout = String::from_utf8_lossy(&initial.stdout);
        assert!(
            stdout.contains("LOADED") && stdout.contains(&format!("* {segment}")),
            "'{segment}' not loaded at init.\nstdout:\n{stdout}\nstderr:\n{}",
            String::from_utf8_lossy(&initial.stderr)
        );

        write_config(&sandbox.xdg_config(), "default", &["clock"]);
        let reload_script = format!(
            "printf '[ui]\\ntheme = \"default\"\\nenabled_segments = [\"%s\"]\\n' $SEGMENT \
             > $XDG_CONFIG_HOME/gpy/config.toml; __gpy_reload_theme; {FISH_RENDER_PROBE}"
        );
        let reloaded = sandbox.fish(&reload_script, segment);
        let reloaded_stdout = String::from_utf8_lossy(&reloaded.stdout);
        assert!(
            reloaded_stdout.contains("LOADED") && reloaded_stdout.contains(&format!("* {segment}")),
            "'{segment}' not loaded on reload.\nstdout:\n{reloaded_stdout}\nstderr:\n{}",
            String::from_utf8_lossy(&reloaded.stderr)
        );
    }
}

#[test]
#[serial]
fn e2e_colliding_segment_tokens_are_rejected_not_redirected() {
    let sandbox = FishSandbox::new();
    sandbox.install_fish_plugin("ctx-a", "k8s-ctx");
    sandbox.install_fish_plugin("ctx-b", "k8s_ctx");

    let list = sandbox.gpy(&["plugin", "list"]);
    assert_success(&list, "gpy plugin list");
    let listing = String::from_utf8_lossy(&list.stdout);
    assert!(
        listing.contains("- ctx-a") && !listing.contains("- ctx-b"),
        "the later colliding plugin must not be registered:\n{listing}"
    );
    assert!(
        listing.contains("k8s-ctx") && listing.contains("k8s_ctx"),
        "a diagnostic must name both colliding segments:\n{listing}"
    );

    write_config(&sandbox.xdg_config(), "default", &["k8s-ctx", "k8s_ctx"]);
    let get = sandbox.gpy(&["config", "get", "ui.enabled_segments"]);
    assert!(
        !get.status.success(),
        "config enabling both colliding names must not load:\nstdout:\n{}",
        String::from_utf8_lossy(&get.stdout)
    );
    let stderr = String::from_utf8_lossy(&get.stderr);
    assert!(
        stderr.contains("k8s-ctx") && stderr.contains("k8s_ctx"),
        "error must name both segments:\n{stderr}"
    );
}
