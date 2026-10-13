# GPY Dependency Audit

The register of GPY's dependency advisories and duplicate crate versions.
`gpy-agent/deny.toml` holds the machine-readable lists that cargo-deny enforces;
the comment block in `gpy-agent/src/lib.rs` repeats the duplicate-version table
next to the `#![allow(clippy::multiple_crate_versions)]` that depends on it.
`tests/bash/deny_skip_list.test.bash` checks that every crate in `deny.toml`'s
`[bans] skip` list is still duplicated in `Cargo.lock`.

**Audited:** 2026-09-01 against `gpy-agent/Cargo.lock` (319 crate dependencies)
**Next review:** 2027-03-01
**Tools:** `cargo audit` 0.22.2, `cargo deny` 0.20.2

## Current state

`cargo audit` reports no advisories against `gpy-agent/Cargo.lock`, and
`gpy-agent/deny.toml`'s `[advisories] ignore` list is empty.

`gpy-agent/deny.toml` also sets `[advisories] unsound = "all"` and
`unmaintained = "all"`. cargo-deny 0.20 skips the `unsound` class by default; these
two lines make a future advisory in either class fail the check instead of passing
unreviewed.

## Review triggers

Re-run this audit on the earliest of:

- 2027-03-01, the scheduled date
- a new advisory reported by `cargo audit` against this lockfile
- a change that adds a dependency, since that can change which crates in the
  duplicate-version table below are still accurate

## Duplicate dependencies

`cargo tree --duplicates` shows 15 crates with more than one version in
`gpy-agent/Cargo.lock`. `gpy-agent/deny.toml`'s `[bans] skip` and the table in
`gpy-agent/src/lib.rs` cover three of them, the ones `cargo deny check bans` needs a
skip for. The other 12 (bit-set, bit-vec, cfg_aliases, hashbrown, itertools, nix,
r-efi, rand, rand_core, syn, thiserror, thiserror-impl) pass `cargo deny check bans`
without a skip, mostly because the second copy is used only in tests or benches
(proptest, criterion, portable-pty) or arrives through an optional `ratatui` backend
(`ratatui-termwiz`) this crate never turns on, and cargo-deny's bans graph does not
count those edges. `bitflags` and `getrandom` have the same shape but stay in `skip`
because they are still duplicated in `Cargo.lock`.

| crate | versions | why |
|---|---|---|
| bitflags | 1.3.2, 2.13.1 | 1.3.2 used only for the Unix `test-support` PTY test (portable-pty 0.9.0); 2.13.1 under crossterm, notify, nix and the ratatui workspace |
| getrandom | 0.3.4, 0.4.3 | both used only in tests: 0.3.4 under rand_core 0.9.5, via proptest; 0.4.3 under tempfile, via insta, proptest and rusty-fork |
| windows-sys | 0.60.2, 0.61.2 | 0.60.2 under notify; 0.61.2 under rustix, mio, socket2, clap's anstream/anstyle-wincon, and ignore/walkdir's winapi-util |

Impact is a slightly larger binary. No advisory is caused by duplication.

Reproduce with:

```bash
(cd gpy-agent && cargo tree --duplicates)
(cd gpy-agent && cargo tree -i <crate>[@version] -e normal,build,dev --target all)
```

## Out of scope

- Vulnerability reporting. See `SECURITY.md`.
- License and source policy. Enforced by `cargo deny check licenses sources`; the
  allowed set is in `gpy-agent/deny.toml`.

## Verification

```bash
(cd gpy-agent && cargo audit)
(cd gpy-agent && cargo deny check advisories licenses bans sources)
bash tests/bash/deny_skip_list.test.bash
```
