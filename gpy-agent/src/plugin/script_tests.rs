#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::missing_panics_doc,
    clippy::shadow_unrelated,
    reason = "tests: panics are the assertions, and each `err` binds a fresh case"
)]

use super::*;
use std::fs;
use tempfile::TempDir;

fn parse_err(text: &str) -> String {
    ScriptSegmentManifest::parse(text).unwrap_err().to_string()
}

#[test]
fn minimal_manifest_takes_documented_defaults() {
    let manifest = ScriptSegmentManifest::parse("command = \"bin/seg.sh\"\n").unwrap();
    assert_eq!(manifest.command.as_str(), "bin/seg.sh");
    assert_eq!(manifest.ttl_secs.get(), 5);
    assert_eq!(manifest.timeout_ms.get(), 500);
    assert!(manifest.inputs.is_empty() && manifest.env.is_empty());
    assert_eq!(manifest.cache_scope(), CacheScope::Session);
}

#[test]
fn full_manifest_parses() {
    let manifest = ScriptSegmentManifest::parse(
        r#"
        description = "Kubernetes context"
        command = "./bin/k8s.sh"
        inputs = ["pwd", "is_root", "is_sudo", "is_ssh", "cmd_duration", "prompt_icons"]
        env = ["KUBECONFIG", "_x1"]
        ttl_secs = 0
        timeout_ms = 120
        require_files = ["Chart.yaml", ".envrc"]
        require_folders = [".git"]
        require_extensions = ["tf", "tar.gz"]
        require_env = ["KUBECONFIG"]
        require_command = "bin/when.sh"
        "#,
    )
    .unwrap();
    assert_eq!(manifest.inputs, InputName::ALL);
    assert_eq!(manifest.ttl_secs.get(), 0);
    assert_eq!(manifest.timeout_ms.get(), 120);
    assert_eq!(manifest.cache_scope(), CacheScope::Folder);
    assert_eq!(
        manifest.require_command.as_ref().map(ScriptPath::as_str),
        Some("bin/when.sh")
    );
}

#[test]
fn input_env_vars_are_gpy_prefixed_upper_case() {
    for input in InputName::ALL {
        assert_eq!(
            input.env_var(),
            format!("GPY_{}", input.as_str().to_ascii_uppercase())
        );
    }
}

#[test]
fn unknown_keys_and_inputs_are_rejected_by_name() {
    let err = parse_err("command = \"a.sh\"\nwhen = \"true\"\n");
    assert!(err.contains("when"), "{err}");
    let err = parse_err("command = \"a.sh\"\ninputs = [\"hostname\"]\n");
    assert!(err.contains("hostname"), "{err}");
    let err = parse_err("inputs = []\n");
    assert!(err.contains("command"), "{err}");
}

#[test]
fn timeout_may_only_be_lowered() {
    for bad in ["0", "501"] {
        let err = parse_err(&format!("command = \"a.sh\"\ntimeout_ms = {bad}\n"));
        assert!(
            err.contains("timeout_ms") && err.contains("1..=500"),
            "{err}"
        );
    }
    let err = parse_err("command = \"a.sh\"\nttl_secs = 31536001\n");
    assert!(err.contains("ttl_secs"), "{err}");
}

#[test]
fn env_names_are_validated_before_use() {
    for bad in ["1A", "a-b", "a[$(touch x)]", "x;touch y", "É", ""] {
        assert!(EnvName::new(bad).is_err(), "{bad:?} must be rejected");
        let err = parse_err(&format!("command = \"a.sh\"\nenv = [{bad:?}]\n"));
        assert!(err.contains("environment variable name"), "{err}");
        let err = parse_err(&format!("command = \"a.sh\"\nrequire_env = [{bad:?}]\n"));
        assert!(err.contains("environment variable name"), "{err}");
    }
    for good in ["A", "_", "a_1", "KUBECONFIG"] {
        assert!(EnvName::new(good).is_ok(), "{good:?}");
    }
}

#[test]
fn reserved_and_duplicate_names_are_rejected() {
    let err = parse_err("command = \"a.sh\"\nenv = [\"GPY_OPT_style\"]\n");
    assert!(err.contains("env: 'GPY_OPT_style' is reserved"), "{err}");
    for (key, value) in [
        ("env", "\"A\", \"A\""),
        ("inputs", "\"pwd\", \"pwd\""),
        ("require_files", "\"x\", \"x\""),
        ("require_folders", "\"x\", \"x\""),
        ("require_extensions", "\"tf\", \"tf\""),
        ("require_env", "\"A\", \"A\""),
    ] {
        let err = parse_err(&format!("command = \"a.sh\"\n{key} = [{value}]\n"));
        assert!(
            err.contains(&format!("{key}: '")) && err.contains("twice"),
            "{err}"
        );
    }
}

