//! Tests for `ThemeManager` reload and watching across user, plugin and builtin
//! theme sources. Isolation as in `theme_manager_tests.rs`: `EnvGuard` plus
//! `#[serial]`.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

#[path = "common/env_guard.rs"]
mod env_guard;
#[path = "common/wait.rs"]
mod wait;

use env_guard::EnvGuard;
use gpy_agent::theme::ThemeManager;
use serial_test::serial;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::TempDir;
use wait::wait_until;

/// A minimal valid theme told apart by its prompt symbol.
fn icon_theme(icon: &str) -> String {
    format!("[ui]\n\n[segments.character]\nsuccess_symbol = \"{icon}\"\n")
}

/// Install plugin `acme` providing theme `name` with `content`; returns the
/// plugin's directory. The manifest is written last, so a poll never sees
/// the plugin without its theme.
fn install_acme_theme(config_home: &Path, name: &str, content: &str) -> PathBuf {
    let plugin_dir = config_home.join("gpy").join("plugins").join("acme");
    fs::create_dir_all(plugin_dir.join("themes")).unwrap();
    fs::write(
        plugin_dir.join("themes").join(format!("{name}.toml")),
        content,
    )
    .unwrap();
    fs::write(
        plugin_dir.join("plugin.toml"),
        "id = \"acme\"\nname = \"Acme\"\nversion = \"0.1.0\"\napi_version = \"v1\"\n\
         entry_type = \"file\"\nprovided_segments = [\"acme_seg\"]\n",
    )
    .unwrap();
    plugin_dir
}

/// Install plugin `acme` providing theme `community`; returns the user
/// override path that outranks it.
fn install_plugin_theme(config_home: &Path) -> PathBuf {
    install_acme_theme(config_home, "community", &icon_theme("P"));
    let user_themes = config_home.join("gpy").join("themes");
    fs::create_dir_all(&user_themes).unwrap();
    user_themes.join("community.toml")
}

fn prompt_symbol(manager: &ThemeManager) -> String {
    manager.get().segments.character.success_symbol.clone()
}

/// `reload` follows the active name when a user override is added above a
/// plugin or builtin theme, edited in place, or removed, and reports a change
/// only when the loaded theme actually differs.
#[test]
#[serial]
fn reload_follows_a_user_override_above_plugin_and_builtin_themes() {
    let temp_dir = TempDir::new().unwrap();
    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());
    let override_path = install_plugin_theme(temp_dir.path());

    let plugin = ThemeManager::new("community").unwrap();
    assert_eq!(prompt_symbol(&plugin), "P");
    assert!(!plugin.reload().unwrap(), "nothing changed on disk");
    fs::write(&override_path, icon_theme("U")).unwrap();
    assert!(plugin.reload().unwrap(), "the override replaces the plugin");
    assert_eq!(prompt_symbol(&plugin), "U");
    fs::write(&override_path, icon_theme("E")).unwrap();
    assert!(plugin.reload().unwrap(), "an in-place edit is a change");
    assert_eq!(prompt_symbol(&plugin), "E");
    assert!(
        !plugin.reload().unwrap(),
        "reloading an applied edit is not a change"
    );
    fs::remove_file(&override_path).unwrap();
    assert!(plugin.reload().unwrap(), "removal falls back to the plugin");
    assert_eq!(prompt_symbol(&plugin), "P");

    let builtin = ThemeManager::new("default").unwrap();
    let builtin_icon = prompt_symbol(&builtin);
    assert!(!builtin.reload().unwrap(), "the builtin never changes");
    let builtin_override = override_path.with_file_name("default.toml");
    fs::write(&builtin_override, icon_theme("U")).unwrap();
    assert!(
        builtin.reload().unwrap(),
        "the override replaces the builtin"
    );
    assert_eq!(prompt_symbol(&builtin), "U");
    fs::remove_file(&builtin_override).unwrap();
    assert!(
        builtin.reload().unwrap(),
        "removal falls back to the builtin"
    );
    assert_eq!(prompt_symbol(&builtin), builtin_icon);
}

