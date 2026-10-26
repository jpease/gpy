# Contributing to GPY

GPY is a personal project, maintained by one person in spare time. Review is
best-effort with no fixed timeline — a PR may sit for a while before anyone
looks at it. That is not a judgment on the change.

Bug reports are wanted, especially from a platform the maintainer
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
`--allow-shadow` to override.

To try a build without touching an installed release, run it in an isolated shell
instead with `just dev` (or `./scripts/gpy-dev.sh`); `exit` returns to the
installed release. See [`scripts/README.md`](scripts/README.md) for the flags.

## Quality Gates

Run before pushing — this is what CI also runs:

```bash
./scripts/quality-check.sh
```

See [`scripts/README.md`](scripts/README.md) for the check list, required tools,
and faster flags (`--fast`, `--rust-only`, `--clippy`, `--shell-only`,
`--fish-only`).
`just check` (or `just check-fast`, `just check-rust`, `just check-shell`,
`just check-fish`) wraps these.
Shell scripts must be shellcheck-clean or carry an inline
`# shellcheck disable=` with a reason on the line beneath it. A bare disable
will be asked about in review.

macOS never compiles the Linux- or Windows-only `cfg` branches. Before pushing
a change that touches `cfg`-gated code, run `just check-platforms` (needs
Docker): it runs Linux `cargo clippy` (the same two passes as `just lint`) and Windows
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
forbids `std::panic::catch_unwind`. Run it with `just lint`, which is what the
gate and CI run: `cargo clippy --all-targets` (tests and benches, with the
`test-support` surface their self dev-dependency enables), then
`cargo clippy --lib --bins` (the code as it ships, without it). While fixing many
hits at once, add `-- --cap-lints warn` to either to report every target instead
of stopping at the first failing one.

A new `#[allow(...)]` must say why, with `reason = "..."` naming the specific
cause at that site (`#[allow(clippy::cast_possible_truncation, reason = "value
is clamped to u16::MAX above")]`). `scripts/check-new-allows.py` checks only
the lines a change adds: the `new-allows` pre-commit hook runs it on staged
changes and the `new-allows` CI job on the pull request's diff. Allows that
already exist are not back-filled.

A Rust source file stays under 800 lines, not counting its inline test
modules (`scripts/check-file-size.py`, run by the `file-size` hook and
`quality-check.sh`). The files already over the cap are listed with their
size in `scripts/file-size-allowlist.txt` and may not grow. When you work in a
listed file, split it, and lower or remove its entry with
`python3 scripts/check-file-size.py --update` in the same commit; the check
fails until a shrunk file's entry is lowered.

For faster iterations and individual test suites, see [`scripts/README.md`](scripts/README.md)
and [Testing Tiers](docs/dev/testing.md).
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
