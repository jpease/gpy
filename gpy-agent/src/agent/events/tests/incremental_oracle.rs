use super::*;

/// `path` as a `git` URL argument; `git` rejects Windows `\\?\` paths.
fn git_url(path: &Path) -> String {
    let text = path.to_str().expect("utf-8 path");
    text.strip_prefix(r"\\?\")
        .filter(|plain| !plain.starts_with("UNC\\"))
        .unwrap_or(text)
        .to_owned()
}

/// Oracle per-path flags: (staged, unstaged, untracked, conflicted).
type OracleFlags = (bool, bool, bool, bool);

/// Per-path status parsed from git independently of `git::native::parser`.
fn oracle_files(repo: &Path) -> HashMap<PathBuf, FileStatus> {
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain=v2", "-z"])
        .current_dir(repo)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("oracle git status");
    assert!(output.status.success(), "oracle git status failed");
    let text = String::from_utf8(output.stdout).expect("utf-8 status");

    let mut files: HashMap<String, OracleFlags> = HashMap::new();
    let mut records = text.split('\0');
    while let Some(record) = records.next() {
        let (path, flags) = match record.chars().next() {
            Some('1') => {
                let mut fields = record.splitn(9, ' ');
                let xy = fields.nth(1).unwrap_or_default();
                let path = fields.nth(6).unwrap_or_default();
                (path, xy_flags(xy))
            }
            Some('2') => {
                let mut fields = record.splitn(10, ' ');
                let xy = fields.nth(1).unwrap_or_default();
                let path = fields.nth(7).unwrap_or_default();
                records.next();
                (path, xy_flags(xy))
            }
            Some('u') => {
                let path = record.splitn(11, ' ').nth(10).unwrap_or_default();
                (path, (false, false, false, true))
            }
            Some('?') => (
                record.get(2..).unwrap_or_default(),
                (false, false, true, false),
            ),
            _ => continue,
        };
        let entry = files.entry(path.to_owned()).or_default();
        *entry = (
            entry.0 || flags.0,
            entry.1 || flags.1,
            entry.2 || flags.2,
            entry.3 || flags.3,
        );
    }

    files
        .into_iter()
        .map(|(path, (staged, unstaged, untracked, conflicted))| {
            (
                PathBuf::from(path),
                FileStatus {
                    staged,
                    unstaged,
                    untracked,
                    conflicted,
                },
            )
        })
        .collect()
}

fn xy_flags(xy: &str) -> OracleFlags {
    let mut chars = xy.chars();
    let staged = chars.next().is_some_and(|c| c != '.');
    let unstaged = chars.next().is_some_and(|c| c != '.');
    (staged, unstaged, false, false)
}

/// xorshift64: deterministic, so a failing seed replays.
struct Rng(u64);

impl Rng {
    fn below(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13_u32;
        self.0 ^= self.0 >> 7_u32;
        self.0 ^= self.0 << 17_u32;
        let bound64 = u64::try_from(bound).expect("small bound");
        usize::try_from(self.0.checked_rem(bound64).expect("non-zero bound")).expect("fits usize")
    }
}

/// Names that are easy to mishandle: TAB, leading `:` (pathspec magic), spaces, new dirs.
/// Windows forbids TAB and `:`.
#[cfg(not(windows))]
const ORACLE_NAMES: [&str; 8] = [
    "a.txt",
    "b c.txt",
    "tab\tname.txt",
    ":colon.txt",
    "src/lib.txt",
    "newdir/x.txt",
    "newdir/deep/y.txt",
    "other/z.txt",
];

#[cfg(windows)]
const ORACLE_NAMES: [&str; 6] = [
    "a.txt",
    "b c.txt",
    "src/lib.txt",
    "newdir/x.txt",
    "newdir/deep/y.txt",
    "other/z.txt",
];

/// Applies one random mutation; returns watcher-reported paths and a log line.
/// Reuses the previous file half the time so per-path op chains are common.
fn apply_random_op(rng: &mut Rng, repo: &Path, previous: &mut usize) -> (Vec<PathBuf>, String) {
    if rng.below(2) == 0 {
        *previous = rng.below(ORACLE_NAMES.len());
    }
    let name = ORACLE_NAMES.get(*previous).copied().unwrap_or("a.txt");
    let path = repo.join(name);
    let index = vec![repo.join(".git/index")];
    match rng.below(6) {
        0 | 1 => {
            // A watcher reports each created directory plus the file.
            let mut reported = Vec::new();
            let mut dir = path.parent();
            while let Some(d) = dir {
                if d == repo || d.exists() {
                    break;
                }
                reported.push(d.to_path_buf());
                dir = d.parent();
            }
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent");
            }
            let body = format!("{}\n", rng.below(1000));
            fs::write(&path, body).expect("write file");
            reported.push(path);
            (reported, format!("write {name:?}"))
        }
        2 => {
            let _ = fs::remove_file(&path);
            // git stops reporting empty dirs; delete them like an editor would.
            let mut reported = vec![path.clone()];
            let mut dir = path.parent();
            while let Some(d) = dir {
                if d == repo || fs::remove_dir(d).is_err() {
                    break;
                }
                reported.push(d.to_path_buf());
                dir = d.parent();
            }
            (reported, format!("rm {name:?}"))
        }
        3 => {
            git_in(repo, &["add", "--", &format!(":(literal){name}")]);
            (index, format!("git add {name:?}"))
        }
        4 => {
            git_in(
                repo,
                &["rm", "-q", "--cached", "--", &format!(":(literal){name}")],
            );
            (index, format!("git rm --cached {name:?}"))
        }
        _ => {
            git_in(repo, &["commit", "-q", "-m", "step", "--allow-empty"]);
            (index, "git commit".to_owned())
        }
    }
}

