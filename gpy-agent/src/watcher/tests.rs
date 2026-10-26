use super::registry::{
    is_common_gitdir_significant_relative, is_git_significant_relative, split_at_git_dir_modules,
};
use super::{FileEvent, PendingEvent, WatchRegistry, filter_ignored};
use std::path::{Path, PathBuf};

/// Classify against a fresh, empty registry.
fn classify_event(path: &Path) -> Option<FileEvent> {
    super::classify_event(path, &WatchRegistry::new())
}

/// As [`classify_event`], through the full filter+classify pipeline.
fn should_trigger_update(path: &Path) -> Option<PendingEvent> {
    super::should_trigger_update(path, &WatchRegistry::new())
}

#[test]
fn python_version_file_is_language_event() {
    let event = classify_event(Path::new("/tmp/project/.python-version"));
    assert!(matches!(event, Some(FileEvent::Language { .. })));
}

#[test]
fn tool_versions_file_is_language_event() {
    let event = classify_event(Path::new("/tmp/project/.tool-versions"));
    assert!(matches!(event, Some(FileEvent::Language { .. })));
}

#[test]
fn git_info_exclude_is_git_event() {
    let event = classify_event(Path::new("/tmp/project/.git/info/exclude"));
    assert!(
        matches!(event, Some(FileEvent::Git { .. })),
        "expected a Git event, got {event:?}"
    );
}

#[test]
fn git_info_exclude_triggers_update() {
    let pending = should_trigger_update(Path::new("/tmp/project/.git/info/exclude"));
    assert!(
        matches!(pending.map(|p| p.event), Some(FileEvent::Git { .. })),
        "should_trigger_update must classify .git/info/exclude as a Git event"
    );
}

/// `git gc` / `git pack-refs` move ref updates into `.git/packed-refs`.
#[test]
fn packed_refs_is_git_event() {
    let event = classify_event(Path::new("/tmp/project/.git/packed-refs"));
    assert!(
        matches!(event, Some(FileEvent::Git { .. })),
        "expected a Git event, got {event:?}"
    );
}

#[test]
fn refs_stash_is_git_event() {
    let event = classify_event(Path::new("/tmp/project/.git/refs/stash"));
    assert!(
        matches!(event, Some(FileEvent::Git { .. })),
        "expected a Git event, got {event:?}"
    );
}

/// Set by merge/rebase/reset before they rewrite HEAD.
#[test]
fn orig_head_is_git_event() {
    let event = classify_event(Path::new("/tmp/project/.git/ORIG_HEAD"));
    assert!(
        matches!(event, Some(FileEvent::Git { .. })),
        "expected a Git event, got {event:?}"
    );
}

#[test]
fn refs_stash_prefix_lookalike_is_not_a_git_event() {
    let event = classify_event(Path::new("/tmp/project/.git/refs/stash-backup/main"));
    assert!(event.is_none(), "expected no event, got {event:?}");
}

/// Loose objects and reflogs are filtered before classification.
#[test]
fn filtered_git_dirs_still_produce_no_event() {
    let ignored_paths = [
        "/tmp/project/.git/objects/ab/cdef1234",
        "/tmp/project/.git/logs/HEAD",
        "/tmp/project/.git/logs/refs/heads/main",
        "/tmp/project/.git/hooks/pre-commit",
        "/tmp/project/.git/refs/tags/v1.0.0",
    ];

    for path in ignored_paths {
        let pending = should_trigger_update(Path::new(path));
        assert!(
            pending.is_none(),
            "expected no event for ignored path {path}, got {pending:?}"
        );
    }
}

