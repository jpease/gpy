# Fish Shell Integration Tests

Integration tests for the Fish prompt and GPY initialization. Every
`*.test.fish` file here is a standalone script; the file name says what it
covers.

Rendered prompt output for every shell is pinned by
`tests/bash/prompt_golden.test.bash` and the Rust `prompt::` unit tests; the
tests here cover what the Fish wrapper itself does and end-to-end behaviour
through a real Fish and agent.

## Running

```bash
./scripts/test-fish.sh                     # check the toolchain, build the agent, run every *.test.fish
fish tests/fish/init_integration.test.fish # one file, from any directory
```

A test that calls `init_test_env` uses this checkout's
`gpy-agent/target/debug` build (built if missing; rebuild after Rust changes),
never an installed `gpy-agent`.

## Writing a test

- Source `tests/lib/test_helpers.fish` for the sandbox (`init_test_env`), the
  live-agent helpers (`start_test_agent`, `stop_test_agent`), bounded waits
  (`poll_until SECS CMD...`) and `test_skip`. Never `sleep` to wait for
  something you can poll; a sleep stays only where elapsed time is what the
  test measures or a window in which nothing may happen, with a comment.
- Anything the test starts that can outlive it is owned by the teardown guard:
  `init_test_env` arms it for the sandbox and agent; a test that makes its own
  scratch directory or background process registers it with
  `test_track_dir DIR` / `test_track_pid PID` / `test_track_socket SOCK`. The
  guard kills those (by PID or by the socket they hold, never by process
  name) and removes the directories on every exit path, including a failed
  assertion, `test_skip`, SIGINT and SIGTERM.
- A test that needs a stand-in agent on a Unix socket (silent, slow,
  answering, cut off mid-reply, or a socket file nobody listens on) uses
  `test_stub_listener SOCKET MODE [OPTION...]`, which runs
  `tests/lib/stub_listener.py` and registers it with the guard; never an
  inline Python listener. `test_stop_pids $test_stub_pid` stops it early.
- A test that cannot run because a prerequisite is missing calls
  `test_skip "reason"`: it passes locally and fails under `CI` or `GPY_GATE_STRICT`.
- Find the repository root from the file (`status --filename`) so the test
  runs from any directory, and exit non-zero on any failed assertion.
- The prompt is drawn by `gpy prompt fish`; the shell side is only
  `fish/conf.d/gpy_init.fish` (loads in an interactive Fish only: use
  `fish -i`) and `fish/functions/fish_prompt.fish`. Assert on what the shell
  prints and on observable effects (the agent's `status`, runtime files), and
  choose segments, theme and icons through `$XDG_CONFIG_HOME/gpy/config.toml`.
  A prompt that must not start an agent runs with `GPY_AGENT_ENABLED=0`.
- Tests that drive an interactive `fish -i` use `tests/lib/pty_session.py`
  (`start`, `send`, `wait-for`, `size`, `stop`); `wait-for` only reads the
  transcript, so it can observe repaints that arrive with no keystroke.