#[test]
/// Incremental refresh must equal an independent full `git status` after each random op.
fn incremental_refresh_matches_full_git_status_oracle() {
    for seed in 1..=24_u64 {
        let (_tmp, repo) = create_temp_repo();
        fs::create_dir_all(repo.join("src")).expect("src dir");
        fs::write(repo.join("a.txt"), "a\n").expect("seed a.txt");
        fs::write(repo.join("src/lib.txt"), "lib\n").expect("seed lib");
        commit_all(&repo);

        let ctx = make_agent_context();
        let config = Config::default();
        let backend = NativeGitBackend;
        refresh_repo_status_with(&backend, &ctx, &repo, &config, None);

        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut log = Vec::new();
        let mut previous = 0_usize;
        for _ in 0_u32..16_u32 {
            let (reported, op) = apply_random_op(&mut rng, &repo, &mut previous);
            log.push(op);
            let hint = git_paths_hint(&GitPaths::Paths(reported), &repo);
            let refreshed =
                refresh_repo_status_with(&backend, &ctx, &repo, &config, hint.as_deref());
            let status = refreshed.status.expect("repository status");
            let want_files = oracle_files(&repo);
            let got_files = ctx.cache.files_for_test(&repo).unwrap_or_default();
            assert_eq!(
                got_files, want_files,
                "seed {seed}: cached file map diverged from git after {log:#?}"
            );
            let got = StatusAggregate {
                staged: status.staged,
                unstaged: status.unstaged,
                untracked: status.untracked,
                conflicts: status.conflicts,
            };
            assert_eq!(
                got,
                StatusAggregate::from_file_statuses(want_files.values()),
                "seed {seed}: status counts diverged from git after {log:#?}"
            );
        }
    }
}

