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
  don't override it with `RUSTUP_TOOLCHAIN`. The pin exists because the strict
  Clippy gate enables pedantic and nursery lints whose membership changes
  between releases, so an unpinned toolchain gives you a different lint set
  than CI. CI resolves the same file, and every job asserts it did
- [`just`](https://github.com/casey/just), the recipe runner the commands
  below use
- [`cargo nextest`](https://nexte.st/) — `cargo install cargo-nextest` (or
  `brew install cargo-nextest`); the Rust test suite runs through it
- Fish, to run `./install-dev.fish` and the Fish test suite; see
  [Supported Platforms and Shells](docs/INSTALL.md#supported-platforms-and-shells)
  for the full OS/shell support matrix — that page is canonical, this file
  does not restate version numbers
- [`moon`](https://moonrepo.dev/) if you use the `just build` / `just
  test-rust` / `just lint` recipes, which proxy through it; `moon` is not
  needed to run `./scripts/quality-check.sh` directly

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

## Quality Gates

Run before pushing — this is what CI also runs:

```bash
./scripts/quality-check.sh
```

It covers: Fish syntax and formatting, Rust formatting, strict Clippy, Rust
unit/integration tests, doc tests, `cargo audit` and `cargo deny`, an
unused-dependency check, shellcheck over every shell script in the repository,
and (in CI and pre-push, optional locally) a release build.

Shell scripts must be shellcheck-clean or carry an inline
`# shellcheck disable=` with a reason on the line beneath it. A bare disable
will be asked about in review.

Clippy also enforces function-level complexity limits via
`gpy-agent/.clippy.toml` (`cognitive-complexity-threshold`,
`too-many-lines-threshold`, `too-many-arguments-threshold`, among others). If
a function trips one, prefer early returns or extracting a helper over
raising the threshold.

Faster subsets for local iteration:

```bash
./scripts/quality-check.sh --fast        # skip slow audits and the release build
./scripts/quality-check.sh --rust-only   # Rust toolchain checks only
./scripts/quality-check.sh --shell-only  # Fish/Bash/Zsh integration suites only
./scripts/quality-check.sh --fish-only   # Fish syntax + formatting only
```

Equivalent `just` recipes, if you have `just` and `moon` installed:

```bash
just check          # same as ./scripts/quality-check.sh
just check-fast
just check-rust
just check-shell
just check-fish
just build           # moon run gpy-agent:build
just test-rust       # moon run gpy-agent:test (cargo nextest)
just test-fish       # ./scripts/test_fish.sh
just lint            # strict clippy: moon run gpy-agent:clippy
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

# Bash and Zsh integration tests (auto-discovered by quality-check.sh;
# tests/bash/README.md and tests/zsh/README.md list every file and which
# are repo-policy checks rather than shell tests)
bash tests/bash/<name>.test.bash
zsh tests/zsh/<name>.test.zsh

# Fish syntax check
fish -n install-dev.fish fish/**/*.fish tests/fish/*.test.fish

# Fish formatting check
fish_indent --check install-dev.fish fish/**/*.fish

# Rust lint (strict, zero warnings)
(cd gpy-agent && cargo clippy --all-targets -- -D warnings)

# Rust formatting
(cd gpy-agent && cargo fmt --all)           # auto-fix
(cd gpy-agent && cargo fmt --all --check)   # check only
```

`cargo`, `nextest`, and `clippy` invocations in this repo sometimes need
`RUSTC_WRAPPER=""` set first — the `just` recipes already do this for you.

`*.bak` files are gitignored: leave a scratch copy of a test next to the
original if you like, but nothing will pick it up, and a stale `.rs.bak`
under `gpy-agent/tests/` is invisible to nextest (it discovers `*.rs` only).
Every test tier and the command that runs it is in
[docs/dev/testing.md](docs/dev/testing.md).

### Git hooks (optional, recommended)

```bash
./scripts/install-hooks.sh
```

Installs `prek` hooks so `git commit` and `git push` run the relevant checks
automatically: `pre-commit` runs fast, file-type-scoped checks (Clippy,
rustfmt, Fish syntax/formatting); `pre-push` runs the shell suites always, and
the Rust suite too when Rust-relevant files changed. `.raven/git-hooks/`
holds the hook scripts.

## Filing a Bug Report

Use the bug report issue template. It asks for the four things needed to
reproduce almost any prompt issue: your shell and its version, your OS,
`gpy --version`, and whether the agent is running (`gpy status`). A report
missing those will likely get a comment asking for them, which costs a round
trip — include them up front if you can.

## Commit Conventions

Commits follow [Conventional Commits](https://www.conventionalcommits.org/)
(`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`, and so on).

- No AI or agent attribution: do not mention "Claude", "AI", "generated by",
  or similar in a commit message. `.raven/git-hooks/` enforces this on commit.
- Prefer rebase over merge commits, and squash fixup commits before pushing.
- Keep the commit body brief; reference the issue number for context rather
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
