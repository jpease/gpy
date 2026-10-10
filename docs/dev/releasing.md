# Releasing GPY

How to cut a GPY release, and what the automated gate checks so you do not have
to. The first release was v0.1.0 (2026-10-10).

Distribution is GitHub Releases, the one-line installer, and the
`jpease/homebrew-tap` Homebrew tap: see
[Distribution channels](#distribution-channels).

## Version-bearing files

Three files declare the version. The `validate` job in
`.github/workflows/release.yml` refuses to build unless all three agree with the
tag, so a release cannot ship binaries that report a different version than the
tag they came from.

<!-- version-bearing-files:start -->
- `gpy-agent/Cargo.toml` — `version = "X.Y.Z"`; stamps both binaries.
- `fish/fisher.json` — `"version": "X.Y.Z"`; `scripts/package-release.sh`
  re-stamps this in the packaged plugin archive, so a mismatch here means the
  committed tree and the published archive disagree.
- `CHANGELOG.md` — a `## [X.Y.Z]` heading. `scripts/release-notes.sh` builds the
  GitHub release body from that section and fails if it is empty.
<!-- version-bearing-files:end -->

`tests/bash/release_version_claims.test.bash` derives this list from the
workflow itself and diffs it against the block above, so adding a fourth
version-bearing file to `release.yml` turns the gate red until this section
names it too.

## Cutting a release

Everything below runs on `main`, with a clean tree.

1. **Pick the version.** GPY is 0.x: the IPC protocol version negotiation and
   the config schema are both still moving, so the minor number carries
   breaking changes. See
   [gpy-agent/SCHEMA_EVOLUTION.md](../../gpy-agent/SCHEMA_EVOLUTION.md).

2. **Bump the three version-bearing files** listed above to the same `X.Y.Z`.

3. **Close the CHANGELOG section.** Rename `## [Unreleased]` work into a
   `## [X.Y.Z] - YYYY-MM-DD` heading and **replace `YYYY-MM-DD` with the real
   release date** in ISO form. The placeholder is deliberate — it is greppable,
   and a date left unset is a visible defect rather than a plausible-looking
   wrong date. Leave an empty `## [Unreleased]` heading above it for the next
   cycle, and add a `[X.Y.Z]: https://github.com/jpease/gpy/releases/tag/vX.Y.Z`
   link reference at the bottom.

   ```sh
   grep -n 'YYYY-MM-DD' CHANGELOG.md   # must print nothing before tagging
   ```

4. **Bump `SECURITY.md`.** The Supported Versions table names one line, the
   released minor series (`0.1.x` for v0.1.0). The gate asserts it matches.

5. **Preview the release body.** This is exactly what the workflow publishes:

   ```sh
   scripts/release-notes.sh --version vX.Y.Z | less
   ```

   It exits non-zero if the CHANGELOG section is missing or empty.

6. **Run the full gate.**

   ```sh
   ./scripts/quality-check.sh
   ```

7. **Commit and push** the version bump. Do not tag a commit that is not on
   `origin/main`.

8. **Tag and push the tag.** The `v` prefix is required; `release.yml` triggers
   on `v*` and rejects anything that is not `vMAJOR.MINOR.PATCH`.

   ```sh
   git tag -a vX.Y.Z -m "GPY vX.Y.Z"
   git push origin vX.Y.Z
   ```

   A tag push and a manual `workflow_dispatch` follow the same path: the
   `validate` job resolves the version once and passes it downstream as a job
   output, so the package and release jobs cannot disagree with what it checked.
   Dispatch takes the version as an input (`vX.Y.Z`) and validates it against
   the same three files on the checked-out ref. A dispatch is a dry run: it
   validates, builds, packages and smoke-runs the artifacts on Linux, macOS
   and Windows, and publishes nothing unless `-f publish=true` is passed
   (`gh -R jpease/gpy workflow run release.yml --ref <branch> -f
   version=vX.Y.Z`). A tag push always publishes.

9. **Watch the run.** `validate` → `build` (five targets) → `package` →
   `release`. The gate that runs on a pull request runs here too, so a tag
   cannot ship something that would not have merged.

10. **Verify the published release.** The assets are asserted by
    `scripts/package-release.sh` after the fact, but check by hand once:

    ```sh
    gh release view vX.Y.Z
    curl -fsSLO https://github.com/jpease/gpy/releases/download/vX.Y.Z/SHA256SUMS
    ```

    Every binary and archive has a `.sha256` sidecar, an entry in `SHA256SUMS`,
    and a build-provenance attestation; `gpy-sbom.cdx.json` is the CycloneDX
    SBOM. Both installers refuse to install a binary they cannot verify.

11. **Install from the release** on a clean machine or container and confirm the
    prompt renders. The one-line installer fetches itself from the tag, not from
    `main`.

12. **Update the Homebrew tap.** The formula lives in `jpease/homebrew-tap`
    (`Formula/gpy.rb`), not in this repository. Point `url` at the new tag
    tarball, set `sha256` from the real digest, then check it installs:

    ```sh
    curl -sL https://github.com/jpease/gpy/archive/refs/tags/vX.Y.Z.tar.gz | shasum -a 256
    brew style Formula/gpy.rb
    brew install --build-from-source jpease/tap/gpy
    ```

## What the gate checks for you

| Check | Where |
|---|---|
| Tag format is `vMAJOR.MINOR.PATCH` | `release.yml`, `validate` |
| Tag matches all three version-bearing files | `release.yml`, `validate` |
| Full Rust + shell gate, `just lint`, and the performance budgets (`just bench-ci`) pass at the tagged commit | `release.yml`, `validate` |
| The packaged binaries actually run: the archive's `install.sh`, both `--version`s, a oneshot render, a responding agent, one prompt each from fish, zsh and bash (Linux); both `.exe` `--version`s and `gpy --help` (Windows). A failure here blocks `create-release` | `release.yml`, `smoke` → `scripts/smoke-release.sh` |
| No Linux binary (either arch) references a glibc newer than `GPY_GLIBC_FLOOR` (2.31); the Linux legs link against that floor with `cargo zigbuild --target <triple>.2.31`, and the packaged x86_64 binaries run their `--version` in `ubuntu:20.04` | `release.yml`, `build` → `scripts/check-glibc-floor.sh`; `smoke` |
| Release notes are non-empty and CHANGELOG-derived | `scripts/release-notes.sh` |
| Archives contain the files the installers reach for | `scripts/package-release.sh` |
| Build matrix agrees with both installers' asset names | `tests/bash/release_asset_contract.test.bash` |
| Packaging and note generation still work | `tests/bash/release_packaging.test.bash` |
| The Linux legs stay pinned to the glibc floor, the gate checks it, and `docs/INSTALL.md` states the same floor | `tests/bash/release_glibc_floor.test.bash` |
| One version declared, no advertised dead channel | `tests/bash/release_version_claims.test.bash` |
| The smoke run passes on the real debug binaries and fails on an agent that answers `--version` but nothing else | `tests/bash/release_smoke.test.bash` |
| A stock `ubuntu:24.04` container with an empty `$HOME` installs from an archive built from the checkout and renders one prompt per shell | `scripts/test-fresh-install.sh` (`tests/docker/Dockerfile.fresh-install`; the `fresh-install` job in `pr-gate.yml`) |

The `tests/bash/*` rows run on every `./scripts/quality-check.sh`, not only when
a tag is pushed. The container check needs a running docker daemon; run it
locally before a release with `scripts/test-fresh-install.sh`.

## Distribution channels

A channel is documented only once it works. Offering an install command that
fails is worse than offering nothing, because the failure looks like the
project's fault to a first-time user.

**Operational:**

- **GitHub Releases** — the one-line installer, and the `gpy-release.tar.gz` /
  `.zip` archives with `install.sh`. See
  [docs/INSTALL.md](../INSTALL.md), which is also the canonical
  [OS and shell support matrix](../INSTALL.md#supported-platforms-and-shells).
  Do not restate that matrix here or anywhere else.

- **Homebrew** — the personal tap `jpease/homebrew-tap`, installed with
  `brew install jpease/tap/gpy` (Fish only; builds from the tag tarball). Every
  reference must be tap-qualified — the bare `brew install <name>` form implies
  homebrew-core and is banned repo-wide by
  `tests/bash/crate_publish_boundary.test.bash`. homebrew-core itself needs 90
  forks, 90 watchers, 225 stars and a repo at least 30 days old; it is a
  post-launch goal (#512).

**Not operational — do not document as available:**

- **Fisher.** `release.yml` publishes `fisher-gpy.tar.gz`, but the plugin tree
  ships no agent binary, so it is not a standalone install. It is not offered by
  the installers or by any doc page.
- **crates.io.** `gpy-agent` sets `publish = false` and is never published
  (#502). A registry install would place two binaries and none of the shell
  files the prompt renders from.
- **Debian and Arch.** No package is built. The release workflow used to
  advertise both; #492 removed the claims rather than the other way round.
