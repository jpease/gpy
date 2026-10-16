# Fish Shell Integration Tests

Integration tests for the Fish prompt and GPY initialization. Every
`*.test.fish` file here is a standalone script; the file name says what it
covers.

Behaviour that must be the same in Fish, Bash and Zsh (cold-miss and variant
fallback, slow replies, the IPC timeout and cache TTL knobs, agent-disabled and
supervisor-disabled starts, the clock/status/root segments) is not tested here
in Fish alone: it is a row of `tests/fixtures/shell_scenarios/scenarios.tsv`,
run through all three shells by `tests/bash/shell_contract.test.bash`.

## Running

```bash
./scripts/test_fish.sh                     # syntax, formatting, then every *.test.fish
fish tests/fish/init_integration.test.fish # one file, from any directory
```

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
- A test that cannot run because a prerequisite is missing calls
  `test_skip "reason"`: it passes locally and fails under `CI`.
- Find the repository root from the file (`status --filename`) so the test
  runs from any directory, and exit non-zero on any failed assertion.
- For unit-style tests set `GPY_AGENT_SUPERVISOR_ENABLED=0` and keep
  `GPY_AGENT_ENABLED=1`; setting both to `0` takes init.fish's
  completely-disabled early exit.
- Tests that drive an interactive `fish -i` use `tests/lib/pty_session.py`
  (`start`, `send`, `wait-for`, `size`, `stop`); `wait-for` only reads the
  transcript, so it can observe repaints that arrive with no keystroke.