/// A read-only config tree (no user themes dir, none creatable) costs only
/// override detection: watching still starts and follows the active source.
#[cfg(unix)]
#[test]
#[serial]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn watching_survives_an_uncreatable_user_themes_dir() {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = TempDir::new().unwrap();
    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());
    install_plugin_theme(temp_dir.path());
    let gpy_dir = temp_dir.path().join("gpy");
    fs::remove_dir(gpy_dir.join("themes")).unwrap();
    let plugin_path = gpy_dir.join("plugins/acme/themes/community.toml");

    let manager = ThemeManager::new("community").unwrap();
    fs::set_permissions(&gpy_dir, fs::Permissions::from_mode(0o500)).unwrap();
    let started = manager.start_watching(Duration::from_millis(50));
    let followed = started.is_ok()
        && wait_until(Duration::from_secs(10), Duration::from_millis(200), || {
            fs::write(&plugin_path, icon_theme("Q")).unwrap();
            prompt_symbol(&manager) == "Q"
        });
    manager.stop_watching();
    fs::set_permissions(&gpy_dir, fs::Permissions::from_mode(0o700)).unwrap();
    started.expect("watching must start without a user themes dir");
    assert!(
        followed,
        "edits to the active plugin theme must still reload"
    );
}

/// The watcher sees a user override created above an active plugin theme,
/// re-arms on the plugin source when the override is removed, and keeps
/// following later edits there.
#[test]
#[serial]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn watcher_follows_a_user_override_above_a_plugin_theme() {
    let temp_dir = TempDir::new().unwrap();
    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());
    let override_path = install_plugin_theme(temp_dir.path());
    let plugin_path = temp_dir
        .path()
        .join("gpy/plugins/acme/themes/community.toml");

    let manager = ThemeManager::new("community").unwrap();
    manager.start_watching(Duration::from_millis(50)).unwrap();

    // A watch has no "armed" signal: re-apply each idempotent change until seen.
    let wait_for_icon = |icon: &str, apply: &dyn Fn()| {
        wait_until(Duration::from_secs(10), Duration::from_millis(200), || {
            apply();
            prompt_symbol(&manager) == icon
        })
    };
    assert!(
        wait_for_icon("U", &|| fs::write(&override_path, icon_theme("U")).unwrap()),
        "a new user override must replace the plugin theme"
    );
    // Re-deleting a gone file makes no event, so if the OS drops or coalesces
    // the delete (seen under FSEvents load), each poll also rewrites an
    // unrelated file in the watched user themes dir: a reload on that event
    // must still resolve the plugin theme.
    let unrelated = override_path.with_file_name("unrelated.txt");
    let nudges = std::cell::Cell::new(0_u32);
    assert!(
        wait_for_icon("P", &|| {
            let _ = fs::remove_file(&override_path);
            nudges.set(nudges.get() + 1);
            fs::write(&unrelated, nudges.get().to_string()).unwrap();
        }),
        "removing the override must fall back to the plugin theme"
    );
    assert!(
        wait_for_icon("Q", &|| {
            fs::write(&plugin_path, icon_theme("Q")).unwrap();
        }),
        "edits to the plugin theme must reload after the fallback"
    );
    manager.stop_watching();
}

/// An overlay of `base` that sets only the root prompt icon.
fn overlay_theme(base: &str, root_icon: &str) -> String {
    format!("extends = \"{base}\"\n\n[ui]\nroot_prompt_icon = \"{root_icon}\"\n")
}

fn root_prompt_icon(manager: &ThemeManager) -> String {
    manager.get().ui.root_prompt_icon.to_string()
}

/// A user overlay of a plugin theme: the watcher follows edits to the plugin
/// base, whose `themes/` directory joins the watches, and to the overlay.
#[test]
#[serial]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn watcher_reloads_edits_to_an_overlay_and_its_plugin_base() {
    let temp_dir = TempDir::new().unwrap();
    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());
    let user_override = install_plugin_theme(temp_dir.path());
    let overlay_path = user_override.with_file_name("mine.toml");
    let base_path = temp_dir
        .path()
        .join("gpy/plugins/acme/themes/community.toml");
    fs::write(&overlay_path, overlay_theme("community", "1")).unwrap();

    let manager = ThemeManager::new("mine").unwrap();
    assert_eq!(
        manager.chain_for_test().to_string(),
        "mine (user) → community (plugin:acme)"
    );
    assert_eq!(prompt_symbol(&manager), "P");
    assert_eq!(root_prompt_icon(&manager), "1");
    manager.start_watching(Duration::from_millis(50)).unwrap();

    // A watch has no "armed" signal: re-apply each idempotent edit until seen.
    let base_edited = wait_until(Duration::from_secs(10), Duration::from_millis(200), || {
        fs::write(&base_path, icon_theme("Q")).unwrap();
        prompt_symbol(&manager) == "Q"
    });
    let overlay_edited = wait_until(Duration::from_secs(10), Duration::from_millis(200), || {
        fs::write(&overlay_path, overlay_theme("community", "2")).unwrap();
        root_prompt_icon(&manager) == "2"
    });
    manager.stop_watching();
    assert!(base_edited, "an edit to the plugin base must reload");
    assert!(overlay_edited, "an edit to the overlay must reload");
    assert_eq!(
        prompt_symbol(&manager),
        "Q",
        "the overlay is still merged over the edited base"
    );
}

