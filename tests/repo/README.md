# Repository Tests

Invariants of the repository, CI, release and tooling, written in bash: CI gate
shape, workflow hardening, release contract and packaging, MSRV and toolchain
pins, privacy patterns, doc links, the file-size gate, tracked symlinks. Tests of the shell
integrations themselves live in `tests/bash/`, `tests/fish/` and `tests/zsh/`.

Every `*.test.bash` here is auto-discovered and run by
`./scripts/quality-check.sh --shell-only repo` (also part of `just check-shell`).
A file exits non-zero on failure and prints `PASS`/`FAIL` lines a reader can
follow.

## Running

```bash
./scripts/quality-check.sh --shell-only repo   # every file, as the gate runs it
bash tests/repo/ci_gate_shape.test.bash        # one file
```

## Prerequisites

Tests that read YAML or TOML (`ci_gate_shape`, `workflow_hardening`,
`deny_skip_list`, `msrv_consistency`) need mikefarah `yq` >= 4.52.4
(`brew install yq`, or the release binary from github.com/mikefarah/yq; not
Debian's `apt install yq`). Without it they print `SKIP:` and pass locally, and
fail under `CI` or `GPY_GATE_STRICT`.

## Writing a test

Source `tests/lib/shell_e2e.sh`. It provides `pass`, `fail`, `test_finish`,
`test_require_command` and `require_yq` (the guard to call first in a test that
runs `yq`).
