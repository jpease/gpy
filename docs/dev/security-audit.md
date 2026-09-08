# GPY Dependency Audit

This is the canonical register for GPY's dependency advisories and duplicate crate
versions. `gpy-agent/deny.toml` holds the machine-readable lists that cargo-deny
enforces; the comment block in `gpy-agent/src/lib.rs` repeats the duplicate-version
table next to the `#![allow(clippy::multiple_crate_versions)]` that depends on it.
`tests/bash/dependency_advisory_claims.test.bash` derives the advisory list from
`cargo audit` and fails if the registers disagree, if a claim here becomes false, or
if the review date below passes.

**Audited:** 2026-09-01 against `gpy-agent/Cargo.lock` (319 crate dependencies)
**Next review:** 2027-03-01
**Tools:** `cargo audit` 0.22.2, `cargo deny` 0.20.2
**Status:** zero advisories; `cargo audit` and `cargo deny check advisories licenses
bans sources` both exit 0

## Current state

As of 2026-09-01, `cargo audit` reports no advisories against `gpy-agent/Cargo.lock`,
and `gpy-agent/deny.toml`'s `[advisories] ignore` list is empty. The five advisories
this register used to track are gone because hyperpolyglot 0.1.7 -- the language
detector that carried all five, last published 2020-07-26 -- is gone: #523 replaced
it with `gengo-language`'s tables, and #525 removed the now-empty waiver that named
it. See History below for the retired advisory IDs and the reachability finding that
justified waiving them at the time.

`gpy-agent/deny.toml` separately sets `[advisories] unsound = "all"` and
`unmaintained = "all"`. cargo-deny 0.20 skips the `unsound` class by default; these
two lines are what makes a future advisory in either class fail the check instead of
passing unreviewed, independent of anything being ignored right now.

## Review triggers

Re-run this audit on the earliest of:

- 2027-03-01, the scheduled date
- a new advisory reported by `cargo audit` against this lockfile
- a change that adds a dependency, since that can shift which crates in the
  duplicate-version table below are still accurate

Nothing here is waiting on anything outside the project.

## Duplicate dependencies

`cargo tree --duplicates` shows 15 crates with more than one version in
`gpy-agent/Cargo.lock` as of 2026-09-01. `gpy-agent/deny.toml`'s `[bans] skip` and the
table in `gpy-agent/src/lib.rs` cover three of them — the ones `cargo deny check bans`
still needs a skip for. The other 12 (bit-set, bit-vec, cfg_aliases, hashbrown,
itertools, nix, r-efi, rand, rand_core, syn, thiserror, thiserror-impl) duplicate too,
but `cargo deny check bans` passes for them without a skip: most of the time because
the second copy is used only in tests or benches (proptest, criterion, portable-pty)
or arrives through an optional `ratatui` backend (`ratatui-termwiz`) this crate never
turns on, and cargo-deny's bans graph does not count those edges. `bitflags` and
`getrandom` are the same shape — a copy used only in tests, or gated behind a feature
this crate never enables — but stay in `skip` because they are still genuinely
duplicated in `Cargo.lock`, which is what
`tests/bash/dependency_advisory_claims.test.bash` checks directly.

| crate | versions | why |
|---|---|---|
| bitflags | 1.3.2, 2.13.1 | 1.3.2 used only for the Unix `test-support` PTY test (portable-pty 0.9.0); 2.13.1 under crossterm, notify, nix and the ratatui workspace |
| getrandom | 0.3.4, 0.4.3 | both used only in tests: 0.3.4 under rand_core 0.9.5, via proptest; 0.4.3 under tempfile, via insta, proptest and rusty-fork |
| windows-sys | 0.60.2, 0.61.2 | 0.60.2 under notify; 0.61.2 under rustix, mio, socket2, clap's anstream/anstyle-wincon, and ignore/walkdir's winapi-util |

Impact is a slightly larger binary. No advisory is caused by duplication, waived or
otherwise.

Reproduce with:

```bash
(cd gpy-agent && cargo tree --duplicates)
(cd gpy-agent && cargo tree -i <crate>[@version] -e normal,build,dev --target all)
```

## What this document does not cover

- Direct dependencies. None currently carries an advisory.
- Vulnerability reporting. See `SECURITY.md`.
- License and source policy. Enforced by `cargo deny check licenses sources`; the
  allowed set is in `gpy-agent/deny.toml`.

## Verification

```bash
(cd gpy-agent && cargo audit)
(cd gpy-agent && cargo deny check advisories licenses bans sources)
bash tests/bash/dependency_advisory_claims.test.bash
```

## History

- **2026-09-01** (#525): retired the hyperpolyglot advisory waiver. `cargo audit`
  reports zero advisories following #523's removal of `hyperpolyglot`, so the five
  RUSTSEC IDs below, their `deny.toml` ignore entries, and the reachability writeup
  are gone rather than updated. Re-derived the duplicate-version table against the
  post-swap lockfile (351 → 319 crate dependencies) and narrowed its scope to the
  crates `deny.toml` actually skips. Fixed `gpy-agent/docs/implementation-guide.md`
  (since removed as an obsolete pre-implementation scaffold, #505), which still
  listed `hyperpolyglot = "0.1"` as a dependency to add.
- **2026-09-01** (#503): the waiver this entry replaces. Corrected to five
  advisories. Added RUSTSEC-2021-0145 and RUSTSEC-2026-0097 to `deny.toml`, turned on
  `unsound`/`unmaintained` evaluation so those entries did real work, replaced the
  blanket rationale with a per-advisory one, and replaced the stale resolution notes
  with a reachability finding and a dated review. For the record, the five waived
  advisories were:

  | Advisory | Crate | Class | Path from `gpy-agent` |
  |---|---|---|---|
  | RUSTSEC-2021-0139 | ansi_term 0.12.1 | unmaintained | hyperpolyglot 0.1.7 → clap 2.34.0 → ansi_term |
  | RUSTSEC-2024-0375 | atty 0.2.14 | unmaintained | hyperpolyglot 0.1.7 → clap 2.34.0 → atty |
  | RUSTSEC-2021-0145 | atty 0.2.14 | unsound | hyperpolyglot 0.1.7 → clap 2.34.0 → atty |
  | RUSTSEC-2024-0320 | yaml-rust 0.4.5 | unmaintained | hyperpolyglot 0.1.7 → serde_yaml 0.8.26 → yaml-rust |
  | RUSTSEC-2026-0097 | rand 0.7.3 | unsound | hyperpolyglot 0.1.7 → phf_codegen 0.8.0 → phf_generator 0.8.0 → rand |

  Every chain reached `gpy-agent` through hyperpolyglot 0.1.7, whose last release was
  2020-07-26; `ansi_term` and `atty` came from a clap v2 the bundled `hyply` binary
  used (never built for library consumers), `yaml-rust` from a codegen-only
  `serde_yaml`, and `rand` from a codegen-only `phf_generator` — none reachable from
  the language-detection code path GPY actually called.
- **2025-11-14**: `lib.rs` duplicate-dependency audit (versions since superseded).
- **2025-09-12**: previous revision of this document; four advisories, review date
  2025-12-12, which passed unobserved.
- **2025-09-09**: removed `sled`, `tarpc` and `rmp-serde`.