#[test]
fn git_significant_relative_predicate() {
    let significant = [
        "HEAD",
        "index",
        "config",
        "ORIG_HEAD",
        "MERGE_HEAD",
        "FETCH_HEAD",
        "packed-refs",
        "refs/heads/main",
        "refs/remotes/origin/main",
        "refs/stash",
        "info/exclude",
    ];
    for rel in significant {
        assert!(
            is_git_significant_relative(Path::new(rel)),
            "{rel} should be git-significant"
        );
    }

    let insignificant = [
        "objects/ab/cdef",
        "logs/HEAD",
        "logs/refs/heads/main",
        "hooks/pre-commit",
        "refs/tags/v1.0.0",
        "refs/stash-backup/main",
        "description",
    ];
    for rel in insignificant {
        assert!(
            !is_git_significant_relative(Path::new(rel)),
            "{rel} must not be git-significant"
        );
    }
}

/// Only rebase dirs and the files status readers use; per-step files stay rejected.
#[test]
fn rebase_metadata_significant_relative_predicate() {
    let significant = [
        "rebase-merge",
        "rebase-merge/msgnum",
        "rebase-merge/end",
        "rebase-merge/git-rebase-todo",
        "rebase-apply",
        "rebase-apply/applying",
    ];
    for rel in significant {
        assert!(
            is_git_significant_relative(Path::new(rel)),
            "{rel} should be git-significant"
        );
    }

    // Listed explicitly so accepting the whole directory fails loudly.
    let insignificant = [
        "rebase-merge/patch",
        "rebase-merge/git-rebase-todo.backup",
        "rebase-merge/message",
        "rebase-merge/stopped-sha",
        "rebase-merge/done",
        "rebase-merge/head-name",
        "rebase-merge/onto",
        "rebase-apply/next",
        "rebase-apply/patch",
        "rebase-apply/last",
        "rebase-apply/msg",
    ];
    for rel in insignificant {
        assert!(
            !is_git_significant_relative(Path::new(rel)),
            "{rel} must not be git-significant"
        );
    }
}

#[test]
fn sequencer_and_reftable_significant_relative_predicate() {
    for rel in ["sequencer", "sequencer/todo", "reftable/tables.list"] {
        assert!(
            is_git_significant_relative(Path::new(rel)),
            "{rel} should be git-significant"
        );
    }
    for rel in [
        "sequencer/head",
        "sequencer/abort-safety",
        "reftable/0x1-0x2-ab.ref",
    ] {
        assert!(
            !is_git_significant_relative(Path::new(rel)),
            "{rel} must not be git-significant"
        );
    }
}

/// Rebase progress files reach `Git` events through the full pipeline.
#[test]
fn rebase_progress_files_trigger_update() {
    for path in [
        "/tmp/project/.git/rebase-merge/msgnum",
        "/tmp/project/.git/rebase-merge/end",
        "/tmp/project/.git/rebase-apply/applying",
    ] {
        let pending = should_trigger_update(Path::new(path));
        assert!(
            matches!(pending.map(|p| p.event), Some(FileEvent::Git { .. })),
            "should_trigger_update must classify {path} as a Git event"
        );
    }
}

/// A new directory's notify event carries the directory path itself.
#[test]
fn rebase_operation_directory_entry_is_git_event() {
    for path in [
        "/tmp/project/.git/rebase-merge",
        "/tmp/project/.git/rebase-apply",
    ] {
        let event = classify_event(Path::new(path));
        assert!(
            matches!(event, Some(FileEvent::Git { .. })),
            "expected a Git event for the {path} directory entry, got {event:?}"
        );
    }
}

/// Classification is the only gate: `filter_ignored` doesn't block rebase dirs.
#[test]
fn rebase_high_churn_files_produce_no_event() {
    for path in [
        "/tmp/project/.git/rebase-merge/patch",
        "/tmp/project/.git/rebase-merge/git-rebase-todo.backup",
        "/tmp/project/.git/rebase-merge/message",
        "/tmp/project/.git/rebase-merge/stopped-sha",
        "/tmp/project/.git/rebase-apply/next",
        "/tmp/project/.git/rebase-apply/patch",
    ] {
        let pending = should_trigger_update(Path::new(path));
        assert!(
            pending.is_none(),
            "expected no event for high-churn rebase path {path}, got {pending:?}"
        );
    }
}

