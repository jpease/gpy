#!/usr/bin/env bash
# tests/bash/release_packaging.test.bash
#
# Regression test for #492: the release workflow packaged root-level fish
# paths (core/, segments/, themes/) that moved into fish/ long ago, and wrapped
# every copy in `2>/dev/null || echo "... may be missing"`. A release could
# therefore publish an archive with no shell files in it and still go green,
# and nothing but a tag push would ever have revealed it.
#
# This exercises scripts/package-release.sh and scripts/release-notes.sh with
# stub build artifacts, so packaging is verified on every quality-check run
# rather than only when a tag is cut. It asserts:
#   (a) the archives contain the files the installers actually reach for
#   (b) binaries are renamed to their release asset names, which is what
#       install.sh resolves under bin/
#   (c) packaging fails closed when a build artifact is missing
#   (d) release notes come from the CHANGELOG and no longer reference the
#       deleted CROSS_PLATFORM_TESTING.md / config.example.fish / REFACTOR.md
#   (e) release notes fail closed when the CHANGELOG has no section for the tag
#   (f) every published file gets a correct SHA-256 sidecar, the aggregate
#       SHA256SUMS manifest covers them all, and the sidecars ride inside the
#       archive where install.sh looks for them (#494)
#   (g) packaging fails closed when neither --sbom nor --no-sbom is given

# shellcheck disable=SC2016
# The single quotes are load-bearing throughout this file: the assertions grep
# for literal `$VAR` text in a script under test, so expanding here would test
# the wrong thing.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

if ! command -v zip >/dev/null 2>&1 || ! command -v unzip >/dev/null 2>&1; then
    echo "SKIP: zip/unzip not available; release packaging test needs both"
    exit 0
fi

WORKDIR="$(mktemp -d)"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

failures=0
fail() {
    echo "FAIL: $*"
    failures=$((failures + 1))
}

# Asset names must stay in step with the build matrix in release.yml;
# package-release.sh fails closed on any that are absent, which is what
# case (c) below relies on.
ASSETS=(
    gpy-agent-linux-x86_64
    gpy-agent-linux-aarch64
    gpy-agent-macos-x86_64
    gpy-agent-macos-aarch64
    gpy-agent-windows-x86_64.exe
    gpy-linux-x86_64
    gpy-linux-aarch64
    gpy-macos-x86_64
    gpy-macos-aarch64
    gpy-windows-x86_64.exe
)

# Stub artifacts laid out the way actions/download-artifact@v4 writes them:
# one directory per artifact, named after the release asset, holding the file
# under the name cargo produced (gpy-agent / gpy / *.exe).
ARTIFACTS="$WORKDIR/release-artifacts"
for asset in "${ASSETS[@]}"; do
    mkdir -p "$ARTIFACTS/$asset"
    case "$asset" in
        gpy-agent-*.exe) inner="gpy-agent.exe" ;;
        gpy-agent-*) inner="gpy-agent" ;;
        *.exe) inner="gpy.exe" ;;
        *) inner="gpy" ;;
    esac
    printf 'stub binary for %s\n' "$asset" >"$ARTIFACTS/$asset/$inner"
done

OUT="$WORKDIR/out"
mkdir -p "$OUT"

# A stand-in for scripts/generate-sbom.sh output. This test is about packaging,
# not SBOM content -- tests/bash/release_sbom.test.bash covers the document
# itself -- and a stub keeps packaging verifiable without a Rust toolchain.
SBOM="$WORKDIR/sbom.cdx.json"
printf '{"bomFormat":"CycloneDX","specVersion":"1.5","version":1,"components":[]}\n' >"$SBOM"

echo "--- packaging v9.9.9 ---"
if ! "$ROOT/scripts/package-release.sh" --version v9.9.9 --artifacts "$ARTIFACTS" --out "$OUT" --sbom "$SBOM"; then
    fail "package-release.sh exited non-zero with a complete set of artifacts"
fi

# (a) + (b): assert against the archive listings, independently of the
# script's own verification pass.
tar_listing="$(tar -tzf "$OUT/gpy-release.tar.gz" 2>/dev/null || true)"
zip_listing="$(unzip -Z1 "$OUT/gpy-release.zip" 2>/dev/null || true)"
fisher_listing="$(tar -tzf "$OUT/fisher-gpy.tar.gz" 2>/dev/null || true)"

expect_entry() {
    local listing="$1" label="$2" entry="$3"
    grep -qxF "$entry" <<<"$listing" || fail "$label does not contain $entry"
}

