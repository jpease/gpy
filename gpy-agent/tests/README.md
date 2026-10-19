# GPY Agent Integration Tests

Every `*.rs` file directly under `gpy-agent/tests/` is its own test binary;
each starts with a `//!` header saying what it covers. Unit tests live next
to the code in `src/`.

## Layout

- `common/` — shared helpers. A binary either declares `mod common;` (all of
  them) or includes one file with `#[path = "common/<file>.rs"] mod <file>;`.
  - `cli_harness.rs` — `CliTestEnv`: runs the `gpy`/`gpy-agent` binaries
    against an isolated HOME/XDG tree.
  - `child_guard.rs` — drop guards that terminate spawned processes on every
    exit path: `KillOnDrop` (a `std::process::Child`), `PtyChild` (a
    `portable_pty` child), `AgentStopGuard` (runs `gpy-agent stop` for a
    daemonised agent).
  - `endpoint.rs` — `endpoint_builder(socket_path)` and
    `endpoint_builder_with_config(socket_path, config)`: in-process
    `EndpointHandle` builders with default subsystems (the second takes the
    `ConfigManager` the server reads); `wait_for_socket`.
  - `env_guard.rs` — `EnvGuard`: restores every environment variable it
    touched on drop, including when an assertion fails.
  - `fixtures.rs` — `ServerGuard` (aborts a spawned server task on drop),
    `TempSocket`, `gpy_test_root()`, `wait_for_agent_ready` (blocking).
  - `git.rs` — `git(dir, args)` and `git_init(dir)` for fixture repositories.
  - `integration_harness.rs` — `IntegrationTestServer`: a real daemonized
    agent plus a temp repository.
  - `sgr.rs` — ANSI SGR interpreter for asserting rendered styles.
  - `skip.rs` — `skip_test(reason)`.
  - `wait.rs` — `wait_until` / `wait_until_async`: poll a condition with a
    deadline.
- `fixtures/` — `TestRepo`, `TestConfig`, IPC message builders, and the
  base16/Starship input files the import golden tests read.
- `test_harness.rs` — `SignalCounter` and `EventFlag` for waiting on signals
  and events without fixed sleeps.
- `snapshots/` — insta snapshots for the help and formatter tests.

## How the targets run

- **Runner**: `cargo nextest run --features test-support` from `gpy-agent/`
  (`just check-rust` wraps it). Plain `cargo test` works but runs each
  binary in one process; nextest's per-test processes are what the
  isolation below assumes. Doc tests still need `cargo test --doc`.
- **`test-support` feature**: compiles in the test-only library surface
  (`ThemeManager::watch_registry_for_test`); `just lint` and the pre-push gate pass it.
- **Serialisation**: tests that touch process-global state (environment
  variables, the config watcher's file, one real agent per socket) are
  marked `#[serial_test::serial]` or `#[serial_test::file_serial]`. The
  `file_locks` feature makes that hold across nextest's processes.
- **Retries**: none by default. `profile.default` in
  `gpy-agent/.config/nextest.toml` has no retry budget; a per-test override
  must name its tests and cite an issue.
- **Skips**: a test that cannot run calls `skip::skip_test("reason")`: it
  prints `SKIP: reason` and passes locally, and fails when `CI` is set.
  Never return silently.
- **Sandboxing**: every spawned agent binds its socket under
  `gpy_test_root()`; `scripts/cleanup-test-agents.sh` reaps leftovers there
  and nowhere else.

## Writing tests

- Wait on a condition, not a duration: `wait_until`/`wait_until_async`,
  `wait_for_socket`, `SignalCounter::wait_for_count`,
  `EventFlag::wait_until_set`. A fixed sleep is only right when the test
  proves that something does *not* happen, and its comment should say so.
- Set environment variables through `EnvGuard` and mark the test
  `#[serial]`.
- Unwrap setup with `.expect("...")`. A `let Ok(..) = .. else { return }`
  turns a broken fixture into a passing test.
- Assert the value, not just `is_ok()`, and make sure the assertion can fail.

## Running tests

All commands from `gpy-agent/`:

```bash
cargo nextest run --features test-support                          # everything (what the gate runs)
cargo nextest run --features test-support --test clock_timer_tests # one binary
cargo nextest run --features test-support -- test_clock_signals    # tests matching a name
cargo nextest run --features test-support --no-capture             # see stdout/stderr live
cargo test --doc                                                   # doc tests (nextest does not run them)
```

## Coverage

`just coverage` (from the repository root) runs the suite under
`cargo llvm-cov nextest`, writes `gpy-agent/lcov.info` and prints the summary.
The current figure is recorded in [docs/dev/testing.md](../../docs/dev/testing.md).
For a browsable report:

```bash
cargo llvm-cov nextest --features test-support --html --open
```

## Platforms

- **macOS** — primary development platform; the maintainer's pre-push gate
  runs the full suite, including the shell end-to-end tiers.
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
