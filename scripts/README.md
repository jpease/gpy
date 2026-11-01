# GPY Scripts

Development, release and quality scripts. Most are wrapped by a `just` recipe
(`just --list`).

## quality-check.sh

The local quality gate.

```bash
./scripts/quality-check.sh               # full gate
./scripts/quality-check.sh --fast        # repo checks, Fish syntax, shellcheck, parity tests, Rust lint and tests
./scripts/quality-check.sh --rust-only   # fmt, clippy, tests, doc tests, audit, deny, machete, release build
./scripts/quality-check.sh --clippy      # clippy only, exactly as the gate runs it (`just lint`)
./scripts/quality-check.sh --shell-only  # Fish lint, shellcheck, repo invariants, Fish/Bash/Zsh suites
./scripts/quality-check.sh --shell-only bash,zsh  # a subset of checks,repo,fish,bash,zsh; bash1 and bash2 are its halves (CI shards by them)
./scripts/quality-check.sh --fish-only   # Fish syntax and formatting
./scripts/quality-check.sh --fix         # fish_indent, cargo fmt, clippy --fix, then the full gate
```

It checks:

- **Repository:** license metadata, privacy patterns, documentation links,
  Rust file size (`check-file-size.py`).
- **Fish:** syntax of every `.fish` file; formatting with `fish_indent`.
- **Shell scripts:** `shellcheck -x` over the installers, `scripts/*.sh`, `scripts/lib/*.sh`,
  `tests/bash/*.test.bash`, `tests/repo/*.test.bash`, `benchmarks/*/*.sh` and the shipped `bash/` tree,
  discovered by glob. `install-oneline.sh` is checked as POSIX `sh`. An inline
  `# shellcheck disable=` needs a reason.
- **Repo invariants:** `tests/repo/*.test.bash` (CI, release, toolchain and
  documentation invariants), run in the `repo` part.
- **Shell suites:** Fish, Bash and Zsh tests against this checkout's debug
  agent, each Bash/Zsh file in its own XDG root. The run fails if a suite
  changes this repository's HEAD or worktrees.
- **Rust:** `cargo fmt`, clippy (policy in `gpy-agent/Cargo.toml` `[lints]`;
  `cargo clippy --all-targets` then `cargo clippy --lib --bins`, the second
  without the `test-support` feature that test and bench builds enable),
  nextest, doc tests, `cargo audit`, `cargo deny`, `cargo machete` and a release
  build (always with `--rust-only`; with the full gate only under CI or
  `GPY_GATE_RELEASE=1`).