/// Rebase metadata in a submodule gitdir resolves to that gitdir; churn to none.
#[test]
fn submodule_rebase_metadata_resolves_to_owning_gitdir() {
    let registry = WatchRegistry::new();
    let git_dir = PathBuf::from("/super/.git/modules/sub");
    let work_root = PathBuf::from("/super/sub");
    registry.register_external_gitdir(&git_dir, &work_root);

    for rel in [
        "rebase-merge",
        "rebase-merge/msgnum",
        "rebase-merge/end",
        "rebase-apply",
        "rebase-apply/applying",
    ] {
        let path = git_dir.join(rel);
        assert_eq!(
            registry.attribute_external_gitdir(&path),
            Some(work_root.clone()),
            "{} should attribute to the submodule's own working root",
            path.display()
        );
        assert_eq!(
            registry.attribute_submodule_parents(&path),
            vec![PathBuf::from("/super")],
            "{} should also fan out to the registered superproject",
            path.display()
        );
    }

    for rel in ["rebase-merge/patch", "rebase-apply/next"] {
        let path = git_dir.join(rel);
        assert_eq!(
            registry.attribute_external_gitdir(&path),
            None,
            "{} is high-churn and must not attribute",
            path.display()
        );
        assert!(
            registry.attribute_submodule_parents(&path).is_empty(),
            "{} is high-churn and must not fan out to the superproject",
            path.display()
        );
    }

    registry.unregister_external_gitdir(&git_dir);
}

#[test]
fn external_gitdir_attribution_maps_to_working_root() {
    let registry = WatchRegistry::new();
    let git_dir = PathBuf::from("/super/.git/modules/sub");
    let work_root = PathBuf::from("/super/sub");
    registry.register_external_gitdir(&git_dir, &work_root);

    assert_eq!(
        registry.attribute_external_gitdir(Path::new("/super/.git/modules/sub/HEAD")),
        Some(work_root.clone())
    );
    assert_eq!(
        registry.attribute_external_gitdir(Path::new("/super/.git/modules/sub/refs/heads/other")),
        Some(work_root)
    );
    assert_eq!(
        registry.attribute_external_gitdir(Path::new("/super/.git/modules/sub/objects/ab/cd")),
        None
    );
    assert_eq!(
        registry.attribute_external_gitdir(Path::new("/elsewhere/.git/HEAD")),
        None
    );

    registry.unregister_external_gitdir(&git_dir);
    assert_eq!(
        registry.attribute_external_gitdir(Path::new("/super/.git/modules/sub/HEAD")),
        None,
        "unregister must remove the mapping"
    );
}

/// `HEAD`/`index` are per-worktree, so they must not fan out from the common dir.
#[test]
fn common_gitdir_significance_covers_shared_state_only() {
    let shared = [
        "refs/heads/main",
        "refs/heads/feature/nested",
        "refs/remotes/origin/main",
        "packed-refs",
        "config",
    ];
    for rel in shared {
        assert!(
            is_common_gitdir_significant_relative(Path::new(rel)),
            "{rel} is shared across worktrees and must fan out"
        );
    }

    let not_shared = [
        "HEAD",
        "index",
        "ORIG_HEAD",
        "MERGE_HEAD",
        "FETCH_HEAD",
        "info/exclude",
        "refs/stash",
        // Another worktree's admin dir must never fan out to siblings.
        "worktrees/other/HEAD",
        "worktrees/other/index",
        "worktrees/other/refs/bisect/bad",
        "objects/ab/cdef",
        "logs/HEAD",
        "refs/tags/v1.0.0",
        "description",
    ];
    for rel in not_shared {
        assert!(
            !is_common_gitdir_significant_relative(Path::new(rel)),
            "{rel} must not fan out to linked worktrees"
        );
    }
}

