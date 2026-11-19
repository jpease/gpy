#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::missing_panics_doc,
    reason = "tests: panics are the assertions"
)]

use super::*;
use crate::config::loader;
use crate::plugin::{PluginManifest, PluginSource};
use std::fs;
use tempfile::TempDir;

const PLUGIN: &str = r#"
id = "k8s-tools"
name = "K8s"
version = "1.0.0"
api_version = "v1"
provided_segments = ["k8s-ctx", "git"]
entry_type = "script"
"#;

const SEGMENT: &str = r#"
command = "bin/seg.sh"
require_command = "./bin/check.sh"
inputs = ["is_ssh", "pwd"]
env = ["KUBECONFIG", "AWS_PROFILE"]
ttl_secs = 30
timeout_ms = 300
require_files = ["Chart.yaml"]
require_folders = [".kube"]
require_extensions = ["tf", "tar.gz"]
require_env = ["KUBECONFIG"]
[settings.style]
type = "string"
default = "bold"
choices = ["bold", "dim $x"]
[settings.depth]
type = "int"
default = 2
"#;

/// A discovered script plugin `k8s-tools` providing `k8s-ctx` and a `git`
/// that shadows the built-in, under a temporary root writable by the user only.
struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    discovery: PluginDiscovery,
}

fn fixture() -> Fixture {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("k8s-tools");
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::create_dir_all(root.join("segments")).unwrap();
    fs::write(root.join("plugin.toml"), PLUGIN).unwrap();
    fs::write(root.join("bin/seg.sh"), "echo hi\n").unwrap();
    fs::write(root.join("bin/check.sh"), "exit 0\n").unwrap();
    fs::write(root.join("segments/k8s-ctx.toml"), SEGMENT).unwrap();
    fs::write(root.join("segments/git.toml"), "command = \"bin/seg.sh\"\n").unwrap();
    for (relative, mode) in [
        ("", 0o755),
        ("plugin.toml", 0o644),
        ("bin", 0o755),
        ("bin/seg.sh", 0o644),
        ("bin/check.sh", 0o644),
        ("segments", 0o755),
        ("segments/k8s-ctx.toml", 0o644),
        ("segments/git.toml", 0o644),
    ] {
        set_mode(&root.join(relative), mode);
    }
    let manifest = PluginManifest::parse(PLUGIN).unwrap();
    let script_segments = DiscoveredPlugin::load_script_segments(&manifest, &root).unwrap();
    Fixture {
        discovery: PluginDiscovery {
            plugins: vec![DiscoveredPlugin {
                manifest,
                root: Some(root.clone()),
                source: PluginSource::User,
                script_segments,
            }],
            diagnostics: Vec::new(),
        },
        root,
        _temp: temp,
    }
}

/// Set a Unix mode (umask-independent fixtures); a no-op elsewhere.
fn set_mode(path: &Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
}

fn config(plugin_table: &str, enabled: &[&str]) -> Config {
    let mut config = loader::parse_config_toml(plugin_table).unwrap();
    config.ui.enabled_segments = enabled.iter().map(|name| (*name).to_owned()).collect();
    config
}

fn canonical_text(path: &Path) -> String {
    path.canonicalize().unwrap().to_str().unwrap().to_owned()
}

#[test]
fn collect_resolves_an_enabled_script_segment_with_its_overrides() {
    let fixture = fixture();
    let config = config(
        "[plugin.k8s-ctx]\nttl_secs = 0\nstyle = \"dim $x\"\n",
        &["directory", "k8s-ctx", "git"],
    );
    let exports = collect_script_segments(&fixture.discovery, &config);
    let root = canonical_text(&fixture.root);
    let command = canonical_text(&fixture.root.join("bin").join("seg.sh"));
    let opts = vec![
        "GPY_OPT_depth=2".to_owned(),
        "GPY_OPT_style=dim $x".to_owned(),
    ];
    let expected = ScriptSegmentExport {
        token: "k8s_ctx".to_owned(),
        name: "k8s-ctx".to_owned(),
        key: static_cache_key(
            ["k8s-tools", "1.0.0", "k8s-ctx", command.as_str()]
                .into_iter()
                .chain(opts.iter().map(String::as_str)),
        ),
        command,
        require_command: canonical_text(&fixture.root.join("bin").join("check.sh")),
        root,
        inputs: vec!["is_ssh", "pwd"],
        env: vec!["KUBECONFIG".to_owned(), "AWS_PROFILE".to_owned()],
        opts,
        ttl_secs: 0,
        timeout_ms: 300,
        scope: "folder",
        require_files: vec!["Chart.yaml".to_owned()],
        require_folders: vec![".kube".to_owned()],
        require_extensions: vec!["tf".to_owned(), "tar.gz".to_owned()],
        require_env: vec!["KUBECONFIG".to_owned()],
    };
    assert_eq!(exports.segments, vec![expected]);
    // `/bin/sh` is only absolute under Unix path rules.
    #[cfg(unix)]
    assert!(Path::new(&exports.sh).is_absolute(), "{}", exports.sh);
}

