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
./scripts/quality-check.sh --shell-only  # Fish lint, shellcheck, Fish/Bash/Zsh suites
./scripts/quality-check.sh --shell-only bash,zsh  # a subset of checks,fish,bash,zsh; bash1 and bash2 are its halves (CI shards by them)
./scripts/quality-check.sh --fish-only   # Fish syntax and formatting
./scripts/quality-check.sh --fix         # fish_indent, cargo fmt, clippy --fix, then the full gate
```

It checks:

- **Repository:** license metadata, privacy patterns, documentation links,
  Rust file size (`check-file-size.py`).
- **Fish:** syntax of every `.fish` file; formatting with `fish_indent`.
- **Shell scripts:** `shellcheck -x` over the installers, `scripts/*.sh`, `scripts/lib/*.sh`,
  `tests/bash/*.test.bash`, `benchmarks/*/*.sh` and the shipped `bash/` tree,
  discovered by glob. `install-oneline.sh` is checked as POSIX `sh`. An inline
  `# shellcheck disable=` needs a reason.
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
`fish`, `cargo-audit` and `cargo-deny`. Optional: `fish_indent`, `shellcheck`,
`cargo-machete`, `zsh`. A missing optional tool prints `SKIP:` and passes
locally (the end-of-run summary lists it); under CI a skip is a failure. `--rust-only`
also skips a missing `cargo-audit` or `cargo-deny` that way.

Exit status is 0 when every check passed, 1 otherwise.

## Other scripts

| Script | Purpose |
|---|---|
| `gpy-dev.sh [--release] [--ref REV \| --pr N \| --bin DIR] [--reset] [--sync-config]` | Run an unreleased build in an isolated fish shell (own config, fish scripts, caches and agent socket under `~/.local/share/gpy-dev`) and leave your installed release untouched; `exit` returns to it. The prompt shows `[gpy-dev <version> <ref>]`. `just dev`. |
| `test-fish.sh` | Build the debug binaries and run `tests/fish/*.test.fish`. |
| `check-active-toolchain.sh` | Fail unless the `rustc` active in `gpy-agent/` is the `rust-toolchain.toml` pin; names the override on a mismatch. |
| `check-license-metadata.sh` | `Cargo.toml`, `LICENSE` and every other license declaration agree on GPL-3.0-or-later. |
| `check-privacy-patterns.sh` | No workstation paths, private identifiers or hardcoded sockets in tracked files. |
| `check-doc-links.sh` | Every repository-relative Markdown link resolves. |
| `check-platforms.sh [--worktree \| <rev>]` | Compile `<rev>` (default `HEAD`; `--worktree` adds uncommitted changes) in a Docker container: Linux `cargo clippy` (the same two passes as `just lint`) and Windows (mingw) `cargo check` for `--lib`, `--locked`, `--profile test --tests`. Exit 1 on a failure, 2 when Docker is unreachable. `just check-platforms`. |
| `check-new-allows.py [--staged \| <base-rev>]` | Fail when a change adds an `allow` attribute without `reason = "..."`; existing allows are not checked. Run at pre-commit (`--staged`) and by the `new-allows` job in `pr-gate.yml`. |
| `check-file-size.py [--update \| --init]` | Fail when a Rust source file (inline test modules excluded) exceeds 800 lines, or a file listed in `file-size-allowlist.txt` grew or shrank without its entry being lowered (`--update`). Run by the `file-size` hook and `quality-check.sh`. |
| `cleanup-test-agents.sh` | Reap agents and sockets left by killed Rust test runs. |
| `bench.sh --ci` | Rebuild the release agent, run `cargo bench` and enforce the Rust micro-benchmark budgets from `tests/performance-baselines.json` (part of `just bench-ci`). Exit 1 on an exceeded budget or a failed bench; exit 2 before benchmarking when `jq`, the baseline file or a budget key is missing. `BASELINE_FILE` points it at another budget file. |
| `ci-bench.sh` | End-to-end prompt-latency and agent-memory budgets (`just bench-ci`); same exit 2 and `BASELINE_FILE` rules as `bench.sh`. |
| `lib/bench-common.sh` | Sourced by both bench scripts: reads every budget in one `jq` call and rebuilds the release agent. |
| `perf-canary.sh` | Compare canary benchmarks against an upstream commit on the same machine. |
| `build-release-binaries.sh` | Build release binaries for every supported platform into `bin/`. |
| `check-glibc-floor.sh <binary> <floor>` | Fail when a Linux binary needs a `GLIBC_` version above `<floor>`. |
| `generate-sbom.sh` | CycloneDX SBOM from `Cargo.lock` (needs `jq`). |
| `gen-released-theme-hashes.sh [--git-dir DIR]...` | Rewrite `gpy-agent/src/theme/released_hashes.rs`, the SHA-256 of every released builtin theme that `gpy setup` retires unedited copies of; run at each release and after a builtin theme change (see `docs/dev/releasing.md`). |
| `package-release.sh` | Build the release archives and checksums; fails closed on any missing file. |
| `release-notes.sh` | Release notes from the CHANGELOG section and the built assets. |
| `smoke-release.sh` | Install and run the binaries a release is about to publish, in a sandboxed `HOME`. |
| `test-fresh-install.sh` | Install into a stock `ubuntu:24.04` container and render a prompt per shell. |
| `wsl-smoke.sh [--contract \| --contract-only]` | Inside WSL: font detection via `gpy setup`, `gpy-agent oneshot git` on Linux and Windows-drive repos, and optionally the cross-shell contract suite. `GPY_AGENT_BIN` and `GPY_WSL_DRVFS_DIR` override paths. |
| `install-hooks.sh [--uninstall]` | Install (or remove) the prek git hooks from `.pre-commit-config.yaml`. |
| `uninstall.sh` | Remove an installed GPY (every shell, Fish included). |