/// Parent repo with a committed submodule `sub`; the first `TempDir` must outlive use.
fn repo_with_submodule() -> (tempfile::TempDir, tempfile::TempDir, PathBuf) {
    let (source_tmp, source) = create_temp_repo();
    fs::write(source.join("s.txt"), "s\n").expect("write s.txt");
    commit_all(&source);

    let (parent_tmp, repo) = create_temp_repo();
    fs::write(repo.join("t.txt"), "t\n").expect("write t.txt");
    commit_all(&repo);
    let source_arg = git_url(&source);
    let added = std::process::Command::new("git")
        .args([
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            &source_arg,
            "sub",
        ])
        .current_dir(&repo)
        .output()
        .expect("spawn git submodule add");
    assert!(
        added.status.success(),
        "git submodule add failed: {}",
        String::from_utf8_lossy(&added.stderr)
    );
    commit_all(&repo);
    let sub = repo.join("sub");
    assert!(git_in(&sub, &["config", "user.email", "test@example.com"]));
    assert!(git_in(&sub, &["config", "user.name", "Test"]));
    (source_tmp, parent_tmp, repo)
}

fn deliver_path(
    ctx: &AgentContext,
    repo: &Path,
    config: &Config,
    path: PathBuf,
) -> RepositoryStatus {
    let event = DebouncedEvent {
        event: FileEvent::Git {
            paths: GitPaths::single(path),
        },
        repo: repo.to_path_buf(),
    };
    handle_file_event(ctx, &event, config);
    ctx.cache.get(repo).expect("status after event")
}

#[test]
/// Submodule child paths match no superproject pathspec; map them to the gitlink.
fn incremental_refresh_maps_submodule_child_paths_to_gitlink() {
    let (_source, _tmp, repo) = repo_with_submodule();
    let ctx = make_agent_context();
    let config = Config::default();
    handle_file_event(&ctx, &whole_repo_event(&repo), &config);

    let file = repo.join("sub/s.txt");
    fs::write(&file, "s\nmodified\n").expect("modify submodule file");
    let incremental = deliver_path(&ctx, &repo, &config, file);
    assert_eq!(
        incremental.unstaged, 1,
        "the submodule must show as modified"
    );

    ctx.cache.invalidate(&repo);
    handle_file_event(&ctx, &whole_repo_event(&repo), &config);
    let full = ctx.cache.get(&repo).expect("status after full scan");
    assert_eq!(incremental, full);
}

#[test]
fn incremental_refresh_clears_gitlink_after_submodule_revert() {
    let (_source, _tmp, repo) = repo_with_submodule();
    let ctx = make_agent_context();
    let config = Config::default();

    let file = repo.join("sub/s.txt");
    fs::write(&file, "s\nmodified\n").expect("modify submodule file");
    handle_file_event(&ctx, &whole_repo_event(&repo), &config);
    assert_eq!(ctx.cache.get(&repo).expect("warm").unstaged, 1);

    assert!(
        git_in(&repo.join("sub"), &["checkout", "--", "s.txt"]),
        "git checkout failed"
    );
    let incremental = deliver_path(&ctx, &repo, &config, file);
    assert_eq!(incremental.state, RepositoryState::Clean);

    ctx.cache.invalidate(&repo);
    handle_file_event(&ctx, &whole_repo_event(&repo), &config);
    let full = ctx.cache.get(&repo).expect("status after full scan");
    assert_eq!(incremental, full);
}

#[test]
fn incremental_refresh_maps_nested_repo_child_paths_to_nested_root() {
    let (_tmp, repo) = create_temp_repo();
    fs::write(repo.join("t.txt"), "t\n").expect("write t.txt");
    commit_all(&repo);
    let ctx = make_agent_context();
    let config = Config::default();
    handle_file_event(&ctx, &whole_repo_event(&repo), &config);

    let child = repo.join("deep/child");
    fs::create_dir_all(&child).expect("create nested repo dir");
    assert!(git_in(&child, &["init", "-q"]), "git init failed");
    let file = child.join("f.txt");
    fs::write(&file, "f\n").expect("write nested file");
    let incremental = deliver_path(&ctx, &repo, &config, file);
    assert_eq!(incremental.untracked, 1);

    ctx.cache.invalidate(&repo);
    handle_file_event(&ctx, &whole_repo_event(&repo), &config);
    let full = ctx.cache.get(&repo).expect("status after full scan");
    assert_eq!(incremental, full);
}

