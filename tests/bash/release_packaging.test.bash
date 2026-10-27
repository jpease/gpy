#!/usr/bin/env bash
# tests/bash/release_packaging.test.bash
#
# The release archive must contain the shell files the installers use, and a
# missing file must fail the release. Asserts:
#   (a) archives contain the files the installers reach for
#   (b) binaries carry their release asset names
#   (c) packaging fails closed on a missing build artifact
#   (d) release notes come from the CHANGELOG and link no deleted docs
#   (e) release notes fail closed without a CHANGELOG section for the tag
#   (f) every published file has a correct SHA-256 sidecar, covered by SHA256SUMS
#   (g) packaging fails without --sbom or --no-sbom
#   (h) install-oneline.sh ships tag-stamped beside its shell files; --out cannot be the repo root

# shellcheck disable=SC2016
# Single quotes are load-bearing: assertions grep for literal `$VAR` text.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

test_require_command zip "zip/unzip not available; release packaging test needs both"
test_require_command unzip "zip/unzip not available; release packaging test needs both"

WORKDIR="$(mktemp -d)"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

# Must match release.yml's matrix; case (c) relies on the packager failing on absent ones.
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

# Stub artifacts as download-artifact writes them: one dir per asset, cargo's file name.
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

# Stand-in SBOM; release_sbom.test.bash covers the document.
SBOM="$WORKDIR/sbom.cdx.json"
printf '{"bomFormat":"CycloneDX","specVersion":"1.5","version":1,"components":[]}\n' >"$SBOM"

echo "--- packaging v9.9.9 ---"
if ! "$ROOT/scripts/package-release.sh" --version v9.9.9 --artifacts "$ARTIFACTS" --out "$OUT" --sbom "$SBOM"; then
    fail "package-release.sh exited non-zero with a complete set of artifacts"
fi

# (a) + (b): asserted against the archive listings.
tar_listing="$(tar -tzf "$OUT/gpy-release.tar.gz" 2>/dev/null || true)"
zip_listing="$(unzip -Z1 "$OUT/gpy-release.zip" 2>/dev/null || true)"

expect_entry() {
    local listing="$1" label="$2" entry="$3"
    grep -qxF "$entry" <<<"$listing" || fail "$label does not contain $entry"
}

for entry in \
    install.sh \
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
    docs/user/configuration-reference.md \
    plugins/hostname/plugin.toml \
    plugins/hostname/bin/hostname.sh \
    plugins/username/plugin.toml \
    plugins/username/bin/username.sh; do
    expect_entry "$tar_listing" "gpy-release.tar.gz" "gpy-release/$entry"
    expect_entry "$zip_listing" "gpy-release.zip" "gpy-release/$entry"
done

for asset in "${ASSETS[@]}"; do
    expect_entry "$tar_listing" "gpy-release.tar.gz" "gpy-release/bin/$asset"
done

grep -qxF "gpy-release/bin/gpy-agent" <<<"$tar_listing" &&
    fail "archive contains an unrenamed bin/gpy-agent; assets must be renamed to their asset names"

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

for published in gpy-release.tar.gz gpy-release.zip gpy-shell-files.tar.gz \
    install-oneline.sh uninstall.sh gpy-sbom.cdx.json; do
    if [[ -f "$OUT/$published" ]]; then
        expect_sidecar "$OUT/$published"
    else
        fail "package-release.sh did not produce $published"
    fi
done

for asset in "${ASSETS[@]}"; do
    expect_sidecar "$OUT/gpy-release/bin/$asset"
    # install.sh verifies bin/<asset>.sha256 inside the extracted archive.
    expect_entry "$tar_listing" "gpy-release.tar.gz" "gpy-release/bin/$asset.sha256"
    expect_entry "$zip_listing" "gpy-release.zip" "gpy-release/bin/$asset.sha256"
done

expect_entry "$tar_listing" "gpy-release.tar.gz" "gpy-release/sbom.cdx.json"

