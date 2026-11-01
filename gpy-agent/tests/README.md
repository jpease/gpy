# GPY Agent Integration Tests

Every `*.rs` file directly under `gpy-agent/tests/` is its own test binary;
each starts with a `//!` header saying what it covers. Unit tests live next
to the code in `src/`.

A test target may instead be a directory, `tests/<name>/main.rs` plus modules,
when one file would exceed the 800-line cap (`gpy_cli_tests/` is one). The
binary keeps the name `<name>`; `main.rs` declares the shared helpers with
`#[path = "../common/<file>.rs"]`, and its modules reach them through
`crate::` paths (`crate::cli_harness`).
Test files over the cap are listed in `scripts/file-size-allowlist.txt`; a
listed file may not grow, so new tests go in a new file. When a listed file
shrinks (a test deleted or moved out), the size check fails until its entry is
lowered with `python3 scripts/check-file-size.py --update` in the same commit
(`--update` also drops a file that falls under the cap).

## Layout

- `common/` — the shared helpers. A binary either declares `mod common;`
  (all of them) or includes one self-contained file with
  `#[path = "common/<file>.rs"] mod <file>;`. `integration_harness.rs` and
  `pty.rs` use their siblings, so they are reachable only through
  `mod common;`.
  - `agent_sandbox.rs` — sets up an isolated daemon with a private socket and test environment.
  - `cli_harness.rs` — runs `gpy` and `gpy-agent` binaries against an isolated HOME/XDG directory structure.
  - `child_guard.rs` — drop guards that terminate spawned processes on every exit path.
  - `endpoint.rs` — in-process IPC endpoints and socket helpers for testing.
  - `env_guard.rs` — restores environment variables on drop.
  - `fake_agent.rs` — fake Unix-socket agent for protocol testing.
  - `fixtures.rs` — test fixtures and temporary socket helpers.
  - `git.rs` — git repository setup and commands for tests. Each command ignores the global and system git config (`GIT_CONFIG_GLOBAL` at the null device, `GIT_CONFIG_NOSYSTEM=1`) and commits as a fixed identity.
  - `integration_harness.rs` — daemonized agent test server with temporary repository fixtures.
  - `pty.rs` — PTY terminal test helpers.
  - `sgr.rs` — ANSI SGR interpreter for asserting rendered styles.
  - `sigurg.rs` — counting SIGURG signal handler for doorbell testing.
  - `skip.rs` — skips tests with explanatory reasons.
  - `wait.rs` — polling and waiting with deadlines.
- `fixtures/` — the base16/Starship input files the import golden tests read,
  and `released_themes/` (`text-v0.6.0.toml`, `default-v0.7.1.toml`,
  `starship-v0.7.1.toml`), builtin themes as an older release shipped them
  (theme retirement tests).
- `snapshots/` — insta snapshots for the help and formatter tests.

## How the targets run

- **Runner**: `cargo nextest run` from `gpy-agent/` (`just test-rust`; the
  gate runs it through `just check-rust`). Plain `cargo test` works but runs
  each binary in one process; nextest's per-test processes are what the
  isolation below assumes. Doc tests still need `cargo test --doc`.
- **Test-only library surface**: integration tests and benches link the
  library as an outside crate, so the hooks they need on public types are
  `pub` items behind `#[cfg(feature = "test-support")]` (or
  `#[cfg(any(test, feature = "test-support"))]` when unit tests share them),
  and nowhere else. `gpy-agent` lists itself as a dev-dependency with that
  feature, so every test, bench and example build compiles the hooks with no
  flag, while `--lib`, release and `cargo doc` builds never do. List them with `git grep -n -A3 'feature = "test-support"' src`. A test that needs a specific
  config builds its manager with `ConfigManager::from_config`, or writes the
  file and calls `reload_now` to change it mid-test.
- **Serialisation**: tests that touch process-global state (environment
  variables, the config watcher's file, one real agent per socket) are
  marked `#[serial_test::serial]` or `#[serial_test::file_serial]`. The
  `file_locks` feature makes that hold across nextest's processes.
- **Retries**: none by default. `profile.default` in
  `gpy-agent/.config/nextest.toml` has no retry budget; a per-test override
  must name its tests and cite an issue.
- **Skips**: a test that cannot run calls `skip::skip_test("reason")`: it
  prints `SKIP: reason` and passes locally, and fails when `CI` or `GPY_GATE_STRICT` is set.
  Never return silently.
- **Sandboxing**: every spawned agent binds its socket under
  `gpy_test_root()`; `scripts/cleanup-test-agents.sh` reaps leftovers there
  and nowhere else.

## Writing tests

- Wait on a condition, not a duration: `wait_until`/`wait_until_async`,
  `wait_for_socket`. When nothing signals readiness (an OS file watch
  arming), write a sentinel the code under test reports, repeating it until
  it arrives, as `prime_watch` in `watcher_tests.rs` and `arm_watch` in
  `multi_repo_integration_tests.rs` do.
- A fixed sleep is only right when the test proves that something does
  *not* happen. Name the duration (`ABSENCE_WINDOW`, `SEVERAL_TICKS`) or
  say so in a comment.
- Do not assert how long something took. A duration is a benchmark
  (`gpy-agent/benches/`); a test asserts ordering instead (the callback was
  still running when `stop()` returned), or a bound only when the bound is
  itself the product contract.
- Build repositories with `common/git.rs`, never a bare
  `Command::new("git")`, so the developer's git config cannot leak in.
- Set environment variables through `EnvGuard` and mark the test
  `#[serial]`.
- Unwrap setup with `.expect("...")`. A `let Ok(..) = .. else { return }`
  turns a broken fixture into a passing test.
- Assert the value, not just `is_ok()`, and make sure the assertion can fail.
- `.clippy.toml`'s `allow-*-in-tests` covers only `#[test]` bodies, so a test file whose helper functions unwrap, or that includes `common/` modules (`mod common;` or `#[path = "common/…"]`), keeps the crate-level `#![allow(clippy::expect_used)]` (and `unwrap_used`/`panic` as needed).

## Running tests

All commands from `gpy-agent/`:

```bash
cargo nextest run                                # everything (what the gate runs)
cargo nextest run --test clock_timer_tests       # one binary
cargo nextest run -- test_clock_signals          # tests matching a name
cargo nextest run --no-capture                   # see stdout/stderr live
cargo test --doc                                 # doc tests (nextest does not run them)
```

## Coverage

`just coverage` (from the repository root) runs the suite under
`cargo llvm-cov nextest`, writes `gpy-agent/lcov.info` and prints the summary.
The current figure is recorded in [docs/dev/testing.md](../../docs/dev/testing.md).
For a browsable report:

```bash
cargo llvm-cov nextest --html --open
```

## Platforms

- **macOS** — primary development platform; runs the full suite,
  including the shell end-to-end tiers.
- **Linux** — the ubuntu legs of `pr-gate.yml`, which runs on every pull
  request to `main` (and on manual dispatch).
- **Windows** — `windows-gate.yml` (build, lib tests, CLI integration
  binaries), called by `pr-gate.yml` on every pull request and required by
  `gate-result`. Signal and socket tests are `#[cfg(unix)]`.

## Troubleshooting

- **Fish integration tests skip or fail**: check `fish --version`. The plugin
  e2e test `e2e_plugin_runtime_loading_and_missing_file_graceful` uses
  Fish-4-only syntax in its inline script and skips on older interpreters;
  the runtime itself supports Fish 3.6+.
- **Leftover agents or sockets after an interrupted run**:
  `scripts/cleanup-test-agents.sh`.
