//! Unit tests for IPC server construction and guard behavior.

use super::EndpointHandle;
use crate::config::manager::ConfigManager;
#[cfg(unix)]
use crate::ipc::ClientDirectory;
use crate::ipc::fixtures::{endpoint_deps, render_deps};
use crate::ipc::{Message, Response, Segment};
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;

fn create_git_repo() -> TempDir {
    let dir = TempDir::new().expect("create temp git repo");
    std::process::Command::new("git")
        .arg("init")
        .current_dir(dir.path())
        .output()
        .expect("git init");
    dir
}

fn create_language_project() -> TempDir {
    let dir = TempDir::new().expect("create temp project");
    std::fs::write(dir.path().join("package.json"), "{\"name\":\"demo\"}")
        .expect("write package.json");
    dir
}

/// A manager over `config.toml` in `dir`, which holds `toml`.
fn config_manager_with(dir: &Path, toml: &str) -> Arc<ConfigManager> {
    let path = dir.join("config.toml");
    std::fs::write(&path, toml).expect("write config");
    Arc::new(ConfigManager::from_path(&path).expect("config"))
}

/// Rewrite `config.toml` in `dir` and reload it into `manager`.
fn reload_config(manager: &ConfigManager, dir: &Path, toml: &str) {
    std::fs::write(dir.join("config.toml"), toml).expect("write config");
    manager.reload_now().expect("reload config");
}

#[cfg(unix)]
#[test]
fn notify_clients_respects_agent_live_updates_flag() {
    use super::handle::notify_clients_for_response;
    use std::sync::atomic::Ordering;

    let (directory, rings) = ClientDirectory::counting_rings();
    let registry = directory.shared();
    let config_dir = TempDir::new().expect("config dir");
    let config_manager = config_manager_with(config_dir.path(), "[agent]\nlive_updates = false\n");

    let response = Response::RepositoryStatus(crate::git::RepositoryStatus {
        branch: "main".to_owned(),
        ahead: 0,
        behind: 0,
        ahead_capped: false,
        behind_capped: false,
        staged: 0,
        unstaged: 0,
        untracked: 0,
        conflicts: 0,
        state: crate::git::RepositoryState::Clean,
        stash_count: 0,
        detached: false,
        rebase_progress: None,
    });

    notify_clients_for_response(&config_manager, &registry, &response, None);
    assert_eq!(
        rings.load(Ordering::SeqCst),
        0,
        "live updates disabled should suppress notifications"
    );

    reload_config(
        &config_manager,
        config_dir.path(),
        "[agent]\nlive_updates = true\n",
    );

    notify_clients_for_response(&config_manager, &registry, &response, None);
    assert_eq!(
        rings.load(Ordering::SeqCst),
        1,
        "live updates enabled should allow notifications"
    );
}

#[test]
fn hot_reload_git_enabled_blocks_requests() {
    let config_dir = TempDir::new().expect("config dir");
    let config_manager = config_manager_with(config_dir.path(), "[git]\nenabled = true\n");
    let handle = EndpointHandle::new(endpoint_deps(
        Path::new("/tmp/hot-reload-git.sock"),
        render_deps(Arc::clone(&config_manager)),
    ));

    let repo = create_git_repo();
    let path_str = repo
        .path()
        .to_str()
        .expect("test repository path should be valid UTF-8");
    let path = crate::security::SafePath::new(path_str).expect("Valid test path");

    let response = handle
        .handler_registry
        .route(&git_status_message(path.clone()));
    let Ok(Response::RepositoryStatus(status)) = &response else {
        panic!("expected a RepositoryStatus for an enabled git segment, got {response:?}");
    };
    // A freshly `git init`ed repo has nothing pending.
    assert_eq!(status.state, crate::git::RepositoryState::Clean);

    reload_config(
        &config_manager,
        config_dir.path(),
        "[git]\nenabled = false\n",
    );

    assert!(matches!(
        handle.handler_registry.route(&git_status_message(path)),
        Err(ref err) if err.to_string().contains("Git segment disabled")
    ));
}

/// A minimal mid-chain `RepositoryStatus` request.
fn git_status_message(path: crate::security::SafePath) -> Message {
    Message::RepositoryStatus {
        path,
        segment: Segment::default(),
    }
}

#[allow(clippy::missing_panics_doc)]
fn build_test_endpoint(
    sock_name: &str,
    config_dir: &Path,
) -> (
    Arc<ConfigManager>,
    EndpointHandle,
    crate::security::SafePath,
    TempDir,
) {
    let config_manager = config_manager_with(config_dir, "[language]\nenabled = true\n");
    let handle = EndpointHandle::new(endpoint_deps(
        Path::new(sock_name),
        render_deps(Arc::clone(&config_manager)),
    ));
    let project = create_language_project();
    let path_str = project.path().to_str().expect("valid UTF-8");
    let path = crate::security::SafePath::new(path_str).expect("Valid test path");
    (config_manager, handle, path, project)
}

#[test]
fn hot_reload_language_enabled_updates_responses() {
    let config_dir = TempDir::new().expect("config dir");
    let (config_manager, handle, path, _project) =
        build_test_endpoint("/tmp/hot-reload-lang.sock", config_dir.path());

    let lang_msg = Message::LanguageDetect {
        path: path.clone(),
        segment: Segment::default(),
        virtual_env: None,
    };
    assert!(matches!(
        handle.handler_registry.route(&lang_msg),
        Ok(Response::Language { languages }) if !languages.is_empty()
    ));

    reload_config(
        &config_manager,
        config_dir.path(),
        "[language]\nenabled = false\n",
    );

    let message = Message::LanguageDetect {
        path,
        segment: Segment::default(),
        virtual_env: None,
    };
    assert!(matches!(
        handle.handler_registry.route(&message),
        Ok(Response::Language { languages }) if languages.is_empty()
    ));
}