if [[ -f "$OUT/SHA256SUMS" ]]; then
    for name in gpy-release.tar.gz gpy-release.zip gpy-shell-files.tar.gz \
        install-oneline.sh uninstall.sh gpy-sbom.cdx.json "${ASSETS[@]}"; do
        grep -qE "^[0-9a-f]{64}  $name\$" "$OUT/SHA256SUMS" ||
            fail "SHA256SUMS has no entry for $name"
    done
    # Nothing downloadable may be left out: 6 published files + 10 binaries.
    manifest_lines="$(wc -l <"$OUT/SHA256SUMS" | tr -d ' ')"
    [[ "$manifest_lines" -eq $((6 + ${#ASSETS[@]})) ]] ||
        fail "SHA256SUMS has $manifest_lines entries, expected $((6 + ${#ASSETS[@]}))"
else
    fail "package-release.sh did not produce a SHA256SUMS manifest"
fi

# The installer is a release asset with the tag stamped in, so a given tag
# always serves the same script; the source copy keeps the placeholder.
echo "--- installer assets ---"
if grep -qxF 'RELEASE_VERSION="v9.9.9"' "$OUT/install-oneline.sh"; then
    [[ "$(grep -c '^RELEASE_VERSION=' "$OUT/install-oneline.sh")" -eq 1 ]] ||
        fail "install-oneline.sh asset has more than one RELEASE_VERSION assignment"
else
    fail "install-oneline.sh asset is not stamped with v9.9.9"
fi
grep -qxF 'RELEASE_VERSION=""' "$ROOT/install-oneline.sh" ||
    fail "the source install-oneline.sh no longer has its empty RELEASE_VERSION placeholder"
# Stamping must change that one line and nothing else.
if ! diff <(sed 's/^RELEASE_VERSION="v9.9.9"$/RELEASE_VERSION=""/' "$OUT/install-oneline.sh") "$ROOT/install-oneline.sh" >/dev/null; then
    fail "install-oneline.sh asset differs from the source beyond the stamped version"
fi
sh -n "$OUT/install-oneline.sh" || fail "the stamped install-oneline.sh does not parse"
cmp -s "$ROOT/scripts/uninstall.sh" "$OUT/uninstall.sh" || fail "uninstall.sh asset differs from scripts/uninstall.sh"

# gpy-shell-files.tar.gz is what install-oneline.sh unpacks and hands to
# `gpy setup --from`: the three shell trees and the bundled plugins at the
# top level, matching the repo.
shell_listing="$(tar -tzf "$OUT/gpy-shell-files.tar.gz" 2>/dev/null || true)"
for entry in \
    fish/conf.d/gpy_init.fish fish/functions/fish_prompt.fish fish/core/init.fish \
    fish/segments/git.fish fish/completions/gpy-dynamic.fish \
    zsh/gpy.zsh zsh/core/init.zsh zsh/segments/git.zsh zsh/completions/_gpy-dynamic \
    bash/gpy.bash bash/core/init.bash bash/segments/git.bash bash/completions/gpy-dynamic.bash \
    plugins/hostname/plugin.toml plugins/username/plugin.toml; do
    expect_entry "$shell_listing" "gpy-shell-files.tar.gz" "$entry"
done
if grep -vE '^(fish|zsh|bash|plugins)/' <<<"$shell_listing" | grep -q .; then
    fail "gpy-shell-files.tar.gz holds entries outside fish/, zsh/, bash/ and plugins/: $(grep -vE '^(fish|zsh|bash|plugins)/' <<<"$shell_listing")"
fi
mkdir -p "$WORKDIR/shell-files"
tar -xzf "$OUT/gpy-shell-files.tar.gz" -C "$WORKDIR/shell-files"
for shell_dir in fish zsh bash plugins; do
    diff -r "$ROOT/$shell_dir" "$WORKDIR/shell-files/$shell_dir" >/dev/null ||
        fail "gpy-shell-files.tar.gz's $shell_dir/ differs from the repository's $shell_dir/"
done

# --out at the repo root would overwrite the source installer.
echo "--- packaging into the repository root (must fail) ---"
if "$ROOT/scripts/package-release.sh" --version v9.9.9 --artifacts "$ARTIFACTS" \
    --out "$ROOT" --sbom "$SBOM" >/dev/null 2>&1; then
    fail "package-release.sh accepted the repository root as --out"
fi
grep -qxF 'RELEASE_VERSION=""' "$ROOT/install-oneline.sh" ||
    fail "packaging into the repository root modified the source install-oneline.sh"

# (g) omitting both SBOM flags is an error.
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
if ! "$ROOT/scripts/release-notes.sh" --version v0.4.0 --output "$NOTES" >/dev/null; then
    fail "release-notes.sh exited non-zero for a version present in the CHANGELOG"
fi

if [[ -f "$NOTES" ]]; then
    # Channels this project does not ship must not appear in any form.
    for stale in "cargo install gpy-agent" "brew install gpy" makepkg dpkg; do
        grep -qF "$stale" "$NOTES" && fail "release notes still offer unsupported channel '$stale'"
    done
    # Deleted docs must not be LINKED; the CHANGELOG may still name them in prose.
    for stale in CROSS_PLATFORM_TESTING.md config.example.fish REFACTOR.md; do
        grep -qE "\]\([^)]*$stale" "$NOTES" && fail "release notes still link removed file '$stale'"
    done
    grep -q "^# GPY v0.4.0" "$NOTES" || fail "release notes missing the version heading"
    grep -q "install-oneline.sh" "$NOTES" || fail "release notes missing the one-line installer"
    grep -q "docs/INSTALL.md" "$NOTES" || fail "release notes do not link the installation guide"
    grep -qF "curl -fsSL https://github.com/jpease/gpy/releases/download/v0.4.0/install-oneline.sh | sh" "$NOTES" ||
        fail "release notes do not give the release-asset one-liner for the tag"
    grep -qF 'raw.githubusercontent.com' "$NOTES" && fail "release notes still fetch an installer from raw.githubusercontent.com"
else
    fail "release-notes.sh produced no output file"
fi

# The asset table is generated from the binaries actually built.
echo "--- release notes asset table ---"
ASSET_NOTES="$WORKDIR/notes-assets.md"
if ! "$ROOT/scripts/release-notes.sh" --version v0.4.0 --assets "$OUT/gpy-release/bin" \
    --output "$ASSET_NOTES" >/dev/null; then
    fail "release-notes.sh exited non-zero with --assets"
else
    for asset in "${ASSETS[@]}"; do
        grep -qF "\`$asset\`" "$ASSET_NOTES" || fail "release notes asset table is missing $asset"
    done
    # Skip per-binary .sha256 sidecars.
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

test_finish