/// An overlay created over the active builtin switches the source to the
/// chain `default (user) → default (builtin)`, inherits the builtin's other
/// keys, and reloads on its own later edits.
#[test]
#[serial]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn an_overlay_appearing_over_the_active_builtin_switches_the_source() {
    let temp_dir = TempDir::new().unwrap();
    let _env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());
    let user_themes = temp_dir.path().join("gpy").join("themes");
    fs::create_dir_all(&user_themes).unwrap();
    let overlay_path = user_themes.join("default.toml");

    let manager = ThemeManager::new("default").unwrap();
    let builtin_icon = prompt_symbol(&manager);
    assert_eq!(manager.chain_for_test().to_string(), "default (builtin)");
    manager.start_watching(Duration::from_millis(50)).unwrap();

    let appeared = wait_until(Duration::from_secs(10), Duration::from_millis(200), || {
        fs::write(&overlay_path, overlay_theme("default", "1")).unwrap();
        root_prompt_icon(&manager) == "1"
    });
    let switched_chain = manager.chain_for_test().to_string();
    let edited = wait_until(Duration::from_secs(10), Duration::from_millis(200), || {
        fs::write(&overlay_path, overlay_theme("default", "2")).unwrap();
        root_prompt_icon(&manager) == "2"
    });
    manager.stop_watching();
    assert!(appeared, "an overlay over the active builtin must reload");
    assert_eq!(switched_chain, "default (user) → default (builtin)");
    assert_eq!(
        prompt_symbol(&manager),
        builtin_icon,
        "the overlay inherits the builtin's other keys"
    );
    assert!(edited, "an edit to the new overlay must reload");
}

/// A plugin providing a self-named overlay's base moves the base from the
/// builtin to the plugin, and back when the plugin goes. The plugin's
/// directory is not watched before it appears, so the poll fallback, which
/// re-resolves the chain on every tick, must see it.
#[test]
#[serial]
#[serial_test::file_serial(watcher_fsevents_bootstrap)]
fn a_plugin_providing_a_self_named_base_appearing_and_disappearing_reloads() {
    let temp_dir = TempDir::new().unwrap();
    let mut env = EnvGuard::set_one("XDG_CONFIG_HOME", temp_dir.path());
    let user_themes = temp_dir.path().join("gpy").join("themes");
    fs::create_dir_all(&user_themes).unwrap();
    fs::write(
        user_themes.join("default.toml"),
        overlay_theme("default", "1"),
    )
    .unwrap();

    let manager = ThemeManager::new("default").unwrap();
    let builtin_icon = prompt_symbol(&manager);
    assert_eq!(
        manager.chain_for_test().to_string(),
        "default (user) → default (builtin)"
    );
    env.set("GPY_THEME_WATCH_POLL_MS", "25");
    manager.start_watching(Duration::from_millis(50)).unwrap();

    let plugin_dir = install_acme_theme(temp_dir.path(), "default", &icon_theme("P"));
    let installed = wait_until(Duration::from_secs(10), Duration::from_millis(100), || {
        prompt_symbol(&manager) == "P"
    });
    let installed_chain = manager.chain_for_test().to_string();
    // Uninstall by removing its files, not its watched directories, which a
    // Windows watch can keep from being deleted.
    fs::remove_file(plugin_dir.join("themes").join("default.toml")).unwrap();
    fs::remove_file(plugin_dir.join("plugin.toml")).unwrap();
    let removed = wait_until(Duration::from_secs(10), Duration::from_millis(100), || {
        prompt_symbol(&manager) == builtin_icon
    });
    let removed_chain = manager.chain_for_test().to_string();
    manager.stop_watching();

    assert!(installed, "a plugin providing the base must reload");
    assert_eq!(installed_chain, "default (user) → default (plugin:acme)");
    assert!(removed, "removing the plugin must fall back to the builtin");
    assert_eq!(removed_chain, "default (user) → default (builtin)");
    assert_eq!(
        root_prompt_icon(&manager),
        "1",
        "the overlay is merged over every base"
    );
}