#[test]
fn incremental_refresh_matches_git_status_for_submodule_edits() {
    for seed in 1..=8_u64 {
        let (_source, _tmp, repo) = repo_with_submodule();
        let sub = repo.join("sub");
        let ctx = make_agent_context();
        let config = Config::default();
        let backend = NativeGitBackend;
        refresh_repo_status_with(&backend, &ctx, &repo, &config, None);

        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut log = Vec::new();
        for _ in 0_u32..10_u32 {
            let (reported, op) = match rng.below(4) {
                0 => {
                    let file = sub.join("s.txt");
                    fs::write(&file, format!("{}\n", rng.below(1000))).expect("write");
                    (vec![file], "edit s.txt")
                }
                1 => {
                    git_in(&sub, &["checkout", "--", "s.txt"]);
                    (vec![sub.join("s.txt")], "revert s.txt")
                }
                2 => {
                    let file = sub.join("extra/new.txt");
                    fs::create_dir_all(sub.join("extra")).expect("mkdir");
                    fs::write(&file, "n\n").expect("write");
                    (vec![sub.join("extra"), file], "add untracked")
                }
                _ => {
                    git_in(&sub, &["commit", "-q", "-a", "-m", "step", "--allow-empty"]);
                    (vec![sub.join("s.txt")], "commit in submodule")
                }
            };
            log.push(op);
            let hint = git_paths_hint(&GitPaths::Paths(reported), &repo);
            let refreshed =
                refresh_repo_status_with(&backend, &ctx, &repo, &config, hint.as_deref());
            let status = refreshed.status.expect("repository status");
            let want_files = oracle_files(&repo);
            let got_files = ctx.cache.files_for_test(&repo).unwrap_or_default();
            assert_eq!(
                got_files, want_files,
                "seed {seed}: cached file map diverged from git after {log:#?}"
            );
            assert_eq!(
                StatusAggregate {
                    staged: status.staged,
                    unstaged: status.unstaged,
                    untracked: status.untracked,
                    conflicts: status.conflicts,
                },
                StatusAggregate::from_file_statuses(want_files.values()),
                "seed {seed}: status counts diverged from git after {log:#?}"
            );
        }
    }
}

/// Whether git set `core.ignorecase` for `repo`.
fn repo_ignores_case(repo: &Path) -> bool {
    let output = std::process::Command::new("git")
        .args(["config", "--type=bool", "--default", "false"])
        .arg("core.ignorecase")
        .current_dir(repo)
        .output()
        .expect("git config");
    String::from_utf8_lossy(&output.stdout).trim() == "true"
}

/// Case-only rename via a temp name so it works on every volume.
fn rename_via_temp(repo: &Path, from: &str, to: &str) {
    let temp = repo.join("case-rename.tmp");
    fs::rename(repo.join(from), &temp).expect("rename to temp");
    fs::rename(&temp, repo.join(to)).expect("rename from temp");
}

#[test]
/// Index spells `Readme.md`, disk `README.md`; the edit and revert must be seen.
fn incremental_refresh_handles_index_case_mismatch() {
    let (_tmp, repo) = create_temp_repo();
    if !repo_ignores_case(&repo) {
        return;
    }
    fs::write(repo.join("Readme.md"), "a\n").expect("write Readme.md");
    commit_all(&repo);
    rename_via_temp(&repo, "Readme.md", "README.md");

    let ctx = make_agent_context();
    let config = Config::default();
    handle_file_event(&ctx, &whole_repo_event(&repo), &config);
    assert_eq!(
        ctx.cache.get(&repo).expect("warm").state,
        RepositoryState::Clean
    );

    let file = repo.join("README.md");
    fs::write(&file, "b\n").expect("modify README.md");
    let edited = deliver_path(&ctx, &repo, &config, file.clone());
    assert_eq!(edited.unstaged, 1, "the edit must be seen incrementally");
    ctx.cache.invalidate(&repo);
    handle_file_event(&ctx, &whole_repo_event(&repo), &config);
    assert_eq!(edited, ctx.cache.get(&repo).expect("full scan"));

    fs::write(&file, "a\n").expect("restore README.md");
    let reverted = deliver_path(&ctx, &repo, &config, file);
    assert_eq!(
        reverted.state,
        RepositoryState::Clean,
        "the revert must clear the entry cached under the index spelling"
    );
    ctx.cache.invalidate(&repo);
    handle_file_event(&ctx, &whole_repo_event(&repo), &config);
    assert_eq!(reverted, ctx.cache.get(&repo).expect("full scan"));
}

