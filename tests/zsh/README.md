# Zsh Tests

Every `*.test.zsh` in this directory is auto-discovered and run by
`scripts/quality-check.sh` (`just check-shell`), each in a hermetic
`XDG_CACHE_HOME`/`XDG_CONFIG_HOME`. A file must exit non-zero on failure and
print `PASS`/`FAIL` lines a reader can follow; the file name says what it
covers. All files here test `zsh/gpy.zsh`; repository-policy checks live in
`tests/bash/`.

Cross-shell behaviour is not tested here: a Zsh behaviour meant to match Fish
and Bash is a row of `tests/fixtures/shell_scenarios/scenarios.tsv`, run by
`tests/bash/shell_contract.test.bash` (see `tests/bash/README.md`).

## Running

```bash
just check-shell                            # everything, as the gate runs it
just check-shell zsh                        # the Zsh suite only
zsh tests/zsh/e2e_agent_autostart.test.zsh  # one file
```

## Writing a test

`tests/lib/shell_e2e.sh` is shared with the Bash suite (assertion helpers
`fail`/`pass`/`test_finish`, the skip contract (`test_skip`,
`test_require_command`, `test_skip_part`: exit 1 under `CI` or `GPY_GATE_STRICT`), `shell_e2e_poll`, the
`shell_e2e_stub_listener` stand-in agent and the live-daemon harness). Source
it with sh semantics so its functions keep sh word-splitting:

```zsh
ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
```

An exported `GPY_AGENT_ENABLED` or `GPY_AGENT_SUPERVISOR_ENABLED` wins over
`config.toml` (the theme export never exports either), so exporting
`GPY_AGENT_ENABLED=0` stops `gpy.zsh` from starting an agent. Tests
that must not start one source `tests/lib/supervisor_off.zsh`, write
`[agent] enabled = false` into their own sandbox `config.toml`, or source with
no `gpy-agent` on `PATH` (`shell_e2e_agent_free_begin` /
`shell_e2e_agent_free_end`).
