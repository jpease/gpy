# GPY Scripts

This directory contains utility scripts for development, testing, and quality assurance.

## Quality Check Script

`quality-check.sh` - Comprehensive code quality checks for both Rust and Fish code.

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

- `test_fish.sh` - Run Fish shell tests
- `test-windows-vm.sh` - Build and run `cargo test --locked --lib` on Windows over SSH, against a local Windows VM (e.g. Parallels) with the repo reachable via a shared folder. Mirrors the `windows-latest` leg of `cross-platform-test.yml` for local iteration without round-tripping through CI. Requires per-contributor setup: an SSH-reachable Windows VM (`~/.ssh/config` host alias, default `gpy-winvm`) with Rust + MSVC Build Tools installed, and the repo shared into the VM (default path `\\Mac\gpy`). Override via `GPY_WINVM_HOST`, `GPY_WINVM_PATH`, `GPY_WINVM_TARGET_DIR` env vars. Not available out of the box for every contributor.

  **What it cannot reproduce.** It approximates `windows-latest`; it is not that runner. Known differences, measured in #513:

  - It clears `HOME`, `XDG_CONFIG_HOME`, and `XDG_CACHE_HOME`, because Windows OpenSSH sets `HOME` in the session and the runner has none. Set `GPY_WINVM_KEEP_HOME=1` to keep them and exercise the shape a real interactive native-Windows user gets.
  - A developer VM carries an installed toolchain — notably a real Git Bash, which the `theme::export` tests source. The runner resolved a `bash` that failed there (#528), so the script prints what `bash` resolves to before running.
  - A Parallels VM on Apple Silicon is `aarch64-pc-windows-msvc`; `windows-latest` is x86_64.

  A green local run is evidence, not proof. Before concluding a Windows failure is fixed, check the CI leg too.
- `check-active-toolchain.sh` - Assert that the `rustc` active **in `gpy-agent/`** is the version `gpy-agent/rust-toolchain.toml` pins. Every CI job that builds or lints runs it, because `actions-rust-lang/setup-rust-toolchain` falls back to `stable` silently when it cannot find the toolchain file — a moved crate or a dropped `rust-src-dir:` input would restore the #518 bug with every job still green. Also useful locally to catch a stray `RUSTUP_TOOLCHAIN` override. On a mismatch it names the override (the `RUSTUP_TOOLCHAIN` value, else rustup's report). Locally, `just clippy-strict`, `just lint`, the pre-commit `rustfmt` hook and every Rust mode of `quality-check.sh` (full, `--fix`, `--fast`, `--rust-only`) run it first and abort before any cargo command (#823). The directory matters and the script enforces it regardless of where you invoke it from: a toolchain file governs its own directory and those below it, so from the repository root `rustc` reports the machine's default instead of the pin. Probing there made the macOS gate red and the Linux and Windows gates green purely on which version those runner images default to (#538); `tests/bash/toolchain_pin_scope.test.bash` holds the probe in place.
- `check-glibc-floor.sh <binary> <floor>` - Fail when a Linux binary's `objdump -T` references a `GLIBC_` symbol version above the floor, and name the symbols that do. `release.yml` runs it on both Linux binaries of each arch against `GPY_GLIBC_FLOOR` (2.31, the Ubuntu 20.04 / Debian 11 glibc) before uploading them (#694). Runs on any Linux build; Ubuntu's stock objdump reads both x86_64 and aarch64 binaries, and `OBJDUMP` overrides which objdump is used.
- `bench.sh` - Run benchmarks
- `perf-canary.sh` - Compare focused canary benchmarks against the last pushed commit on the same machine
- `ci-bench.sh` - End-to-end prompt-latency and agent-memory budget benchmark (run via `just bench-ci`)
- `install-hooks.sh` - Configure git to use the repo-provided hooks (pre-push runs the quality check and a same-machine perf canary). Run once per clone:
  ```bash
  ./scripts/install-hooks.sh          # install
  ./scripts/install-hooks.sh --uninstall  # remove
  ```
