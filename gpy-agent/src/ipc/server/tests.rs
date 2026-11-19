//! Unit tests for IPC server construction and guard behavior.

use super::EndpointHandle;
use crate::config::manager::ConfigManager;
use crate::ipc::fixtures::{endpoint_deps, render_deps};
use crate::ipc::{Message, Response, Segment};
use std::assert_matches;
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

#[test]
fn hot_reload_git_segment_blocks_requests() {
    let config_dir = TempDir::new().expect("config dir");
    let config_manager =
        config_manager_with(config_dir.path(), "[ui]\nenabled_segments = [\"git\"]\n");
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
        "[ui]\nenabled_segments = [\"directory\"]\n",
    );

    assert_matches!(
        handle.handler_registry.route(&git_status_message(path)),
        Err(ref err) if err.to_string().contains("Git segment disabled")
    );
}

/// A minimal mid-chain `RepositoryStatus` request.
fn git_status_message(path: crate::security::SafePath) -> Message {
    Message::RepositoryStatus {
        path,
        segment: Segment::default(),
    }
}

fn build_test_endpoint(
    sock_name: &str,
    config_dir: &Path,
) -> (
    Arc<ConfigManager>,
    EndpointHandle,
    crate::security::SafePath,
    TempDir,
) {
    // No version probe: it runs the language's own binary within the request
    // budget, which a slow runner can miss (jpease/gpy-archive#1245). The test
    // is about the segment toggle, so detection alone is enough.
    let config_manager = config_manager_with(
        config_dir,
        "[ui]\nenabled_segments = [\"language\"]\n[language]\nshow_versions = false\n",
    );
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
fn hot_reload_language_segment_updates_responses() {
    let config_dir = TempDir::new().expect("config dir");
    let (config_manager, handle, path, _project) =
        build_test_endpoint("/tmp/hot-reload-lang.sock", config_dir.path());

    let lang_msg = Message::LanguageDetect {
        path: path.clone(),
        segment: Segment::default(),
        virtual_env: None,
    };
    assert_matches!(
        handle.handler_registry.route(&lang_msg),
        Ok(Response::Language { languages }) if !languages.is_empty()
    );

    reload_config(
        &config_manager,
        config_dir.path(),
        "[ui]\nenabled_segments = [\"directory\"]\n",
    );

    let message = Message::LanguageDetect {
        path,
        segment: Segment::default(),
        virtual_env: None,
    };
    assert_matches!(
        handle.handler_registry.route(&message),
        Ok(Response::Language { languages }) if languages.is_empty()
    );
}