Required: the pinned Rust toolchain with clippy and rustfmt, `cargo-nextest`,
`fish`, `zsh` (for the `zsh` part), `cargo-audit`, `cargo-deny` and, for the
`repo` part, mikefarah `yq` ≥ 4.52.4
(`brew install yq`, or the release binary from github.com/mikefarah/yq; not
Debian's `apt install yq`). Optional: `fish_indent`, `shellcheck`,
`cargo-machete`. A missing optional tool prints `SKIP:` and passes
locally (the end-of-run summary lists it); under CI or with `GPY_GATE_STRICT=1`
a skip is a failure. `just check` sets `GPY_GATE_STRICT=1`, so it is the one
pre-push command; `just check-fast` stays lenient. `--rust-only` also skips a
missing `cargo-audit` or `cargo-deny` that way.

## Languages

Scripts and tests are Bash or POSIX `sh` (Fish and Zsh for their own suites).
Python 3, standard library only, is used where Bash cannot do the job: PTY
driving, Unix-socket stubs and line-accurate source scanning. Every Python file
runs in the gate, so there is no separate Python lint.

Exit status is 0 when every check passed, 1 otherwise.

## Other scripts

| Script | Purpose |
|---|---|
| `gpy-dev.sh [--release] [--ref REV \| --pr N \| --bin DIR] [--reset] [--sync-config]` | Run an unreleased build in an isolated fish shell (own config, fish scripts, caches and agent socket under `~/.local/share/gpy-dev`) and leave your installed release untouched; `exit` returns to it. The prompt shows `[gpy-dev <version> <ref>]`. `just dev`. |
| `test-fish.sh` | Build the debug binaries and run `tests/fish/*.test.fish`; `just test-shell` and `just check-shell fish` run it. |
| `check-active-toolchain.sh` | Fail unless the `rustc` active in `gpy-agent/` is the `rust-toolchain.toml` pin; names the override on a mismatch. |
| `check-license-metadata.sh` | `Cargo.toml`, `LICENSE` and every other license declaration agree on GPL-3.0-or-later. |
| `check-privacy-patterns.sh` | No workstation paths, private identifiers or hardcoded sockets in tracked files. |
| `check-doc-links.sh` | Every repository-relative Markdown link resolves. |
| `check-platforms.sh [--worktree \| <rev>]` | Compile `<rev>` (default `HEAD`; `--worktree` adds uncommitted changes) in a Docker container: Linux `cargo clippy` (the same two passes as `just lint`) and Windows (mingw) `cargo check` for `--lib`, `--locked`, `--profile test --tests`. Exit 1 on a failure, 2 when Docker is unreachable. `just check-platforms`. |
| `check-new-allows.py [--staged \| <base-rev>]` | Fail when a change adds an `allow` attribute without `reason = "..."`; existing allows are not checked. Run at pre-commit (`--staged`) and by the `new-allows` job in `pr-gate.yml`. |
| `check-file-size.py [--update \| --init]` | Fail when a tracked Rust file (top-level inline test modules excluded) exceeds 800 lines, or a file listed in `file-size-allowlist.txt` grew or shrank without its entry being lowered (`--update`). Test files (`tests/` targets, `tests.rs`, `*_tests.rs`) are held to the same cap; those over it are listed in `file-size-allowlist.txt` until split, and no new test may be added to a listed file. Run by the `file-size` hook and `quality-check.sh`. |
| `cleanup-test-agents.sh` | Reap agents and sockets left by killed Rust test runs. |
| `bench.sh --ci` | Rebuild the release agent, run `cargo bench` and enforce the Rust micro-benchmark budgets from `benchmarks/perf-budgets.json` (part of `just bench-ci`; the PR gate's `perf` job runs it). Lists every benchmark ID first and fails when two bench targets register the same one. Exit 1 on an exceeded budget, a duplicate ID or a failed bench; exit 2 before benchmarking when `jq`, the budget file or a budget key is missing. `PERF_BUDGETS_FILE` points it at another budget file. |
| `ci-bench.sh` | End-to-end prompt-latency and agent-memory budgets (`just bench-ci`); same exit 2 and `PERF_BUDGETS_FILE` rules as `bench.sh`. |
| `lib/bench-common.sh` | Sourced by both bench scripts: reads every budget in one `jq` call, rebuilds the release agent and prints the `info:`/`PASS:`/`warn:`/`FAIL:` log lines. |
| `perf-canary.sh` | Compare the `canary_ipc_roundtrip` and `canary_git_status_cold` benchmarks against an upstream commit on the same machine. |
| `build-release-binaries.sh` | Build release binaries for every supported platform into `bin/`. |
| `check-glibc-floor.sh <binary> <floor>` | Fail when a Linux binary needs a `GLIBC_` version above `<floor>`. |
| `generate-sbom.sh` | CycloneDX SBOM from `Cargo.lock` (needs `jq`). |
| `package-release.sh` | Build the release archives and checksums; fails closed on any missing file. |
| `release-notes.sh` | Release notes from the CHANGELOG section and the built assets. |
| `smoke-release.sh` | Install and run the binaries a release is about to publish, in a sandboxed `HOME`. |
| `test-fresh-install.sh` | Install into a stock `ubuntu:24.04` container and render a prompt per shell. |
| `wsl-smoke.sh [--contract \| --contract-only]` | Inside WSL: font detection via `gpy setup`, `gpy-agent oneshot git` on Linux and Windows-drive repos, and optionally the cross-shell contract suite. `GPY_AGENT_BIN` and `GPY_WSL_DRVFS_DIR` override paths. |
| `install-hooks.sh [--uninstall]` | Install (or remove) the prek git hooks from `.pre-commit-config.yaml`. |