#[test]
fn common_gitdir_attribution_fans_out_to_every_dependent() {
    let registry = WatchRegistry::new();
    let common = PathBuf::from("/main/.git");
    let first = PathBuf::from("/wt/one");
    let second = PathBuf::from("/wt/two");
    registry.register_common_gitdir(&common, &first);
    registry.register_common_gitdir(&common, &second);

    let mut fanned =
        registry.attribute_common_gitdir(Path::new("/main/.git/refs/remotes/origin/main"));
    fanned.sort();
    assert_eq!(fanned, vec![first.clone(), second.clone()]);

    assert_eq!(
        registry
            .attribute_common_gitdir(Path::new("/main/.git/config"))
            .len(),
        2,
        "a shared config change must reach every dependent worktree"
    );
    assert!(
        registry
            .attribute_common_gitdir(Path::new("/main/.git/index"))
            .is_empty(),
        "a main-checkout index write must wake zero linked worktrees"
    );
    assert!(
        registry
            .attribute_common_gitdir(Path::new("/main/.git/worktrees/one/HEAD"))
            .is_empty(),
        "a per-worktree HEAD write must not fan out to siblings"
    );
    assert!(
        registry
            .attribute_common_gitdir(Path::new("/elsewhere/.git/config"))
            .is_empty(),
        "an unregistered common dir matches nothing"
    );

    // Longest prefix wins for a submodule's nested common dir.
    let nested = PathBuf::from("/main/.git/modules/sub");
    let nested_root = PathBuf::from("/wt/sub");
    registry.register_common_gitdir(&nested, &nested_root);
    assert_eq!(
        registry.attribute_common_gitdir(Path::new("/main/.git/modules/sub/refs/heads/main")),
        vec![nested_root.clone()],
        "the nested common dir must win over the superproject's"
    );
    registry.unregister_common_gitdir(&nested, &nested_root);

    // Reference counted: survives the first dependent leaving.
    registry.unregister_common_gitdir(&common, &first);
    assert_eq!(
        registry.attribute_common_gitdir(Path::new("/main/.git/packed-refs")),
        vec![second.clone()],
        "the remaining dependent must keep receiving shared metadata"
    );

    registry.unregister_common_gitdir(&common, &second);
    assert!(
        registry
            .attribute_common_gitdir(Path::new("/main/.git/packed-refs"))
            .is_empty(),
        "the entry must be dropped once the last dependent unregisters"
    );
}

/// Nested repos and linked-worktree admin dirs hold no gitlink and never fire.
#[test]
fn submodule_parents_derive_the_superproject_from_the_gitdir() {
    let registry = WatchRegistry::new();
    assert_eq!(
        registry.attribute_submodule_parents(Path::new("/super/.git/modules/sub/HEAD")),
        vec![PathBuf::from("/super")]
    );
    // The gitdir is named after the submodule name, which may contain `/`.
    assert_eq!(
        registry.attribute_submodule_parents(Path::new("/super/.git/modules/libs/foo/index")),
        vec![PathBuf::from("/super")]
    );
    assert_eq!(
        registry.attribute_submodule_parents(Path::new("/super/.git/modules/sub/refs/heads/topic")),
        vec![PathBuf::from("/super")]
    );

    for internal in [
        "/super/.git/modules/sub/objects/ab/cdef",
        "/super/.git/modules/sub/refs/tags/v1.0.0",
        "/super/.git/modules/sub/hooks/pre-commit",
        "/super/.git/modules/sub/description",
    ] {
        assert!(
            registry
                .attribute_submodule_parents(Path::new(internal))
                .is_empty(),
            "{internal} cannot move the superproject's gitlink"
        );
    }

    for not_a_submodule in [
        "/outer/nested/.git/HEAD",
        // Linked worktree admin dir: the main checkout holds no gitlink to it.
        "/main/.git/worktrees/wt/HEAD",
        "/super/sub/src/main.rs",
    ] {
        assert!(
            registry
                .attribute_submodule_parents(Path::new(not_a_submodule))
                .is_empty(),
            "{not_a_submodule} has no parent gitlink to refresh"
        );
    }
}