for entry in \
    install.sh \
    scripts/uninstall.fish \
    scripts/uninstall.sh \
    fish/core/init.fish \
    fish/conf.d/gpy_init.fish \
    fish/functions/fish_prompt.fish \
    fish/completions/gpy-dynamic.fish \
    bash/gpy.bash \
    bash/core/init.bash \
    zsh/gpy.zsh \
    zsh/core/init.zsh \
    config/config.example.toml \
    docs/README.md \
    docs/LICENSE \
    docs/INSTALL.md \
    docs/user/configuration-reference.md; do
    expect_entry "$tar_listing" "gpy-release.tar.gz" "gpy-release/$entry"
    expect_entry "$zip_listing" "gpy-release.zip" "gpy-release/$entry"
done

for asset in "${ASSETS[@]}"; do
    expect_entry "$tar_listing" "gpy-release.tar.gz" "gpy-release/bin/$asset"
done

# The old packaging step copied every artifact as its build-time name, so all
# five agent binaries collided on bin/gpy-agent and install.sh -- which looks
# for bin/gpy-agent-linux-x86_64 -- found nothing.
grep -qxF "gpy-release/bin/gpy-agent" <<<"$tar_listing" &&
    fail "archive contains an unrenamed bin/gpy-agent; assets must be renamed to their asset names"

for entry in fisher.json conf.d/gpy_init.fish core/init.fish functions/fish_prompt.fish LICENSE; do
    expect_entry "$fisher_listing" "fisher-gpy.tar.gz" "fisher-gpy/$entry"
done

# The Fisher manifest must carry the released version, not the hardcoded
# "1.0.0" the workflow used to generate, and must keep the project license.
tar -xzf "$OUT/fisher-gpy.tar.gz" -C "$WORKDIR"
grep -q '"version": "9.9.9"' "$WORKDIR/fisher-gpy/fisher.json" ||
    fail "fisher.json was not stamped with the release version"
grep -q '"license": "GPL-3.0-or-later"' "$WORKDIR/fisher-gpy/fisher.json" ||
    fail "fisher.json lost its GPL-3.0-or-later declaration"

# (f) verified downloads: every published file carries a correct digest, and
# the per-binary sidecars ship inside the archive where install.sh reads them.
echo "--- checksums ---"

if command -v sha256sum >/dev/null 2>&1; then
    sha256_of() { sha256sum "$1" | awk '{print $1}'; }
elif command -v shasum >/dev/null 2>&1; then
    sha256_of() { shasum -a 256 "$1" | awk '{print $1}'; }
else
    sha256_of() { echo "no-sha256-tool"; }
fi

expect_sidecar() {
    local file="$1" recorded actual
    if [[ ! -f "$file.sha256" ]]; then
        fail "no checksum sidecar for $(basename "$file")"
        return
    fi
    recorded="$(awk '{print $1}' "$file.sha256")"
    actual="$(sha256_of "$file")"
    [[ "$recorded" == "$actual" ]] ||
        fail "checksum sidecar for $(basename "$file") does not match its contents"
    # Coreutils format, bare filename: `sha256sum -c` has to work from the
    # directory the asset sits in, and install-oneline.sh takes field 1.
    grep -qE "^[0-9a-f]{64}  $(basename "$file")\$" "$file.sha256" ||
        fail "checksum sidecar for $(basename "$file") is not in '<digest>  <basename>' format"
}

for published in gpy-release.tar.gz gpy-release.zip fisher-gpy.tar.gz gpy-sbom.cdx.json; do
    if [[ -f "$OUT/$published" ]]; then
        expect_sidecar "$OUT/$published"
    else
        fail "package-release.sh did not produce $published"
    fi
done

for asset in "${ASSETS[@]}"; do
    expect_sidecar "$OUT/gpy-release/bin/$asset"
    # install.sh verifies bin/<asset>.sha256 from inside the extracted archive,
    # so the sidecar has to be an archive member, not just an OUT_DIR artifact.
    expect_entry "$tar_listing" "gpy-release.tar.gz" "gpy-release/bin/$asset.sha256"
    expect_entry "$zip_listing" "gpy-release.zip" "gpy-release/bin/$asset.sha256"
done

expect_entry "$tar_listing" "gpy-release.tar.gz" "gpy-release/sbom.cdx.json"

