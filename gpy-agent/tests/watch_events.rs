use gpy_agent::watcher::{WatchRegistry, should_trigger_update};
use std::path::Path;

#[test]
/// # Panics
///
/// Panics if `should_trigger_update` does not return `None` for a non-Git path.
fn should_trigger_update_ignores_non_git_paths() {
    let path = Path::new("/tmp/file.txt");
    assert!(should_trigger_update(path, &WatchRegistry::new()).is_none());
}
