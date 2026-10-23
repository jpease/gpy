# Contributing to GPY

GPY is a personal project, maintained by one person in spare time. Review is
best-effort with no fixed timeline — a PR may sit for a while before anyone
looks at it. That is not a judgment on the change.

Bug reports are genuinely wanted, especially from a platform the maintainer
cannot test directly: a Linux distribution, an older bash, Windows. If your
prompt broke, file it — see [Filing a Bug Report](#filing-a-bug-report).

For anything bigger than a bug fix — a new feature, a behavior change, a
refactor — open an issue first and wait for a response before writing code.
That avoids spending time on a change that would not get merged.

## Ground Rules

- Be respectful and follow the [Code of Conduct](CODE_OF_CONDUCT.md).
- Use GitHub Issues for planning and tracking work.
- Keep changes small, focused, and reviewable.
- Include tests and documentation updates with behavior changes.
- Security issues never go through a public issue or PR — see
  [Reporting a Security Issue](#reporting-a-security-issue).

## Prerequisites

- Rust and Cargo, matching the edition pinned in `gpy-agent/Cargo.toml`
  (currently `edition = "2024"`). The exact toolchain comes from
  `gpy-agent/rust-toolchain.toml`, which `rustup` picks up automatically —
  don't override it with `RUSTUP_TOOLCHAIN`. The pin exists because Clippy's
  pedantic lint set changes between releases, so an unpinned toolchain gives
  you a different lint set than CI. CI resolves the same file, and every job
  asserts it did
- [`just`](https://github.com/casey/just), the recipe runner the commands
  below use
- [`cargo nextest`](https://nexte.st/) — `cargo install cargo-nextest` (or
  `brew install cargo-nextest`); the Rust test suite runs through it
- Fish, to run `./install-dev.fish` and the Fish test suite; see
  [Supported Platforms and Shells](docs/INSTALL.md#supported-platforms-and-shells)
  for the full OS/shell support matrix — that page is canonical, this file
  does not restate version numbers

None of this requires any AI or agent tooling. If you use an AI coding
assistant, that is your own workflow choice; it is not part of the project's
contribution requirements and nothing here depends on it.

## Workflow

1. Fork the repository and clone your fork.
2. Open or select an issue (skip this for a small, obvious fix).
3. Create a branch from `main`.
4. Implement the change with tests.
5. Run the quality gates (below).
6. Open a PR linked to the issue, from your fork's branch.

## Development Setup

```bash
git clone https://github.com/<you>/gpy.git
cd gpy
./install-dev.fish
```

`./install-dev.fish` installs into `~/.local/bin` and `~/.config`, so it refuses
to run when a packaged gpy (for example Homebrew's) is already on `PATH`; pass
`--allow-shadow` to override. To try a build without touching an installed
release, run it in an isolated shell instead:

```bash
./scripts/gpy-dev.sh                 # this checkout
./scripts/gpy-dev.sh --pr 123        # a pull request
./scripts/gpy-dev.sh --ref v0.7.0-rc1
./scripts/gpy-dev.sh --bin ./dist    # prebuilt gpy + gpy-agent, no build
```

`exit` returns to the installed release. See
[`scripts/README.md`](scripts/README.md) for the flags.

## Quality Gates

Run before pushing — this is what CI also runs:

```bash
./scripts/quality-check.sh
```

It covers: Fish syntax and formatting, Rust formatting, strict Clippy, Rust
unit/integration tests, doc tests, `cargo audit` and `cargo deny`, an
unused-dependency check, shellcheck over every shell script in the repository,
and (in CI, optional locally) a release build.

Shell scripts must be shellcheck-clean or carry an inline
`# shellcheck disable=` with a reason on the line beneath it. A bare disable
will be asked about in review.

macOS never compiles the Linux- or Windows-only `cfg` branches. Before pushing
a change that touches `cfg`-gated code, run `just check-platforms` (needs
Docker): it runs Linux `cargo clippy --all-targets --all-features` and Windows
(mingw) `cargo check` for `--lib`, `--locked` and `--profile test --tests` on
`HEAD` in a container. Pass `--worktree` to include uncommitted changes.

Lint policy lives in the `[lints]` tables of `gpy-agent/Cargo.toml`. Every
entry is `warn` and `warnings = "deny"` turns each into an error. They cover
the `clippy::pedantic` group plus restriction and nursery lints in five
groups: panics and stray output (`unwrap_used`, `expect_used`, `panic`,
`print_stdout`, `exit`, ...), documented and isolated `unsafe` (`unsafe_code`,
`undocumented_unsafe_blocks`), lost information (`map_err_ignore`,
`use_debug`, ...), numeric conversions, arithmetic and indexing
(`as_conversions`, `arithmetic_side_effects`, `indexing_slicing`), and
readability (`shadow_*`, `str_to_string`, `missing_const_for_fn`, ...).
`gpy-agent/.clippy.toml` relaxes the panic and print lints in test code and
forbids `std::panic::catch_unwind`. Run it with `just lint`; while fixing many
hits at once, `cargo clippy --all-targets --all-features -- --cap-lints warn`
reports every target instead of stopping at the first failing one.

A new `#[allow(...)]` must say why, with `reason = "..."` naming the specific
cause at that site (`#[allow(clippy::cast_possible_truncation, reason = "value
is clamped to u16::MAX above")]`). `scripts/check_new_allows.py` checks only
the lines a change adds: the `new-allows` pre-commit hook runs it on staged
changes and the `new-allows` CI job on the pull request's diff. Allows that
already exist are not back-filled.

A Rust source file stays under 800 lines, not counting its inline test
modules (`scripts/check_file_size.py`, run by the `file-size` hook and
`quality-check.sh`). The files already over the cap are listed with their
size in `scripts/file-size-allowlist.txt` and may not grow. When you work in a
listed file, split it, and lower or remove its entry with
`python3 scripts/check_file_size.py --update` in the same commit; the check
fails until a shrunk file's entry is lowered.

Faster subsets for local iteration:

```bash
./scripts/quality-check.sh --fast        # skip slow audits and the release build
./scripts/quality-check.sh --rust-only   # Rust toolchain checks only
./scripts/quality-check.sh --shell-only  # Fish/Bash/Zsh integration suites only
./scripts/quality-check.sh --shell-only bash  # one part: checks, fish, bash or zsh (comma-separated)
./scripts/quality-check.sh --fish-only   # Fish syntax + formatting only
```

Equivalent `just` recipes:

```bash
just check          # same as ./scripts/quality-check.sh
just check-fast
just check-rust
just check-shell
just check-fish
just build           # cargo build
just test-rust       # cargo nextest run --features test-support
just test-fish       # ./scripts/test_fish.sh
just lint            # cargo clippy --all-targets --all-features
just format          # cargo fmt
just fmt-check       # cargo fmt --check
```

### Running individual suites

```bash
# Rust tests (unit + integration)
(cd gpy-agent && cargo nextest run)
(cd gpy-agent && cargo nextest run --test <suite_name>)
(cd gpy-agent && cargo nextest run --lib)   # unit tests only
(cd gpy-agent && cargo test --doc)          # doc tests

# Fish integration tests (auto-discovers tests/fish/*.test.fish)
./scripts/test_fish.sh
fish tests/fish/<name>.test.fish            # a single suite

# Bash and Zsh integration tests (auto-discovered by quality-check.sh)
bash tests/bash/<name>.test.bash
zsh tests/zsh/<name>.test.zsh

# Fish syntax check
fish -n install-dev.fish fish/**/*.fish tests/fish/*.test.fish

# Fish formatting check
fish_indent --check install-dev.fish fish/**/*.fish

# Rust lint (uses the [lints] policy in Cargo.toml)
(cd gpy-agent && cargo clippy --all-targets --all-features)

# Rust formatting
(cd gpy-agent && cargo fmt --all)           # auto-fix
(cd gpy-agent && cargo fmt --all --check)   # check only
```

`*.bak` files are gitignored: leave a scratch copy of a test next to the
original if you like, but nothing will pick it up, and a stale `.rs.bak`
under `gpy-agent/tests/` is invisible to nextest (it discovers `*.rs` only).
Every test tier and the command that runs it is in
[docs/dev/testing.md](docs/dev/testing.md).

### Git hooks (optional, recommended)

```bash
./scripts/install-hooks.sh
```

Installs `prek` shims so `git commit` and `git push` run the relevant checks
automatically: `pre-commit` runs fast, file-type-scoped checks (the `clippy`
hook, rustfmt, reasons on new allows, Fish syntax/formatting, secret scan);
`pre-push` repeats the Clippy pass. The heavy gates (the Rust toolchain gate, the
Fish/Bash/Zsh suites and the performance job) run in CI on every pull request;
run `just check-rust`, `just check-shell` or `just perf-canary` yourself when
you want them locally. `.pre-commit-config.yaml` defines the hooks.

## Filing a Bug Report

Use the bug report issue template. It asks for the four things needed to
reproduce almost any prompt issue: your shell and its version, your OS,
`gpy --version`, and whether the agent is running (`gpy status`). A report
missing those will likely get a comment asking for them, which costs a round
trip — include them up front if you can.

## Commit Conventions

Commits follow [Conventional Commits](https://www.conventionalcommits.org/)
(`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`, and so on).

- Prefer rebase over merge commits, and squash fixup commits before pushing.
- Keep the commit body brief; link a related GitHub issue or pull request for context rather
  than repeating the whole discussion.

## Pull Request Expectations

- Clear title and description of what changed and why.
- Linked issue, for context and tracking (see [Workflow](#workflow) above for
  when an issue is expected before a PR).
- Passing CI and the quality gates above.
- Tests for behavior changes; updated docs for user-visible or
  developer-facing changes.
- Use the PR template's checklist — most of it should already be true from
  running the quality gates before opening the PR.

## Reporting a Security Issue

Do not open a public issue or PR for a security vulnerability. Use
[GitHub private vulnerability reporting](https://github.com/jpease/gpy/security/advisories/new).
See [`SECURITY.md`](SECURITY.md) for what to include and what to expect.

## Canonical Developer References

Detailed technical and architecture docs live in [`docs/dev/`](docs/dev/README.md):

- Architecture and design rationale
- Segment and plugin development guides
- Theme and plugin authoring guides
- Performance budgets and benchmarking references

## License

By contributing, you agree your contributions are licensed under the project
license in [LICENSE](LICENSE).