if [[ -f "$OUT/SHA256SUMS" ]]; then
    for name in gpy-release.tar.gz gpy-release.zip fisher-gpy.tar.gz gpy-sbom.cdx.json "${ASSETS[@]}"; do
        grep -qE "^[0-9a-f]{64}  $name\$" "$OUT/SHA256SUMS" ||
            fail "SHA256SUMS has no entry for $name"
    done
    # Nothing downloadable may be left out: 4 published files + 10 binaries.
    manifest_lines="$(wc -l <"$OUT/SHA256SUMS" | tr -d ' ')"
    [[ "$manifest_lines" -eq $((4 + ${#ASSETS[@]})) ]] ||
        fail "SHA256SUMS has $manifest_lines entries, expected $((4 + ${#ASSETS[@]}))"
else
    fail "package-release.sh did not produce a SHA256SUMS manifest"
fi

# (g) the SBOM cannot be forgotten: omitting both flags is an error, not a
# silent skip.
echo "--- packaging with no SBOM decision (must fail) ---"
if "$ROOT/scripts/package-release.sh" --version v9.9.9 --artifacts "$ARTIFACTS" \
    --out "$WORKDIR/out-nosbom-flag" >/dev/null 2>&1; then
    fail "package-release.sh succeeded without --sbom or --no-sbom"
fi

echo "--- packaging with --no-sbom (must succeed, no SBOM assets) ---"
if ! "$ROOT/scripts/package-release.sh" --version v9.9.9 --artifacts "$ARTIFACTS" \
    --out "$WORKDIR/out-no-sbom" --no-sbom >/dev/null 2>&1; then
    fail "package-release.sh failed with an explicit --no-sbom"
else
    [[ ! -e "$WORKDIR/out-no-sbom/gpy-sbom.cdx.json" ]] ||
        fail "--no-sbom still produced gpy-sbom.cdx.json"
    grep -q 'gpy-sbom' "$WORKDIR/out-no-sbom/SHA256SUMS" &&
        fail "--no-sbom left an SBOM entry in SHA256SUMS"
fi

# (c) fail closed on a missing build artifact.
echo "--- packaging with a missing artifact (must fail) ---"
rm -rf "$ARTIFACTS/gpy-macos-aarch64"
if "$ROOT/scripts/package-release.sh" --version v9.9.9 --artifacts "$ARTIFACTS" \
    --out "$WORKDIR/out-partial" --sbom "$SBOM" >/dev/null 2>&1; then
    fail "package-release.sh succeeded with a missing build artifact"
fi

# (d) release notes come from the CHANGELOG.
echo "--- release notes for a version present in the CHANGELOG ---"
NOTES="$WORKDIR/notes.md"
if ! "$ROOT/scripts/release-notes.sh" --version v0.1.0 --output "$NOTES" >/dev/null; then
    fail "release-notes.sh exited non-zero for a version present in the CHANGELOG"
fi

if [[ -f "$NOTES" ]]; then
    # Channels this project does not ship must not appear in any form.
    for stale in "cargo install gpy-agent" "brew install gpy" makepkg dpkg; do
        grep -qF "$stale" "$NOTES" && fail "release notes still offer unsupported channel '$stale'"
    done
    # Deleted docs must not be LINKED. The notes body is the CHANGELOG section,
    # and a CHANGELOG entry may legitimately name a deleted file while
    # describing the bug that removed it -- #492's own entry names all three.
    # Match the markdown link form so a bare mention in prose is allowed.
    for stale in CROSS_PLATFORM_TESTING.md config.example.fish REFACTOR.md; do
        grep -qE "\]\([^)]*$stale" "$NOTES" && fail "release notes still link removed file '$stale'"
    done
    grep -q "^# GPY v0.1.0" "$NOTES" || fail "release notes missing the version heading"
    grep -q "install-oneline.sh" "$NOTES" || fail "release notes missing the one-line installer"
    grep -q "docs/INSTALL.md" "$NOTES" || fail "release notes do not link the installation guide"
    grep -qF '| GPY_VERSION=v0.1.0 sh' "$NOTES" || fail "release notes pin the version on curl instead of sh"
else
    fail "release-notes.sh produced no output file"
fi

# The asset table is generated from the binaries actually built, so it cannot
# drift from the build matrix the way the old hardcoded platform table did.
echo "--- release notes asset table ---"
ASSET_NOTES="$WORKDIR/notes-assets.md"
if ! "$ROOT/scripts/release-notes.sh" --version v0.1.0 --assets "$OUT/gpy-release/bin" \
    --output "$ASSET_NOTES" >/dev/null; then
    fail "release-notes.sh exited non-zero with --assets"
else
    for asset in "${ASSETS[@]}"; do
        grep -qF "\`$asset\`" "$ASSET_NOTES" || fail "release notes asset table is missing $asset"
    done
    # bin/ also holds a .sha256 per binary now; those belong in the
    # verification section, not the platform table (#494).
    grep -qE '^\| `[^`]+\.sha256`' "$ASSET_NOTES" &&
        fail "release notes asset table lists checksum sidecars as platform binaries"
    grep -qF 'gh attestation verify' "$ASSET_NOTES" ||
        fail "release notes do not explain how to verify build provenance"
    grep -qF 'SHA256SUMS' "$ASSET_NOTES" ||
        fail "release notes do not explain how to verify checksums"
fi

# (e) fail closed when the CHANGELOG has no section for the tag.
echo "--- release notes for an unreleased version (must fail) ---"
if "$ROOT/scripts/release-notes.sh" --version v9.9.9 >/dev/null 2>&1; then
    fail "release-notes.sh succeeded for a version with no CHANGELOG section"
fi

if [[ $failures -ne 0 ]]; then
    echo "$failures assertion(s) failed"
    exit 1
fi

echo "PASS"
