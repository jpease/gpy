# Bash Tests

Every `*.test.bash` in this directory is auto-discovered and run by
`scripts/quality-check.sh` (`just check-shell`), each in a hermetic
`XDG_CACHE_HOME`/`XDG_CONFIG_HOME`. A file must exit non-zero on failure and
print `PASS`/`FAIL` lines a reader can follow; the file name says what it
covers.

Two kinds of file live here: shell-integration tests of `bash/gpy.bash`, and
repository-policy checks that use bash as a scripting language to assert on
the repo itself (doc links, privacy, workflow hardening, the CI gate shape,
the release asset contract, installer checksum verification).

## Running

```bash
just check-shell                                  # everything, as the gate runs it
bash tests/bash/e2e_agent_autostart.test.bash     # one file
GPY_GATE_RELEASE=1 bash tests/bash/install_from_source_docs.test.bash   # builds a release binary
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
  locally, exit 1 under `CI`. Never `exit 0` on a missing prerequisite.
- All waits are bounded polls (`shell_e2e_poll SECONDS CMD...`); do not add
  fixed sleeps for synchronisation.
- Tests that must not start an agent source `tests/lib/supervisor_off.bash`,
  write `[agent] enabled = false` into their own sandbox `config.toml`, or
  source with no `gpy-agent` on `PATH` (`shell_e2e_agent_free_begin` /
  `shell_e2e_agent_free_end`). Exporting `GPY_AGENT_ENABLED=0` alone is not
  enough: the theme export re-sets it from `config.toml`.

### The live-daemon harness

`tests/lib/shell_e2e.sh` (POSIX sh, shared with the Zsh suite) documents its
functions in its header: `shell_e2e_init` builds the sandbox, debug agent and
a scratch git repo; `shell_e2e_start_agent` / `shell_e2e_stop_agent`;
`shell_e2e_text_config` writes a text-theme `config.toml`;
`shell_e2e_spawn_client bash|zsh` drives an interactive shell on a pty
through `tests/lib/pty_session.py` (`shell_e2e_send`, `shell_e2e_wait_for`,
`shell_e2e_transcript`).

### The cross-shell contract harness: `shell_contract`

Behaviour that must be identical in Fish, Bash and Zsh is a row of
`tests/fixtures/shell_scenarios/scenarios.tsv`, not a per-shell file.
`shell_contract.test.bash` runs every row through the real integrations
(`tests/lib/shell_scenarios/render.{fish,bash,zsh}`), each in its own sandbox,
and fails when the normalized prompts or the side effects (agent starts,
`gpy-agent oneshot` forks, IPC requests, registrations) differ between shells
or miss the row's `expect` values. Side effects are observed from outside the
shells: a recording `gpy-agent` wrapper first on `PATH` and a recording IPC
proxy (`tests/lib/shell_scenarios/ipc_proxy.py`) in front of the real agent.

To add a row, append it to `scenarios.tsv` (its header documents every column
and flag) and run just that row:

```bash
SCN_FILTER='^my-row$' SCN_VERBOSE=1 bash tests/bash/shell_contract.test.bash
```

`SCN_VERBOSE=1` prints each shell's side effects and normalized prompt, which
is how to pick the `expect` values. Pin the behaviour, not the whole log.
