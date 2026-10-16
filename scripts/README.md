# GPY Scripts

Development, release and quality scripts. Most are wrapped by a `just` recipe
(`just --list`).

## quality-check.sh

The local quality gate.

```bash
./scripts/quality-check.sh               # full gate
./scripts/quality-check.sh --fast        # repo checks, Fish syntax, shellcheck, parity tests, Rust lint and tests
./scripts/quality-check.sh --rust-only   # fmt, clippy, tests, doc tests, audit, deny, release build
./scripts/quality-check.sh --shell-only  # Fish lint, shellcheck, Fish/Bash/Zsh suites
./scripts/quality-check.sh --fish-only   # Fish syntax and formatting
./scripts/quality-check.sh --fix         # fish_indent, cargo fmt, clippy --fix, then the full gate
```

It checks:

- **Repository:** license metadata, privacy patterns, documentation links.
- **Fish:** syntax of every `.fish` file; formatting with `fish_indent`.
- **Shell scripts:** `shellcheck -x` over the installers, `scripts/*.sh`,
  `tests/bash/*.test.bash`, `benchmarks/*/*.sh` and the shipped `bash/` tree,
  discovered by glob. `install-oneline.sh` is checked as POSIX `sh`. An inline
  `# shellcheck disable=` needs a reason.
- **Shell suites:** Fish, Bash and Zsh tests against this checkout's debug
  agent, each Bash/Zsh file in its own XDG root. The run fails if a suite
  changes this repository's HEAD or worktrees.
- **Rust:** `cargo fmt`, clippy (policy in `gpy-agent/Cargo.toml` `[lints]`),
  nextest, doc tests, `cargo audit`, `cargo deny`, `cargo machete` if
  installed, and a release build (always with `--rust-only`; with the full gate
  only under CI or `GPY_GATE_RELEASE=1`).

Required: the pinned Rust toolchain with clippy and rustfmt, `cargo-nextest`,
`fish`, `cargo-audit` and `cargo-deny`. Optional: `fish_indent`, `shellcheck`,
`cargo-machete`, `zsh`. A missing optional tool prints `SKIP:`; under CI a
skip is a failure.

Exit status is 0 when every check passed, 1 otherwise.

## Other scripts

| Script | Purpose |
|---|---|
| `test_fish.sh` | Build the debug binaries and run `tests/fish/*.test.fish`. |
| `check-active-toolchain.sh` | Fail unless the `rustc` active in `gpy-agent/` is the `rust-toolchain.toml` pin; names the override on a mismatch. |
| `check-license-metadata.sh` | Cargo, Fisher, Homebrew and release metadata agree on the license. |
| `check-privacy-patterns.sh` | No workstation paths, private identifiers or hardcoded sockets in tracked files. |
| `check-doc-links.sh` | Every repository-relative Markdown link resolves. |
| `cleanup-test-agents.sh` | Reap agents and sockets left by killed Rust test runs. |
| `bench.sh` | Rust micro-benchmarks; `--ci` enforces `tests/performance-baselines.json`. |
| `ci-bench.sh` | End-to-end prompt-latency and agent-memory budgets (`just bench-ci`). |
| `perf-canary.sh` | Compare canary benchmarks against an upstream commit on the same machine. |
| `perf-baseline.sh` | Establish or compare performance baselines (`just bench-baseline`). |
| `build-release-binaries.sh` | Build release binaries for every supported platform into `bin/`. |
| `check-glibc-floor.sh <binary> <floor>` | Fail when a Linux binary needs a `GLIBC_` version above `<floor>`. |
| `generate-sbom.sh` | CycloneDX SBOM from `Cargo.lock` (needs `jq`). |
| `package-release.sh` | Build the release archives and checksums; fails closed on any missing file. |
| `release-notes.sh` | Release notes from the CHANGELOG section and the built assets. |
| `smoke-release.sh` | Install and run the binaries a release is about to publish, in a sandboxed `HOME`. |
| `test-fresh-install.sh` | Install into a stock `ubuntu:24.04` container and render a prompt per shell. |
| `wsl-smoke.sh [--contract \| --contract-only]` | Inside WSL: font detection via `gpy setup`, `gpy-agent oneshot git` on Linux and Windows-drive repos, and optionally the cross-shell contract suite. `GPY_AGENT_BIN` and `GPY_WSL_DRVFS_DIR` override paths. |
| `test-windows-vm.sh` | Run `cargo test --locked --lib` over SSH on a local Windows VM; see below. |
| `install-hooks.sh [--uninstall]` | Install (or remove) the prek git hooks from `.pre-commit-config.yaml`. |
| `uninstall.sh`, `uninstall.fish` | Remove an installed GPY. |
| `update.fish` | Update the installed Fish scripts and config from this checkout. |

### test-windows-vm.sh

Requires an SSH-reachable Windows VM (`~/.ssh/config` host alias, default
`gpy-winvm`) with Rust and the MSVC Build Tools, and the repo shared into it
(default `\\Mac\gpy`). Override with `GPY_WINVM_HOST`, `GPY_WINVM_PATH` and
`GPY_WINVM_TARGET_DIR`.

It approximates the `windows-latest` runner but is not that runner:

- It clears `HOME`, `XDG_CONFIG_HOME` and `XDG_CACHE_HOME`, because Windows
  OpenSSH sets `HOME` and the runner has none. `GPY_WINVM_KEEP_HOME=1` keeps them.
- A developer VM usually has a real Git Bash, which the `theme::export` tests
  source; the script prints what `bash` resolves to.
- A Parallels VM on Apple Silicon is `aarch64-pc-windows-msvc`; `windows-latest`
  is x86_64.
