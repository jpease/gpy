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
}
