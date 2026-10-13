# GPY Scripts

This directory contains utility scripts for development, testing, and quality assurance.

## Quality Check Script

`quality-check.sh` - Code quality gate for the Rust, Fish and shell code.

### Usage

```bash
# Run all quality checks (Fish + Rust)
./scripts/quality-check.sh

# Run fast checks only (skips slow integration tests and security audits)
./scripts/quality-check.sh --fast

# Auto-fix formatting issues
./scripts/quality-check.sh --fix

# Run only Rust checks
./scripts/quality-check.sh --rust-only

# Run only Fish checks
./scripts/quality-check.sh --fish-only

# Show help
./scripts/quality-check.sh --help
```

### What It Checks

**Fish Shell:**
- Syntax validation for all `.fish` files
- Formatting checks (if `fish_indent` is installed)
- Skips test framework files with special syntax

**Shell scripts:**
- `shellcheck` over every `scripts/*.sh`, `tests/bash/*.test.bash`, plus
  `install.sh` and `install-oneline.sh` (if `shellcheck` is installed).
  Discovered by glob, not by a hand-maintained list, so a new script is gated
  the moment it is added; the gate fails if the glob matches nothing.
- Run with `-x`, so `source`d files are followed rather than reported as
  SC1091. `install-oneline.sh` is checked as POSIX `sh`, since it runs under
  whatever `/bin/sh` the user has.
- Every file must be clean or carry an inline `# shellcheck disable=` with a
  reason on the line beneath it. Bare disables are not acceptable — the
  existing ones cover greps for literal `$VAR` text, mock functions that are
  shadowed rather than called, and variables assigned by a sourced file.

**Rust:**
- Code formatting (`cargo fmt`)
- Clippy lints with strict pedantic warnings
- Unit tests and doc tests
- Release build verification
- Security audit (`cargo-audit`) — mandatory, script fails if not installed
- Dependency/license checks (`cargo-deny`) — mandatory, script fails if not installed
- Unused dependency detection (if `cargo-machete` is installed)

### Requirements

**Required:**
- Rust toolchain (rustc, cargo, rustfmt, clippy)
- Fish shell
- `cargo-audit` - Security vulnerability scanning
- `cargo-deny` - License and dependency validation

**Optional (for additional checks):**
- `fish_indent` - For Fish formatting checks
- `cargo-machete` - Unused dependency detection
- `shellcheck` - Shell script lint (all of `scripts/`, `tests/bash/`, and the installers)
- `jq` - Required by `generate-sbom.sh`; the SBOM test skips without it

Install required security tools:
```bash
cargo install cargo-audit cargo-deny
```

Install all optional tools:
```bash
cargo install cargo-machete
```

### CI Integration

The `--fast` mode is recommended for CI environments as it skips slower checks while still validating core functionality:

```bash
./scripts/quality-check.sh --fast
```

### Exit Codes

- `0` - All checks passed
- `1` - One or more checks failed

### Notes

- The script automatically navigates to the project root
- Fish test files using `@test` syntax are automatically skipped
- Rust checks clear any `RUSTC_WRAPPER` environment override, so the wrapper configured in Cargo's config (e.g. kache) applies
- The script uses colors for better readability (can be piped to `less -R`)

## Other Scripts

| Script | Purpose |
|---|---|
| `test_fish.sh` | Run the Fish shell tests. |
| `bench.sh` | Run benchmarks; `--ci` checks the budgets in `tests/performance-baselines.json`. |
| `perf-canary.sh` | Compare focused canary benchmarks against the last pushed commit on the same machine. |
| `ci-bench.sh` | End-to-end prompt-latency and agent-memory budget benchmark (run via `just bench-ci`). |
| `check-active-toolchain.sh` | Fail unless the `rustc` active in `gpy-agent/` is the version `gpy-agent/rust-toolchain.toml` pins; names the override on a mismatch. CI jobs and the local Rust gates run it first. It always probes from `gpy-agent/`, because the toolchain file only governs its own directory. |
| `check-glibc-floor.sh <binary> <floor>` | Fail when a Linux binary references a `GLIBC_` symbol version above `<floor>`, naming the symbols. `release.yml` runs it against `GPY_GLIBC_FLOOR` (2.31). `OBJDUMP` selects the objdump. |
| `wsl-smoke.sh [--contract]` | Smoke run inside a WSL distribution with the debug agent built: checks WSL detection, font detection and `GPY_NERD_FONT`, and `gpy-agent oneshot git` on a Linux-filesystem and a Windows-drive repository. `--contract` also runs `tests/bash/shell_contract.test.bash`. `GPY_WSL_DRVFS_DIR` sets the Windows-drive directory, `GPY_AGENT_BIN` the agent binary. |
| `test-windows-vm.sh` | Run `cargo test --locked --lib` over SSH on a local Windows VM with the repo shared into it. Needs per-contributor setup; see below. |
| `install-hooks.sh [--uninstall]` | Install (or remove) prek shims for `pre-commit`, `pre-push` and `commit-msg` from `.pre-commit-config.yaml`. Run once per clone. |

### `test-windows-vm.sh`

Requires an SSH-reachable Windows VM (`~/.ssh/config` host alias, default `gpy-winvm`) with Rust and the MSVC Build Tools installed, and the repo shared into the VM (default `\\Mac\gpy`). Override with `GPY_WINVM_HOST`, `GPY_WINVM_PATH` and `GPY_WINVM_TARGET_DIR`.

It approximates the `windows-latest` CI runner but is not that runner:

- It clears `HOME`, `XDG_CONFIG_HOME` and `XDG_CACHE_HOME`, because Windows OpenSSH sets `HOME` and the runner has none. `GPY_WINVM_KEEP_HOME=1` keeps them.
- A developer VM usually has a real Git Bash, which the `theme::export` tests source; the script prints what `bash` resolves to.
- A Parallels VM on Apple Silicon is `aarch64-pc-windows-msvc`; `windows-latest` is x86_64.

A green local run is evidence, not proof; check the CI leg too.