#[test]
fn script_paths_stay_relative() {
    for bad in [
        "",
        "/bin/sh",
        "../x.sh",
        "bin/../../x.sh",
        "a\\b.sh",
        "a\nb",
    ] {
        assert!(ScriptPath::new(bad).is_err(), "{bad:?} must be rejected");
    }
    for good in ["seg.sh", "./bin/seg.sh", "bin/seg"] {
        assert!(ScriptPath::new(good).is_ok(), "{good:?}");
    }
}

#[test]
fn scan_names_and_extensions_are_single_components() {
    for bad in ["", ".", "..", "a/b", "a\\b", "a\tb"] {
        assert!(ScanName::new(bad).is_err(), "{bad:?}");
    }
    assert!(ScanName::new(".git").is_ok());
    for bad in ["", ".tf", "t f", "a/b"] {
        assert!(Extension::new(bad).is_err(), "{bad:?}");
    }
    for good in ["tf", "tar.gz", "c++", "d-ts"] {
        assert!(Extension::new(good).is_ok(), "{good:?}");
    }
}

#[test]
fn variant_names_follow_the_output_protocol() {
    for good in ["warn", "prod_1", "a", &"x".repeat(32)] {
        assert!(VariantName::new(good).is_ok(), "{good:?}");
    }
    for bad in ["", "Warn", "a-b", "a b", &"x".repeat(33)] {
        assert!(VariantName::new(bad).is_err(), "{bad:?}");
    }
}

fn plugin_root() -> (TempDir, SegmentName) {
    let temp = TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("segments")).unwrap();
    fs::create_dir_all(temp.path().join("bin")).unwrap();
    fs::write(temp.path().join("bin/seg.sh"), "echo hi\n").unwrap();
    (temp, SegmentName::new("seg").unwrap())
}

#[test]
fn load_checks_the_script_exists_inside_the_root() {
    let (temp, segment) = plugin_root();
    let manifest_path = script_segment_manifest_path(temp.path(), &segment);

    fs::write(&manifest_path, "command = \"bin/seg.sh\"\n").unwrap();
    ScriptSegmentManifest::load(temp.path(), &segment).unwrap();

    fs::write(&manifest_path, "command = \"bin/missing.sh\"\n").unwrap();
    let err = ScriptSegmentManifest::load(temp.path(), &segment)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("seg.toml") && err.contains("command:") && err.contains("not found"),
        "{err}"
    );

    fs::write(&manifest_path, "command = \"bin\"\n").unwrap();
    let err = ScriptSegmentManifest::load(temp.path(), &segment)
        .unwrap_err()
        .to_string();
    assert!(err.contains("not a regular file"), "{err}");

    fs::write(
        &manifest_path,
        "command = \"bin/seg.sh\"\nrequire_command = \"bin/nope.sh\"\n",
    )
    .unwrap();
    let err = ScriptSegmentManifest::load(temp.path(), &segment)
        .unwrap_err()
        .to_string();
    assert!(err.contains("require_command:"), "{err}");
}

#[test]
fn load_names_the_file_for_parse_errors() {
    let (temp, segment) = plugin_root();
    fs::write(
        script_segment_manifest_path(temp.path(), &segment),
        "command = \"bin/seg.sh\"\ntimeout_ms = 900\n",
    )
    .unwrap();
    let err = ScriptSegmentManifest::load(temp.path(), &segment)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("seg.toml") && err.contains("timeout_ms"),
        "{err}"
    );
}

#[cfg(unix)]
#[test]
fn load_rejects_a_symlink_out_of_the_root() {
    let (temp, segment) = plugin_root();
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("evil.sh"), "echo evil\n").unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("evil.sh"),
        temp.path().join("bin/link.sh"),
    )
    .unwrap();
    fs::write(
        script_segment_manifest_path(temp.path(), &segment),
        "command = \"bin/link.sh\"\n",
    )
    .unwrap();
    let err = ScriptSegmentManifest::load(temp.path(), &segment)
        .unwrap_err()
        .to_string();
    assert!(err.contains("outside the plugin root"), "{err}");

    fs::write(
        script_segment_manifest_path(temp.path(), &segment),
        "command = \"bin/seg.sh\"\nrequire_command = \"bin/link.sh\"\n",
    )
    .unwrap();
    let err = ScriptSegmentManifest::load(temp.path(), &segment)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("require_command:") && err.contains("outside the plugin root"),
        "{err}"
    );
}

