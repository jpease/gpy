# Bash Tests

Every `*.test.bash` in this directory is auto-discovered and run by
`scripts/quality-check.sh` (`just check-shell`), each in a hermetic
`XDG_CACHE_HOME`/`XDG_CONFIG_HOME`. A file must exit non-zero on failure and
print `PASS`/`FAIL` lines a reader can follow; the file name says what it
covers.

This directory holds tests of `bash/gpy.bash` and of the shipped `sh`/`bash`
installers (`install*.test.bash`, `uninstall_*`, `installer_*`). Checks of the
repository, CI, release and tooling live in `tests/repo/`. Repository
invariants belong in tests; doc wording and source text do not.

## Running

```bash
just check-shell                                  # everything, as the gate runs it
just check-shell bash                             # the Bash suite only
bash tests/bash/e2e_agent_autostart.test.bash     # one file
bash tests/bash/install_e2e_real_binary.test.bash # one installer end-to-end
```

## Writing a test

Source the shared library first; it provides the assertion helpers and the
skip contract:

```bash
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

if [[ "$got" == "$want" ]]; then pass "label"; else fail "label: got '$got'"; fi
test_finish "one-line summary"   # exit 1 if any fail, else PASS: summary
```

- `test_skip "reason"` / `test_require_command NAME`: `SKIP:` and exit 0
  locally, exit 1 under `CI` or `GPY_GATE_STRICT`. `test_skip_part "reason"` does the same for one
  section of a test: it returns locally so the rest still runs. Never print
  `SKIP:` or `exit 0` on a missing prerequisite directly; `quality-check.sh`
  and `test-fish.sh` list every test whose output has a `SKIP:` line.
- All waits are bounded polls (`shell_e2e_poll SECONDS CMD...`); do not add
  fixed sleeps for synchronisation. A sleep stays only where elapsed time is
  the behaviour (a measured duration, a TTL) or a window in which nothing may
  happen, and says so in a comment. Backdate a file (`shell_e2e_backdate`) or
  fake the clock a function reads rather than waiting an interval out.
- Sourcing `shell_e2e.sh` turns SIGINT/SIGTERM/SIGHUP into an `exit`, so a
  test's EXIT trap runs when it is interrupted or timed out.
  `shell_e2e_init` arms the EXIT trap that stops the client and agent, kills
  whatever holds a unix socket under the sandbox, and removes it; register a
  helper process with `shell_e2e_track_pid PID`.
- A test that needs a stand-in agent on a Unix socket (silent, slow,
  answering, cut off mid-reply, or a socket file nobody listens on) uses
  `shell_e2e_stub_listener SOCKET MODE [OPTION...]`, which runs
  `tests/lib/stub_listener.py`; never an inline Python listener. Stop it with
  `shell_e2e_stop_pids`; a test without `shell_e2e_init` calls
  `shell_e2e_kill_tracked` from its own EXIT trap.
- Tests that must not start an agent source `tests/lib/supervisor_off.bash`
  or write `[agent] enabled = false` into their own sandbox `config.toml`.
  An exported `GPY_AGENT_ENABLED=0` wins over `config.toml`, so it is enough
  on its own. `gpy prompt` starts the `gpy-agent` beside the `gpy` binary, so
  a test that records agent starts copies `gpy` next to a stub `gpy-agent`.

### The live-daemon harness

`tests/lib/shell_e2e.sh` (POSIX sh, shared with the Zsh suite) documents its
functions in its header: `shell_e2e_init` builds the sandbox, debug agent and
a scratch git repo; `shell_e2e_start_agent` / `shell_e2e_stop_agent`;
`shell_e2e_text_config` writes a text-theme `config.toml`;
`shell_e2e_spawn_client bash|zsh` drives an interactive shell on a pty
through `tests/lib/pty_session.py` (`shell_e2e_send`, `shell_e2e_wait_for`,
`shell_e2e_transcript`).

### Rendered prompts: `prompt_golden`

Every prompt is drawn by `gpy prompt <shell>`, so what a prompt looks like is
pinned once for all three shells: `prompt_golden.test.bash` compares the
rendered output for each theme and case with
`tests/fixtures/prompt_golden/**`. Per-shell tests cover only what the thin
wrappers still own (hooks, `$?`, traps, durations, exit) and end-to-end
behaviour through a real shell and agent.