/// A nested submodule moves both the grandparent's and the intermediate's gitlink.
#[test]
fn nested_submodule_parents_include_the_intermediate_working_root() {
    let registry = WatchRegistry::new();
    let inner_gitdir = PathBuf::from("/super/.git/modules/outer/modules/inner");
    let outer_gitdir = PathBuf::from("/super/.git/modules/outer");
    let outer_root = PathBuf::from("/super/outer");
    let inner_root = PathBuf::from("/super/outer/inner");

    assert_eq!(
        registry.attribute_submodule_parents(&inner_gitdir.join("HEAD")),
        vec![PathBuf::from("/super")]
    );

    registry.register_external_gitdir(&outer_gitdir, &outer_root);
    registry.register_external_gitdir(&inner_gitdir, &inner_root);

    let mut parents = registry.attribute_submodule_parents(&inner_gitdir.join("HEAD"));
    parents.sort();
    assert_eq!(
        parents,
        vec![PathBuf::from("/super"), outer_root],
        "both the grandparent and the intermediate submodule hold a moved gitlink"
    );

    // The changed submodule itself is delivered by the external-gitdir arm.
    assert!(
        !registry
            .attribute_submodule_parents(&inner_gitdir.join("HEAD"))
            .contains(&inner_root),
        "the changed submodule must not be fanned out to as a parent"
    );

    registry.unregister_external_gitdir(&inner_gitdir);
    registry.unregister_external_gitdir(&outer_gitdir);
}

/// The hot-path lookup does no resolution: nonexistent registered paths match
/// exactly and unregistered ones don't.
#[test]
fn config_path_lookup_is_pure_no_resolution() {
    let registry = WatchRegistry::new();
    // `/tmp/...` would be relative to the drive on Windows.
    let nonexistent = std::env::temp_dir().join("gpy-agent-602-does-not-exist/gpy.toml");
    registry.register_config_path(&nonexistent);

    assert!(
        registry.is_registered_config_path(&nonexistent),
        "a registered path must be found by exact lookup, with no \
         canonicalize/current_dir call required for a file that doesn't exist"
    );

    let unregistered = std::env::temp_dir().join("gpy-agent-602-also-does-not-exist/other.toml");
    assert!(
        !registry.is_registered_config_path(&unregistered),
        "lookup must be exact HashSet::contains, matching only what was registered"
    );

    registry.unregister_config_path(&nonexistent);
    assert!(
        !registry.is_registered_config_path(&nonexistent),
        "unregister must remove the exact entry"
    );
}

#[test]
fn socket_lookalike_filename_is_not_filtered() {
    assert!(
        filter_ignored(Path::new("/tmp/project/src/foo.socket.rs")).is_some(),
        "foo.socket.rs must NOT be filtered: `.sock` is a substring of \
         its name, not its extension"
    );
}

#[test]
fn real_sock_file_is_still_filtered() {
    assert!(
        filter_ignored(Path::new("/tmp/gpy.sock")).is_none(),
        "the agent's gpy.sock socket must be filtered by its .sock extension"
    );
    assert!(
        filter_ignored(Path::new("/tmp/project/other.sock")).is_none(),
        "any other *.sock file must still be filtered"
    );
}

#[test]
fn gpy_agent_log_filename_is_still_filtered() {
    assert!(filter_ignored(Path::new("/var/log/gpy-agent.log")).is_none());
}