#[test]
fn resolve_returns_the_canonical_path() {
    let (temp, _) = plugin_root();
    let resolved = ScriptPath::new("./bin/seg.sh")
        .unwrap()
        .resolve(temp.path())
        .unwrap();
    assert!(resolved.is_absolute(), "{resolved:?}");
    assert_eq!(
        resolved,
        temp.path().canonicalize().unwrap().join("bin/seg.sh")
    );
}

/// The script and every directory up to the plugin root must be writable by
/// the user only: a group- or world-writable one is refused, naming the path.
#[cfg(unix)]
#[test]
fn resolve_refuses_a_script_or_directory_others_can_write() {
    use std::os::unix::fs::PermissionsExt;

    let (temp, _) = plugin_root();
    let script = ScriptPath::new("bin/seg.sh").unwrap();
    let set_mode = |relative: &str, mode: u32| {
        fs::set_permissions(temp.path().join(relative), fs::Permissions::from_mode(mode)).unwrap();
    };
    set_mode("", 0o700);
    set_mode("bin", 0o755);
    set_mode("bin/seg.sh", 0o644);
    script.resolve(temp.path()).unwrap();

    for (relative, mode, restore) in [
        ("bin/seg.sh", 0o664, 0o644),
        ("bin/seg.sh", 0o646, 0o644),
        ("bin", 0o775, 0o755),
        ("", 0o777, 0o700),
    ] {
        set_mode(relative, mode);
        let err = script.resolve(temp.path()).unwrap_err().to_string();
        assert!(
            err.contains("bin/seg.sh") && err.contains("group- or world-writable"),
            "{relative:?} {mode:o}: {err}"
        );
        set_mode(relative, restore);
    }
    script.resolve(temp.path()).unwrap();
}