/// Only enabled script segments are exported: a built-in name provided by a
/// plugin (`git`), an unknown name and a second spelling of a token are not.
#[test]
fn collect_exports_only_enabled_script_segments_once() {
    let fixture = fixture();
    let tokens = |enabled: &[&str]| -> Vec<String> {
        collect_script_segments(&fixture.discovery, &config("", enabled))
            .segments
            .into_iter()
            .map(|segment| segment.token)
            .collect()
    };
    assert_eq!(tokens(&[]), Vec::<String>::new());
    assert_eq!(
        tokens(&["git", "missing", "Bad Name"]),
        Vec::<String>::new()
    );
    assert_eq!(tokens(&["K8S-CTX", "k8s_ctx", "k8s-ctx"]), ["k8s_ctx"]);
}

/// An override the manifest rejects (a raised `timeout_ms`) skips the segment
/// rather than running it with values the user did not choose.
#[test]
fn collect_skips_a_segment_whose_overrides_are_invalid() {
    let fixture = fixture();
    let exports = collect_script_segments(
        &fixture.discovery,
        &config("[plugin.k8s-ctx]\ntimeout_ms = 400\n", &["k8s-ctx"]),
    );
    assert!(exports.segments.is_empty(), "{exports:?}");
}

/// Containment is enforced at export, not only at discovery: a script swapped
/// for a symlink out of the root after discovery is not exported.
#[cfg(unix)]
#[test]
fn collect_rechecks_containment_at_export() {
    let fixture = fixture();
    let config = config("", &["k8s-ctx"]);
    assert_eq!(
        collect_script_segments(&fixture.discovery, &config)
            .segments
            .len(),
        1
    );

    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("evil.sh"), "echo evil\n").unwrap();
    fs::remove_file(fixture.root.join("bin/check.sh")).unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("evil.sh"),
        fixture.root.join("bin/check.sh"),
    )
    .unwrap();
    assert_eq!(
        collect_script_segments(&fixture.discovery, &config)
            .segments
            .len(),
        0
    );
}

/// A script, or a directory up to the plugin root, that others can write is
/// refused at export.
#[cfg(unix)]
#[test]
fn collect_refuses_a_script_or_directory_others_can_write() {
    let fixture = fixture();
    let config = config("", &["k8s-ctx"]);
    let exported = || {
        collect_script_segments(&fixture.discovery, &config)
            .segments
            .len()
    };
    for (relative, mode, restore) in [
        ("bin/seg.sh", 0o664, 0o644),
        ("bin/check.sh", 0o646, 0o644),
        ("bin", 0o775, 0o755),
        ("", 0o777, 0o755),
    ] {
        set_mode(&fixture.root.join(relative), mode);
        assert_eq!(exported(), 0, "{relative:?} at {mode:o} must be refused");
        set_mode(&fixture.root.join(relative), restore);
        assert_eq!(exported(), 1, "{relative:?} at {restore:o}");
    }
}

#[test]
fn static_cache_key_prefixes_each_field_with_its_byte_length() {
    assert_eq!(static_cache_key(["ab", "", "c:d", "é"]), "2:ab0:3:c:d2:é");
    assert_ne!(
        static_cache_key(["a", "bc"]),
        static_cache_key(["ab", "c"]),
        "fields must not shift into their neighbours"
    );
}

#[cfg(unix)]
#[test]
fn launcher_target_requires_an_executable_file_only_the_user_can_change() {
    let dir = TempDir::new().unwrap();
    set_mode(dir.path(), 0o700);
    let shell = dir.path().join("fake-sh");
    fs::write(&shell, "#!/bin/sh\n").unwrap();
    set_mode(&shell, 0o755);
    let selector = dir.path().join("select-sh");
    std::os::unix::fs::symlink(&shell, &selector).unwrap();
    let target = canonical_text(&shell);

    assert_eq!(launcher_target(&selector).as_deref(), Some(target.as_str()));
    set_mode(&shell, 0o775);
    assert_eq!(launcher_target(&selector), None, "group-writable");
    set_mode(&shell, 0o644);
    assert_eq!(launcher_target(&selector), None, "not executable");
    fs::remove_file(&shell).unwrap();
    assert_eq!(launcher_target(&selector), None, "dangling");
    fs::create_dir_all(&shell).unwrap();
    assert_eq!(launcher_target(&selector), None, "a directory");
}

/// The launcher is skipped on macOS only; anywhere else, and when the
/// selector is unusable, scripts run with `/bin/sh`.
#[cfg(unix)]
#[test]
fn resolve_sh_skips_the_launcher_on_macos_only() {
    let dir = TempDir::new().unwrap();
    set_mode(dir.path(), 0o700);
    let shell = dir.path().join("fake-sh");
    fs::write(&shell, "#!/bin/sh\n").unwrap();
    set_mode(&shell, 0o755);
    let selector = dir.path().join("select-sh");
    std::os::unix::fs::symlink(&shell, &selector).unwrap();

    let expected = if cfg!(target_os = "macos") {
        canonical_text(&shell)
    } else {
        DEFAULT_SH.to_owned()
    };
    assert_eq!(resolve_sh(&selector), expected);
    assert_eq!(resolve_sh(&dir.path().join("missing")), DEFAULT_SH);
}
