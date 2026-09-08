#![allow(clippy::expect_used, clippy::unwrap_used, clippy::missing_panics_doc)]

use gpy_agent::config::Config;
use gpy_agent::shell::Shell;
use gpy_agent::theme::{ThemeManager, ThemeSource};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

#[path = "common/skip.rs"]
mod skip;

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
enabled = true

[language]
enabled = true

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
prompt_icon = ">"

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

/// Returns `true` only when a Fish 4.x+ interpreter is on `PATH`.
///
/// The runtime itself supports Fish 3.6+, but the inline script in
/// [`verify_plugin_segment_loads_in_fish`] relies on Fish-4-only syntax that
/// Fish 3.7 (shipped by Ubuntu CI's apt) rejects with a parse error. Gating on
/// the major version lets this e2e test skip cleanly on older interpreters
/// instead of failing on a version-specific parse difference (see issue #282).
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
        .args(["theme", "export", "--format", "fish"])
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
    let fish_script = "source fish/core/init.fish; gpy-agent theme export --format fish 2>/dev/null | source; if set -q __gpy_plugin_segment_file_k8s_tools; echo FILE_VAR_OK; else echo FILE_VAR_MISSING; end; __gpy_load_segments fish/core k8s_tools; if functions -q segment_k8s_tools_render; echo SEGMENT_OK; segment_k8s_tools_render 1; echo; else echo SEGMENT_MISSING; end";

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
fn e2e_plugin_segment_and_theme_discovery_and_export() {
    let xdg = TempDir::new().expect("xdg tempdir");
    install_plugin(xdg.path(), true);
    write_config(xdg.path(), "community", &["clock", "k8s_tools"]);

    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", xdg.path());
        std::env::remove_var("GPY_BUNDLED_PLUGIN_DIR");
    }

    let themes = ThemeManager::discover_available_themes();
    let community = themes
        .iter()
        .find(|theme| theme.name == "community")
        .expect("plugin theme should be discovered");
    assert!(matches!(community.source, ThemeSource::Plugin { .. }));

    let manager = ThemeManager::new("community").expect("load plugin theme");
    let mut config = Config::default();
    config.ui.enabled_segments = vec!["clock".to_owned(), "k8s_tools".to_owned()];
    let fish_export = manager.export(Shell::Fish, &config);

    assert!(fish_export.contains("set -g __enabled_segments clock k8s_tools"));
    assert!(fish_export.contains("set -g __gpy_segment_k8s_tools_bg_color \"#073642\""));
    assert!(fish_export.contains("set -g __gpy_plugin_segment_file_k8s_tools"));

    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}

#[test]
fn e2e_plugin_runtime_loading_and_missing_file_graceful() {
    // The inline fish script below uses Fish-4-only syntax; skip on older
    // interpreters (e.g. Ubuntu CI's Fish 3.7) rather than fail on a parse
    // difference. The gpy runtime still supports Fish 3.6+ (see issue #282).
    if !has_fish_4() {
        skip::skip_test(
            "fish 4 is not installed; the inline script uses Fish-4-only syntax (#282)",
        );
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