/// A tracked path whose first component can be respelled in case.
struct CaseSlot {
    index: &'static str,
    variant: &'static str,
    tail: &'static str,
    on_disk: &'static str,
}

impl CaseSlot {
    fn file(&self, repo: &Path) -> PathBuf {
        repo.join(format!("{}{}", self.on_disk, self.tail))
    }
}

fn apply_case_op(rng: &mut Rng, repo: &Path, slots: &mut [CaseSlot]) -> (Vec<PathBuf>, String) {
    let choice = rng.below(slots.len());
    let slot = slots.get_mut(choice).expect("slot in range");
    match rng.below(7) {
        0 | 1 => {
            let file = slot.file(repo);
            fs::write(&file, format!("{}\n", rng.below(1000))).expect("write");
            (vec![file], format!("edit {}", slot.on_disk))
        }
        2 => {
            let file = slot.file(repo);
            fs::write(&file, "orig\n").expect("restore");
            (vec![file], format!("revert {}", slot.on_disk))
        }
        3 | 4 => {
            let old = repo.join(slot.on_disk);
            let target = if slot.on_disk == slot.index {
                slot.variant
            } else {
                slot.index
            };
            rename_via_temp(repo, slot.on_disk, target);
            let op = format!("rename {} -> {target}", slot.on_disk);
            slot.on_disk = target;
            (vec![old, repo.join(target), slot.file(repo)], op)
        }
        5 => {
            git_in(repo, &["add", "-u"]);
            (vec![repo.join(".git/index")], "git add -u".to_owned())
        }
        _ => {
            git_in(repo, &["commit", "-q", "-m", "step", "--allow-empty"]);
            (vec![repo.join(".git/index")], "git commit".to_owned())
        }
    }
}

#[test]
fn incremental_refresh_matches_git_status_for_case_variant_edits() {
    for seed in 1..=12_u64 {
        let (_tmp, repo) = create_temp_repo();
        if !repo_ignores_case(&repo) {
            return;
        }
        let mut slots = [
            ("Readme.md", "README.md", ""),
            ("Notes.txt", "NOTES.TXT", ""),
            ("Src", "SRC", "/lib.txt"),
        ]
        .map(|(index, variant, tail)| CaseSlot {
            index,
            variant,
            tail,
            on_disk: index,
        });
        fs::create_dir_all(repo.join("Src")).expect("src dir");
        for slot in &slots {
            fs::write(slot.file(&repo), "orig\n").expect("seed file");
        }
        commit_all(&repo);

        let ctx = make_agent_context();
        let config = Config::default();
        let backend = NativeGitBackend;
        refresh_repo_status_with(&backend, &ctx, &repo, &config, None);

        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut log = Vec::new();
        for _ in 0_u32..14_u32 {
            let (reported, op) = apply_case_op(&mut rng, &repo, &mut slots);
            log.push(op);
            let hint = git_paths_hint(&GitPaths::Paths(reported), &repo);
            let refreshed =
                refresh_repo_status_with(&backend, &ctx, &repo, &config, hint.as_deref());
            let status = refreshed.status.expect("repository status");
            let want_files = oracle_files(&repo);
            let got_files = ctx.cache.files_for_test(&repo).unwrap_or_default();
            assert_eq!(
                got_files, want_files,
                "seed {seed}: cached file map diverged from git after {log:#?}"
            );
            assert_eq!(
                StatusAggregate {
                    staged: status.staged,
                    unstaged: status.unstaged,
                    untracked: status.untracked,
                    conflicts: status.conflicts,
                },
                StatusAggregate::from_file_statuses(want_files.values()),
                "seed {seed}: status counts diverged from git after {log:#?}"
            );
        }
    }
}
