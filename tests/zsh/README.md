# Zsh Tests

Every `*.test.zsh` in this directory is auto-discovered and run by
`scripts/quality-check.sh` (`just check-shell`), each in a hermetic
`XDG_CACHE_HOME`/`XDG_CONFIG_HOME`. A file must exit non-zero on failure and
print `PASS`/`FAIL` lines a reader can follow; the file name says what it
covers. All files here test `zsh/gpy.zsh`; repository-policy checks live in
`tests/bash/`.

Rendered prompt output for all three shells is pinned by
`tests/bash/prompt_golden.test.bash`; the tests here cover what `zsh/gpy.zsh`
itself owns and end-to-end behaviour through a real shell and agent.

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
`config.toml`, so exporting `GPY_AGENT_ENABLED=0` stops `gpy prompt zsh` from
starting an agent on the shell's first prompt. Tests that must not start one
source `tests/lib/supervisor_off.zsh` or write `[agent] enabled = false` into
their own sandbox `config.toml`. The binary starts the `gpy-agent` beside
itself, so a test that records starts copies `gpy` next to a stub
`gpy-agent` (see `first_prompt_agent_start.test.zsh`).
