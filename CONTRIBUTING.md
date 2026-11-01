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
- [`yq`](https://github.com/mikefarah/yq) (mikefarah, ≥ 4.52.4; `brew install
  yq`, not Debian's `apt install yq`), which the `tests/repo` invariant tests
  use to parse workflow and TOML files
- [`cargo nextest`](https://nexte.st/) — `cargo install cargo-nextest` (or
  `brew install cargo-nextest`); the Rust test suite runs through it
- Fish, Zsh, and Bash 4 or newer (macOS's `/bin/bash` is 3.2; `brew install
  bash`), to run every shell suite. `just check` fails when one is missing
  (`just check-rust` needs none). The Bash 4 minimum is for the dev scripts
  only; the supported shell versions for users are on
  [Supported Platforms and Shells](docs/INSTALL.md#supported-platforms-and-shells),
  which is canonical, and this file does not restate them.

## Workflow

1. Fork the repository and clone your fork.
2. Open or select an issue (skip this for a small, obvious fix).
3. Create a branch from `main`.
4. Implement the change with tests.
5. Run the quality gates (below).
6. Open a PR linked to the issue, from your fork's branch.

## Development Setup

The same steps work for Fish, Zsh and Bash: clone, build, install both
binaries and run `gpy setup --from .`, as in
[Building from Source](docs/INSTALL.md#building-from-source).

```bash
git clone https://github.com/<you>/gpy.git
cd gpy
(cd gpy-agent && cargo build --profile release-dist)
mkdir -p ~/.local/bin
install -m 755 gpy-agent/target/release-dist/gpy-agent gpy-agent/target/release-dist/gpy ~/.local/bin/
~/.local/bin/gpy setup --from .
```

Optional, Fish only:

- `just install-dev`: rebuild, install and check hot reload; needs Fish.
- `just dev`: an isolated Fish shell running your build; `exit` returns to your
  install. See [`scripts/README.md`](scripts/README.md) for the flags.

`./install-dev.fish` installs into `~/.local/bin` and `~/.config`, so it refuses
to run when a packaged gpy (for example Homebrew's) is already on `PATH`; pass
`--allow-shadow` to override. It also warns about other `gpy` copies earlier on `PATH`; it never deletes them.

## Quality Gates

Before pushing, run `just check`: the full gate, strict about missing optional
tools as CI is (`GPY_GATE_STRICT=1`). `just check-fast` is
the quick loop. Run `just check-platforms` before a change to `cfg`-gated code
(needs Docker).

Lint policy: the `[lints]` tables in `gpy-agent/Cargo.toml`; run it with
`just lint`. File-size and new-`allow` rules:
[`scripts/README.md`](scripts/README.md). The 800-line file cap counts test
files too (`gpy-agent/tests/` targets, `tests.rs` and `*_tests.rs`); the ones
still over it are on the allowlist, and new tests for their code go in a new
file.

Shell scripts must be shellcheck-clean or carry an inline
`# shellcheck disable=` with a reason on the line beneath it. A bare disable
will be asked about in review.

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