/// Build `<outer>/<middle>/plugin` with private modes and return the plugin
/// root; `outer` and `middle` keep the modes the caller sets afterwards.
#[cfg(unix)]
fn nested_plugin(outer: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let middle = outer.join("middle");
    let root = middle.join("plugin");
    fs::create_dir_all(root.join("bin")).unwrap();
    for dir in [&middle, &root, &root.join("bin")] {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(root.join("bin/seg.sh"), "echo hi\n").unwrap();
    fs::set_permissions(root.join("bin/seg.sh"), fs::Permissions::from_mode(0o644)).unwrap();
    root
}

/// Replacing the plugin root needs write access to its parent, so a
/// group- or world-writable ancestor without the sticky bit (immediate parent
/// or higher) is refused, naming that ancestor, even when the plugin itself
/// is private; a sticky one owned by the user stays usable.
#[cfg(unix)]
#[test]
fn resolve_refuses_a_replaceable_ancestor_of_the_root() {
    use std::os::unix::fs::PermissionsExt;

    let outer = TempDir::new().unwrap();
    let root = nested_plugin(outer.path());
    let middle = root.parent().unwrap().to_path_buf();
    let script = ScriptPath::new("bin/seg.sh").unwrap();
    let set_mode = |dir: &Path, mode: u32| {
        fs::set_permissions(dir, fs::Permissions::from_mode(mode)).unwrap();
    };
    set_mode(outer.path(), 0o755);
    script.resolve(&root).unwrap();

    let canonical_outer = outer.path().canonicalize().unwrap();
    let canonical_middle = middle.canonicalize().unwrap();
    for (dir, named) in [
        (&middle, &canonical_middle),
        (&outer.path().to_path_buf(), &canonical_outer),
    ] {
        for mode in [0o777, 0o757, 0o707] {
            set_mode(dir, mode);
            let err = script.resolve(&root).unwrap_err().to_string();
            let shown = named.to_string_lossy();
            assert!(
                err.contains(shown.as_ref()) && err.contains("sticky"),
                "{dir:?} {mode:o}: {err}"
            );
        }
        set_mode(dir, 0o1777);
        script.resolve(&root).unwrap();
        set_mode(dir, 0o755);
    }
    script.resolve(&root).unwrap();
}

/// A group-writable (not world-writable) ancestor is accepted only through a
/// group the user alone belongs to, as `$HOME` is under user-private-group
/// distros with umask 002; otherwise it is refused. The directory is created
/// by the test user, so its group is the user's primary group.
#[cfg(unix)]
#[test]
fn resolve_accepts_a_group_writable_ancestor_only_in_a_private_group() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let outer = TempDir::new().unwrap();
    let root = nested_plugin(outer.path());
    let middle = root.parent().unwrap();
    fs::set_permissions(outer.path(), fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(middle, fs::Permissions::from_mode(0o775)).unwrap();
    let private =
        crate::plugin::script_ancestry::group_is_private(middle.metadata().unwrap().gid());
    let result = ScriptPath::new("bin/seg.sh").unwrap().resolve(&root);
    if private {
        result.unwrap();
    } else {
        let err = result.unwrap_err().to_string();
        assert!(err.contains("sticky"), "{err}");
    }
    fs::set_permissions(middle, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(
        ScriptPath::new("bin/seg.sh")
            .unwrap()
            .resolve(&root)
            .is_err()
    );
}

/// A discovered plugin root that is a symlink is judged by the canonical
/// location it points to, not by the (safe) directory holding the link.
#[cfg(unix)]
#[test]
fn resolve_checks_the_ancestry_of_a_symlinked_root() {
    use std::os::unix::fs::PermissionsExt;

    let shared = TempDir::new().unwrap();
    let physical = nested_plugin(shared.path());
    let linked = TempDir::new().unwrap();
    let link = linked.path().join("plugin");
    std::os::unix::fs::symlink(&physical, &link).unwrap();
    let script = ScriptPath::new("bin/seg.sh").unwrap();
    fs::set_permissions(shared.path(), fs::Permissions::from_mode(0o755)).unwrap();
    script.resolve(&link).unwrap();

    let middle = physical.parent().unwrap();
    fs::set_permissions(middle, fs::Permissions::from_mode(0o777)).unwrap();
    let err = script.resolve(&link).unwrap_err().to_string();
    assert!(
        err.contains(&middle.canonicalize().unwrap().to_string_lossy().to_string())
            && err.contains("replace"),
        "{err}"
    );
}

/// `load` (used by export and `plugin validate`) applies the ancestry check to
/// `command` and to `require_command`, which resolve through the same path.
#[cfg(unix)]
#[test]
fn load_refuses_a_replaceable_ancestor_for_both_scripts() {
    use std::os::unix::fs::PermissionsExt;

    let outer = TempDir::new().unwrap();
    let root = nested_plugin(outer.path());
    fs::set_permissions(outer.path(), fs::Permissions::from_mode(0o755)).unwrap();
    fs::create_dir_all(root.join("segments")).unwrap();
    fs::set_permissions(root.join("segments"), fs::Permissions::from_mode(0o755)).unwrap();
    let segment = SegmentName::new("seg").unwrap();
    let manifest = script_segment_manifest_path(&root, &segment);
    fs::write(
        &manifest,
        "command = \"bin/seg.sh\"\nrequire_command = \"bin/seg.sh\"\n",
    )
    .unwrap();
    fs::set_permissions(&manifest, fs::Permissions::from_mode(0o644)).unwrap();
    ScriptSegmentManifest::load(&root, &segment).unwrap();

    fs::set_permissions(root.parent().unwrap(), fs::Permissions::from_mode(0o777)).unwrap();
    let err = ScriptSegmentManifest::load(&root, &segment)
        .unwrap_err()
        .to_string();
    assert!(err.contains("sticky") && err.contains("replace"), "{err}");
}

/// Rejected values are echoed `{:?}`-escaped: ESC and CSI (U+009B) from a
/// manifest never reach the terminal raw.
#[test]
fn rejected_values_are_echoed_escaped() {
    let hostile = "a\u{1b}]0;x\u{7}\u{9b}31m";
    for err in [
        EnvName::new(hostile).unwrap_err(),
        ScriptPath::new(hostile).unwrap_err(),
        ScanName::new(hostile).unwrap_err(),
        Extension::new(hostile).unwrap_err(),
        crate::plugin::SettingKey::new(hostile).unwrap_err(),
        VariantName::new(hostile).unwrap_err(),
        VariantColorKey::new(&format!("{hostile}_bg_color")).unwrap_err(),
    ] {
        let message = err.to_string();
        assert!(!message.chars().any(char::is_control), "{message:?}");
        assert!(
            message.contains(r"\u{1b}") && message.contains(r"\u{9b}"),
            "{message}"
        );
    }
}