/// Built with the platform separator; a literal `C:\...` is one component on Unix.
#[test]
fn classify_event_recognizes_platform_separated_paths() {
    let git_objects = PathBuf::from_iter([
        "C:", "Users", "dev", "project", ".git", "objects", "ab", "cdef1234",
    ]);
    assert!(
        filter_ignored(&git_objects).is_none(),
        "a .git/objects churn path must still be filtered regardless of platform separator"
    );

    let git_head = PathBuf::from_iter(["C:", "Users", "dev", "project", ".git", "HEAD"]);
    assert!(
        matches!(classify_event(&git_head), Some(FileEvent::Git { .. })),
        "a .git/HEAD path must still classify as a Git event regardless of platform separator"
    );

    let themes_dir = PathBuf::from_iter([
        "C:",
        "Users",
        "dev",
        ".config",
        "gpy",
        "themes",
        "default.toml",
    ]);
    assert!(
        matches!(classify_event(&themes_dir), Some(FileEvent::Theme { .. })),
        "a gpy/themes path must still classify as a Theme event regardless of platform separator"
    );
}

#[test]
fn git_metadata_under_gpy_themes_classifies_as_git() {
    for leaf in ["HEAD", "index", "packed-refs", "refs/heads/main"] {
        let path = Path::new("/x/gpy/themes/.git").join(leaf);
        assert!(
            matches!(classify_event(&path), Some(FileEvent::Git { .. })),
            "{} must classify as Git",
            path.display()
        );
    }
    assert!(
        matches!(
            classify_event(Path::new("/x/gpy/themes/mine.toml")),
            Some(FileEvent::Theme { .. })
        ),
        "a plain theme file must still classify as Theme"
    );
}

#[test]
fn metadata_marker_files_trigger_language_event() {
    for file_name in ["composer.json", "CMakeLists.txt", "tsconfig.json"] {
        let path = Path::new("/tmp/project").join(file_name);
        assert!(
            matches!(classify_event(&path), Some(FileEvent::Language { .. })),
            "{file_name} should classify as a Language event (metadata.rs marker file)"
        );
    }
}

#[test]
fn residual_lockfiles_still_trigger_language_event() {
    for file_name in [
        "package-lock.json",
        "Cargo.lock",
        "poetry.lock",
        "rust-toolchain",
        "go.work",
    ] {
        let path = Path::new("/tmp/project").join(file_name);
        assert!(
            matches!(classify_event(&path), Some(FileEvent::Language { .. })),
            "{file_name} must keep triggering a Language event (residual lockfile list)"
        );
    }
}

#[test]
fn version_pin_files_trigger_language_event() {
    for file_name in [
        ".node-version",
        ".nvmrc",
        "mise.toml",
        ".mise.toml",
        "mise.local.toml",
        ".mise.local.toml",
    ] {
        let path = Path::new("/tmp/project").join(file_name);
        assert!(
            matches!(classify_event(&path), Some(FileEvent::Language { .. })),
            "{file_name} must trigger a Language event (version pin)"
        );
    }
}

#[test]
fn split_at_git_dir_modules_uses_first_occurrence_for_nested_submodules() {
    let nested = Path::new("/super/.git/modules/outer/modules/inner/HEAD");
    let (super_root, modules_root) =
        split_at_git_dir_modules(nested).expect("nested submodule path must split");
    assert_eq!(super_root, PathBuf::from("/super"));
    assert_eq!(modules_root, PathBuf::from("/super/.git/modules"));
}

#[test]
fn split_at_git_dir_modules_single_level() {
    let path = Path::new("/super/.git/modules/sub/HEAD");
    let (super_root, modules_root) =
        split_at_git_dir_modules(path).expect("submodule path must split");
    assert_eq!(super_root, PathBuf::from("/super"));
    assert_eq!(modules_root, PathBuf::from("/super/.git/modules"));
}

#[test]
fn split_at_git_dir_modules_rejects_non_submodule_paths() {
    assert!(split_at_git_dir_modules(Path::new("/outer/nested/.git/HEAD")).is_none());
    assert!(split_at_git_dir_modules(Path::new("/main/.git/worktrees/wt/HEAD")).is_none());
}
