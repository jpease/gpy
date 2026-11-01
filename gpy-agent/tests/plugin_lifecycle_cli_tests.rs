//! `gpy plugin install` and `gpy plugin remove`: copying a plugin into the
//! user plugins directory with trusted modes, and removing it with a backup.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests fail loudly on a broken sandbox or an unexpected result"
)]

mod common;

use common::cli_harness::CliTestEnv;
use std::path::PathBuf;

/// Scaffold plugin `id` with `gpy plugin new` in a throwaway env; returns the
/// env (kept alive) and the plugin directory.
fn scaffold(id: &str) -> (CliTestEnv, PathBuf) {
    let source_env = CliTestEnv::new().expect("create source env");
    source_env
        .run_gpy(&["plugin", "new", id])
        .expect("spawn gpy")
        .assert_success("gpy plugin new");
    let dir = source_env.plugins_dir().join(id);
    (source_env, dir)
}

fn install(
    env: &CliTestEnv,
    dir: &std::path::Path,
    extra: &[&str],
) -> common::cli_harness::CliCommandResult {
    let dir_arg = dir.to_string_lossy().into_owned();
    let mut args = vec!["plugin", "install", dir_arg.as_str()];
    args.extend_from_slice(extra);
    env.run_gpy(&args).expect("spawn gpy")
}

#[test]
fn install_copies_a_scaffold_and_lists_it_ready() {
    let (_source_env, dir) = scaffold("foo");
    let env = CliTestEnv::new().expect("create env");
    std::fs::remove_dir(env.plugins_dir()).expect("remove plugins dir");

    let result = install(&env, &dir, &[]);
    result.assert_success("gpy plugin install");
    assert!(result.stdout.contains("gpy enable foo"), "{result:?}");

    let list = env.run_gpy(&["plugin", "list"]).expect("spawn gpy");
    assert!(list.stdout.contains("foo 0.1.0"), "{list:?}");
    assert!(list.stdout.contains("(ready)"), "{list:?}");
}

#[test]
fn install_refuses_an_existing_plugin_without_force() {
    let (_source_env, dir) = scaffold("foo");
    let env = CliTestEnv::new().expect("create env");
    install(&env, &dir, &[]).assert_success("first install");

    let again = install(&env, &dir, &[]);
    assert_eq!(again.exit_code, 1_i32, "{again:?}");

    install(&env, &dir, &["--force"]).assert_success("forced install");
    let backups = std::fs::read_dir(env.config_dir().join("plugins.bak"))
        .expect("plugins.bak exists")
        .count();
    assert_eq!(backups, 1_usize);
}

#[test]
fn remove_refuses_a_path_like_id() {
    let env = CliTestEnv::new().expect("create env");
    for id in ["../themes", "a/b"] {
        let result = env.run_gpy(&["plugin", "remove", id]).expect("spawn gpy");
        assert_eq!(result.exit_code, 1_i32, "{id}: {result:?}");
    }
    assert!(env.themes_dir().is_dir());
}

#[test]
fn remove_disables_segments_and_backs_up_the_directory() {
    let (_source_env, dir) = scaffold("foo");
    let env = CliTestEnv::new().expect("create env");
    install(&env, &dir, &[]).assert_success("install");
    env.run_gpy(&["enable", "foo"])
        .expect("spawn gpy")
        .assert_success("gpy enable foo");

    env.run_gpy(&["plugin", "remove", "foo"])
        .expect("spawn gpy")
        .assert_success("gpy plugin remove foo");
    assert!(!env.plugins_dir().join("foo").exists());
    assert_eq!(
        std::fs::read_dir(env.config_dir().join("plugins.bak"))
            .expect("plugins.bak exists")
            .count(),
        1_usize
    );
    let config = std::fs::read_to_string(env.config_path()).expect("read config");
    assert!(!config.contains("\"foo\""), "{config}");
}

#[test]
fn remove_explains_an_unknown_or_unloadable_id() {
    let env = CliTestEnv::new().expect("create env");
    let unknown = env
        .run_gpy(&["plugin", "remove", "nosuch"])
        .expect("spawn gpy");
    assert_eq!(unknown.exit_code, 1_i32);
    assert!(
        unknown.stderr.contains("no user plugin 'nosuch'"),
        "{unknown:?}"
    );

    std::fs::create_dir_all(env.plugins_dir().join("broken")).expect("create broken dir");
    let broken = env
        .run_gpy(&["plugin", "remove", "broken"])
        .expect("spawn gpy");
    assert_eq!(broken.exit_code, 1_i32);
    assert!(
        broken
            .stderr
            .contains("exists but is not a loadable plugin"),
        "{broken:?}"
    );
    assert!(env.plugins_dir().join("broken").is_dir());
}

#[cfg(unix)]
#[test]
fn install_refuses_a_source_with_a_symlink() {
    let (_source_env, dir) = scaffold("foo");
    std::os::unix::fs::symlink("/etc/hosts", dir.join("link")).expect("symlink");
    let env = CliTestEnv::new().expect("create env");

    let result = install(&env, &dir, &[]);
    assert_eq!(result.exit_code, 1_i32, "{result:?}");
    assert!(!env.plugins_dir().join("foo").exists());
    assert!(!env.config_dir().join("plugins.staging").exists());
}
