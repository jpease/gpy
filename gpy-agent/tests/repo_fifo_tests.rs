//! A writer-less FIFO inside `.git` (say, unpacked from an archive) blocks every
//! git subprocess that opens it. `gpy-agent oneshot git` must still finish
//! within `git.timeout_seconds`, not once per git command it runs.
#![cfg(unix)]
#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]

#[path = "common/cli_harness.rs"]
mod cli_harness;
#[path = "common/git.rs"]
mod git_helpers;

use cli_harness::CliTestEnv;
use git_helpers::{git, git_init};
use std::fs;
use std::time::{Duration, Instant};

/// `git.timeout_seconds` for these tests.
const GIT_TIMEOUT: Duration = Duration::from_secs(1);
/// Process start, config load and the SIGTERM-to-SIGKILL grace on top of the budget.
const SLACK: Duration = Duration::from_secs(2);

#[test]
fn oneshot_git_stays_within_its_timeout_when_git_dir_files_are_fifos() {
    for name in ["HEAD", "config", "index"] {
        let env = CliTestEnv::with_config(&format!(
            "[git]\ntimeout_seconds = {}\n",
            GIT_TIMEOUT.as_secs()
        ))
        .expect("create isolated CLI test env");
        let repo = env.root().join("repo");
        fs::create_dir_all(&repo).expect("create repo dir");
        git_init(&repo);
        fs::write(repo.join("tracked"), "x\n").expect("write tracked file");
        git(&repo, &["add", "tracked"]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        let fifo = repo.join(".git").join(name);
        fs::remove_file(&fifo).expect("remove the regular file");
        nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::S_IRWXU).expect("create FIFO");

        let started = Instant::now();
        let result = env
            .run_gpy_agent(&["oneshot", "git", "--cwd", &repo.to_string_lossy()])
            .expect("spawn gpy-agent oneshot git");
        let elapsed = started.elapsed();

        assert!(
            elapsed < GIT_TIMEOUT + SLACK,
            "a FIFO .git/{name} held oneshot git for {elapsed:?}, past the {GIT_TIMEOUT:?} git budget: {result:?}"
        );
    }
}
