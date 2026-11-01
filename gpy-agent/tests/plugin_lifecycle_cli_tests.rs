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

/// Out-of-sandbox env vars that point discovery at the repo's bundled plugins.
fn bundled_env(env: &CliTestEnv) -> Vec<(&'static str, String)> {
    vec![(
        "GPY_BUNDLED_PLUGIN_DIR",
        env.bundled_plugins_dir().to_string_lossy().into_owned(),
    )]
}

fn run_with_bundled(env: &CliTestEnv, args: &[&str]) -> common::cli_harness::CliCommandResult {
    env.run_gpy_with_env(args, &bundled_env(env))
        .expect("spawn gpy")
}

fn count_entries(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir).map_or(0_usize, Iterator::count)
}

#[cfg(unix)]
fn set_mode(path: &std::path::Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
}

#[cfg(unix)]
fn mode_of(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::symlink_metadata(path)
        .expect("stat")
        .permissions()
        .mode()
        & 0o777
}

/// Every path under `root` (and `root`) must lack group/other write bits.
#[cfg(unix)]
fn assert_no_shared_write(root: &std::path::Path) {
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        assert_eq!(mode_of(&path) & 0o022, 0_u32, "{}", path.display());
        if path.is_dir() {
            for entry in std::fs::read_dir(&path).expect("read dir") {
                pending.push(entry.expect("entry").path());
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn install_creates_missing_parents_without_group_write_under_umask_0002() {
    let (_source_env, dir) = scaffold("foo");
    let env = CliTestEnv::new().expect("create env");
    std::fs::remove_dir(env.plugins_dir()).expect("remove plugins dir");
    assert!(!env.config_dir().join("plugins.bak").exists());
    assert!(!env.config_dir().join("plugins.staging").exists());

    let dir_arg = dir.to_string_lossy().into_owned();
    let gpy = env!("CARGO_BIN_EXE_gpy");
    let no_env: &[(&str, &str)] = &[];
    let result = env
        .run_program_with_env(
            std::path::Path::new("/bin/sh"),
            &[
                "-c",
                "umask 0002; exec \"$0\" \"$@\"",
                gpy,
                "plugin",
                "install",
                dir_arg.as_str(),
            ],
            no_env,
        )
        .expect("spawn sh");
    result.assert_success("gpy plugin install under umask 0002");

    assert_eq!(mode_of(&env.plugins_dir()) & 0o022, 0_u32);
    for name in ["plugins.staging", "plugins.bak"] {
        let path = env.config_dir().join(name);
        if path.exists() {
            assert_eq!(mode_of(&path) & 0o022, 0_u32, "{name}");
        }
    }
    assert_no_shared_write(&env.plugins_dir().join("foo"));
    let list = env.run_gpy(&["plugin", "list"]).expect("spawn gpy");
    assert!(list.stdout.contains("foo 0.1.0"), "{list:?}");
    assert!(list.stdout.contains("(ready)"), "{list:?}");
}

#[cfg(unix)]
#[test]
fn install_fixes_permissive_source_modes() {
    let (_source_env, dir) = scaffold("foo");
    set_mode(&dir, 0o775);
    set_mode(&dir.join("bin"), 0o775);
    set_mode(&dir.join("bin/foo.sh"), 0o664);
    let env = CliTestEnv::new().expect("create env");

    install(&env, &dir, &[]).assert_success("install from permissive source");

    let installed = env.plugins_dir().join("foo");
    assert_no_shared_write(&installed);
    assert_eq!(mode_of(&installed.join("bin/foo.sh")), 0o644_u32);
    assert_eq!(mode_of(&installed.join("bin")), 0o755_u32);
    let list = env.run_gpy(&["plugin", "list"]).expect("spawn gpy");
    assert!(list.stdout.contains("(ready)"), "{list:?}");
}

#[cfg(unix)]
#[test]
fn install_keeps_the_owner_execute_bit() {
    let (_source_env, dir) = scaffold("foo");
    set_mode(&dir.join("bin/foo.sh"), 0o775);
    let env = CliTestEnv::new().expect("create env");

    install(&env, &dir, &[]).assert_success("install executable source");

    let script = env.plugins_dir().join("foo/bin/foo.sh");
    assert_eq!(mode_of(&script), 0o755_u32);
}

#[test]
fn install_accepts_a_file_entry_plugin() {
    let source_env = CliTestEnv::new().expect("create source env");
    let dir = source_env.root().join("filesrc");
    std::fs::create_dir_all(dir.join("segments")).expect("create segments");
    std::fs::write(
        dir.join("plugin.toml"),
        "id = \"filey\"\nname = \"Filey\"\nversion = \"1.0.0\"\napi_version = \"v1\"\n\
         entry_type = \"file\"\nprovided_segments = [\"filey\"]\n",
    )
    .expect("write manifest");
    std::fs::write(dir.join("segments/filey.fish"), "echo hi\n").expect("write fish");
    let env = CliTestEnv::new().expect("create env");

    let result = install(&env, &dir, &[]);
    result.assert_success("install file plugin");
    assert!(result.stdout.contains("gpy enable filey"), "{result:?}");
    assert!(
        env.plugins_dir()
            .join("filey/segments/filey.fish")
            .is_file()
    );
    let list = env.run_gpy(&["plugin", "list"]).expect("spawn gpy");
    assert!(list.stdout.contains("filey 1.0.0"), "{list:?}");
    assert!(list.stdout.contains("(ready)"), "{list:?}");
}

#[test]
fn install_refuses_a_file_entry_plugin_without_its_fish_file() {
    let source_env = CliTestEnv::new().expect("create source env");
    let dir = source_env.root().join("filesrc");
    std::fs::create_dir_all(&dir).expect("create dir");
    std::fs::write(
        dir.join("plugin.toml"),
        "id = \"filey\"\nname = \"Filey\"\nversion = \"1.0.0\"\napi_version = \"v1\"\n\
         entry_type = \"file\"\nprovided_segments = [\"filey\"]\n",
    )
    .expect("write manifest");
    let env = CliTestEnv::new().expect("create env");

    let result = install(&env, &dir, &[]);
    assert_eq!(result.exit_code, 1_i32, "{result:?}");
    assert!(!env.plugins_dir().join("filey").exists());
}

#[test]
fn install_force_backs_up_the_old_copy_outside_the_plugins_root() {
    let (_source_env, dir) = scaffold("foo");
    let env = CliTestEnv::new().expect("create env");
    install(&env, &dir, &[]).assert_success("first install");
    let marker = env.plugins_dir().join("foo/old-marker");
    std::fs::write(&marker, "old").expect("write marker");

    let before = count_entries(&env.plugins_dir());
    let refused = install(&env, &dir, &[]);
    assert_eq!(refused.exit_code, 1_i32, "{refused:?}");
    assert!(marker.exists(), "refused install must change nothing");
    assert!(!env.config_dir().join("plugins.bak").exists());

    install(&env, &dir, &["--force"]).assert_success("forced install");
    assert_eq!(count_entries(&env.plugins_dir()), before);
    assert!(!marker.exists());
    let backups: Vec<_> = std::fs::read_dir(env.config_dir().join("plugins.bak"))
        .expect("plugins.bak exists")
        .map(|e| e.expect("entry").path())
        .collect();
    assert_eq!(backups.len(), 1_usize);
    assert!(
        backups
            .first()
            .expect("one backup")
            .join("old-marker")
            .is_file()
    );
    let backup = backups.first().expect("one backup");
    let name = backup.file_name().expect("name").to_string_lossy();
    assert!(name.starts_with("foo."), "{name}");
    let list = env.run_gpy(&["plugin", "list"]).expect("spawn gpy");
    assert_eq!(
        list.stdout.matches("foo 0.1.0").count(),
        1_usize,
        "{list:?}"
    );
}

#[test]
fn install_needs_force_to_shadow_a_bundled_plugin() {
    let source_env = CliTestEnv::new().expect("create source env");
    source_env
        .run_gpy(&["plugin", "new", "hostname", "--segment", "myhost"])
        .expect("spawn gpy")
        .assert_success("gpy plugin new hostname");
    let dir = source_env.plugins_dir().join("hostname");
    let env = CliTestEnv::new().expect("create env");
    let dir_arg = dir.to_string_lossy().into_owned();

    let refused = run_with_bundled(&env, &["plugin", "install", dir_arg.as_str()]);
    assert_eq!(refused.exit_code, 1_i32, "{refused:?}");
    assert!(refused.stderr.contains("bundled"), "{refused:?}");
    assert!(!env.plugins_dir().join("hostname").exists());

    let forced = run_with_bundled(&env, &["plugin", "install", dir_arg.as_str(), "--force"]);
    forced.assert_success("forced shadow install");
    assert!(
        forced.stdout.contains("shadows the bundled plugin"),
        "{forced:?}"
    );
    assert!(env.plugins_dir().join("hostname/plugin.toml").is_file());
}

#[test]
fn install_finds_an_existing_plugin_by_id_not_directory_name() {
    let (_source_env, dir) = scaffold("foo");
    let env = CliTestEnv::new().expect("create env");
    install(&env, &dir, &[]).assert_success("first install");
    std::fs::rename(
        env.plugins_dir().join("foo"),
        env.plugins_dir().join("foo-old"),
    )
    .expect("rename to foo-old");

    let refused = install(&env, &dir, &[]);
    assert_eq!(refused.exit_code, 1_i32, "{refused:?}");
    assert!(refused.stderr.contains("foo-old"), "{refused:?}");
    assert!(!env.plugins_dir().join("foo").exists());

    install(&env, &dir, &["--force"]).assert_success("forced install");
    assert!(!env.plugins_dir().join("foo-old").exists());
    assert!(env.plugins_dir().join("foo/plugin.toml").is_file());
    assert_eq!(
        count_entries(&env.config_dir().join("plugins.bak")),
        1_usize
    );

    // Removal also finds the plugin by id wherever its directory lives.
    std::fs::rename(
        env.plugins_dir().join("foo"),
        env.plugins_dir().join("foo-old"),
    )
    .expect("rename again");
    env.run_gpy(&["plugin", "remove", "foo"])
        .expect("spawn gpy")
        .assert_success("remove foo");
    assert!(!env.plugins_dir().join("foo-old").exists());
}

#[test]
fn install_ignores_unrelated_broken_plugins() {
    let (_source_env, dir) = scaffold("foo");
    let env = CliTestEnv::new().expect("create env");
    let broken = env.plugins_dir().join("broken");
    std::fs::create_dir_all(&broken).expect("create broken");
    std::fs::write(broken.join("plugin.toml"), "this is = not a manifest [").expect("write");

    let result = install(&env, &dir, &[]);
    result.assert_success("install beside a broken plugin");
    assert!(env.plugins_dir().join("foo/plugin.toml").is_file());
    assert!(broken.join("plugin.toml").is_file());
}

#[test]
fn install_leaves_nothing_behind_when_validation_fails() {
    let (_source_env, dir) = scaffold("foo");
    let manifest = std::fs::read_to_string(dir.join("plugin.toml")).expect("read manifest");
    std::fs::write(
        dir.join("plugin.toml"),
        manifest.replace("id = \"foo\"", "id = \"Bad ID!\""),
    )
    .expect("write manifest");
    let env = CliTestEnv::new().expect("create env");
    let before = count_entries(&env.plugins_dir());

    let result = install(&env, &dir, &[]);
    assert_eq!(result.exit_code, 1_i32, "{result:?}");
    assert_eq!(count_entries(&env.plugins_dir()), before);
    assert_eq!(
        count_entries(&env.config_dir().join("plugins.staging")),
        0_usize
    );
    assert!(!env.config_dir().join("plugins.bak").exists());
}

#[test]
fn install_over_a_directory_without_a_manifest_is_guarded() {
    let (_source_env, dir) = scaffold("foo");
    let env = CliTestEnv::new().expect("create env");
    let target = env.plugins_dir().join("foo");
    std::fs::create_dir_all(&target).expect("create target");
    std::fs::write(target.join("notes.txt"), "keep me").expect("write notes");

    let refused = install(&env, &dir, &[]);
    assert_eq!(refused.exit_code, 1_i32, "{refused:?}");
    assert!(
        refused.stderr.contains(&*target.to_string_lossy()),
        "{refused:?}"
    );
    assert!(target.join("notes.txt").is_file());
    assert!(!target.join("plugin.toml").exists());
    assert!(!env.config_dir().join("plugins.bak").exists());

    install(&env, &dir, &["--force"]).assert_success("forced install");
    assert!(target.join("plugin.toml").is_file());
    let backups: Vec<_> = std::fs::read_dir(env.config_dir().join("plugins.bak"))
        .expect("plugins.bak exists")
        .map(|e| e.expect("entry").path())
        .collect();
    assert_eq!(backups.len(), 1_usize);
    assert!(
        backups
            .first()
            .expect("one backup")
            .join("notes.txt")
            .is_file()
    );
}

#[test]
fn remove_refuses_a_bundled_plugin() {
    let env = CliTestEnv::new().expect("create env");
    let result = run_with_bundled(&env, &["plugin", "remove", "hostname"]);
    assert_eq!(result.exit_code, 1_i32, "{result:?}");
    assert!(result.stderr.contains("gpy disable hostname"), "{result:?}");
    assert!(!env.config_dir().join("plugins.bak").exists());
}

#[test]
fn remove_leaves_the_plugin_when_the_id_is_path_like() {
    let (_source_env, dir) = scaffold("foo");
    let env = CliTestEnv::new().expect("create env");
    install(&env, &dir, &[]).assert_success("install");
    let result = env
        .run_gpy(&["plugin", "remove", "../plugins/foo"])
        .expect("spawn gpy");
    assert_eq!(result.exit_code, 1_i32, "{result:?}");
    assert!(env.plugins_dir().join("foo/plugin.toml").is_file());
    assert!(!env.config_dir().join("plugins.bak").exists());
}
