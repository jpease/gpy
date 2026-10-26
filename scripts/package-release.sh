#!/usr/bin/env bash
# Build the GPY release archives from downloaded build artifacts.
#
# Lives outside .github/workflows/release.yml so the packaging logic is
# testable locally (tests/bash/release_packaging.test.bash) instead of only
# being exercised by a tag push.
#
# Everything here is fail-closed: a missing source file, a missing build
# artifact, or an archive that does not contain the expected entries aborts
# the run.
#
# Usage:
#   scripts/package-release.sh --version v0.1.0 \
#       --artifacts release-artifacts --out package-out \
#       (--sbom sbom.cdx.json | --no-sbom)
#
# --artifacts points at the directory actions/download-artifact@v4 wrote into,
# where each artifact is its own subdirectory named after the release asset:
#   release-artifacts/gpy-agent-linux-x86_64/gpy-agent
#   release-artifacts/gpy-linux-x86_64/gpy
#
# --sbom takes the CycloneDX document from scripts/generate-sbom.sh. It is
# required for a real release; --no-sbom exists for local runs that only care
# about archive contents. Nothing is optional by omission.
#
# Produces, in --out (which must not be the repository root: the published
# install-oneline.sh would overwrite the source installer):
#   gpy-release.tar.gz / gpy-release.zip  (binaries + all three shells + docs)
#   gpy-shell-files.tar.gz                (fish/, zsh/, bash/ trees: the files
#                                          install-oneline.sh installs)
#   install-oneline.sh                    (the source installer with this
#                                          release's tag stamped in)
#   uninstall.sh                          (the uninstaller, as a release asset)
#   gpy-sbom.cdx.json                     (unless --no-sbom)
#   <asset>.sha256                        (one per published file)
#   SHA256SUMS                            (aggregate manifest)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

VERSION=""
ARTIFACTS_DIR=""
OUT_DIR="$PWD"
SBOM_SRC=""
SBOM_OPTED_OUT=0

die() {
    echo "package-release: $*" >&2
    exit 1
}

usage() {
    sed -n '2,38p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    exit 2
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --version) VERSION="${2:-}"; shift 2 ;;
        --artifacts) ARTIFACTS_DIR="${2:-}"; shift 2 ;;
        --out) OUT_DIR="${2:-}"; shift 2 ;;
        --sbom) SBOM_SRC="${2:-}"; shift 2 ;;
        --no-sbom) SBOM_OPTED_OUT=1; shift ;;
        -h|--help) usage ;;
        *) die "unknown argument: $1" ;;
    esac
done

[[ -n "$VERSION" ]] || die "--version is required (e.g. --version v0.1.0)"
[[ -n "$ARTIFACTS_DIR" ]] || die "--artifacts is required"
[[ -d "$ARTIFACTS_DIR" ]] || die "artifacts directory not found: $ARTIFACTS_DIR"
[[ "$VERSION" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "invalid version: $VERSION (expected vMAJOR.MINOR.PATCH)"

if [[ -n "$SBOM_SRC" ]]; then
    [[ $SBOM_OPTED_OUT -eq 0 ]] || die "--sbom and --no-sbom are mutually exclusive"
    [[ -f "$SBOM_SRC" ]] || die "SBOM not found: $SBOM_SRC"
elif [[ $SBOM_OPTED_OUT -eq 0 ]]; then
    die "one of --sbom <file> or --no-sbom is required (generate one with scripts/generate-sbom.sh)"
fi

ARTIFACTS_DIR="$(cd "$ARTIFACTS_DIR" && pwd)"
[[ -z "$SBOM_SRC" ]] || SBOM_SRC="$(cd "$(dirname "$SBOM_SRC")" && pwd)/$(basename "$SBOM_SRC")"
mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
[[ "$OUT_DIR" != "$REPO_ROOT" ]] ||
    die "--out must not be the repository root: it would overwrite the source install-oneline.sh and uninstall scripts"

for tool in tar zip; do
    command -v "$tool" >/dev/null 2>&1 || die "required tool not found: $tool"
done

if command -v sha256sum >/dev/null 2>&1; then
    sha256_of() { sha256sum "$1" | awk '{print $1}'; }
elif command -v shasum >/dev/null 2>&1; then
    sha256_of() { shasum -a 256 "$1" | awk '{print $1}'; }
else
    die "no SHA-256 tool found (need sha256sum or shasum)"
fi

# Coreutils two-space format, bare filename: `sha256sum -c` works beside the
# asset and installers read the digest with `awk '{print $1}'`.
write_sidecar() {
    local file="$1"
    printf '%s  %s\n' "$(sha256_of "$file")" "$(basename "$file")" >"$file.sha256"
}

# One per release.yml matrix row; also the name install.sh resolves under bin/.
# tests/bash/release_asset_contract.test.bash diffs this against the matrix and installers.
EXPECTED_ASSETS=(
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

STAGE="$OUT_DIR/gpy-release"
rm -rf "$STAGE"
mkdir -p "$STAGE"/{bin,config,docs}

for asset in "${EXPECTED_ASSETS[@]}"; do
    src_dir="$ARTIFACTS_DIR/$asset"
    [[ -d "$src_dir" ]] || die "missing build artifact directory: $src_dir"

    # while-read, not mapfile: must run under macOS bash 3.2.
    found_count=0
    found_file=""
    while IFS= read -r candidate; do
        found_count=$((found_count + 1))
        found_file="$candidate"
    done < <(find "$src_dir" -type f | sort)
    [[ $found_count -eq 1 ]] || die "expected exactly 1 file in $src_dir, found $found_count"

    cp "$found_file" "$STAGE/bin/$asset"
    chmod +x "$STAGE/bin/$asset"

    # Sidecar lives in bin/: shipped in the archive and uploaded with bin/*.
    write_sidecar "$STAGE/bin/$asset"
done

for shell_dir in fish bash zsh; do
    [[ -d "$REPO_ROOT/$shell_dir" ]] || die "missing shell directory: $shell_dir/"
    cp -R "$REPO_ROOT/$shell_dir" "$STAGE/$shell_dir"
done

# Built-in themes/palettes are compiled in via include_str!; only the config template ships.
cp "$REPO_ROOT/config/config.example.toml" "$STAGE/config/config.example.toml"

for doc in README.md CHANGELOG.md LICENSE SECURITY.md CONTRIBUTING.md CODE_OF_CONDUCT.md; do
    [[ -f "$REPO_ROOT/$doc" ]] || die "missing documentation file: $doc"
    cp "$REPO_ROOT/$doc" "$STAGE/docs/$doc"
done
[[ -f "$REPO_ROOT/docs/INSTALL.md" ]] || die "missing documentation file: docs/INSTALL.md"
cp "$REPO_ROOT/docs/INSTALL.md" "$STAGE/docs/INSTALL.md"
[[ -d "$REPO_ROOT/docs/user" ]] || die "missing documentation directory: docs/user/"
cp -R "$REPO_ROOT/docs/user" "$STAGE/docs/user"

if [[ -n "$SBOM_SRC" ]]; then
    cp "$SBOM_SRC" "$STAGE/sbom.cdx.json"
fi

cp "$REPO_ROOT/install.sh" "$STAGE/install.sh"
chmod +x "$STAGE/install.sh"

mkdir -p "$STAGE/scripts"
cp "$REPO_ROOT/scripts/uninstall.sh" "$STAGE/scripts/"
chmod +x "$STAGE/scripts/uninstall.sh"

cd "$OUT_DIR"
rm -f gpy-release.tar.gz gpy-release.zip gpy-shell-files.tar.gz gpy-sbom.cdx.json SHA256SUMS
rm -f install-oneline.sh uninstall.sh
rm -f ./*.sha256
tar -czf gpy-release.tar.gz gpy-release/
zip -qr gpy-release.zip gpy-release/

# Stamp the tag so the asset installs the release it was published with.
sed "s/^RELEASE_VERSION=\"\"\$/RELEASE_VERSION=\"$VERSION\"/" "$REPO_ROOT/install-oneline.sh" >install-oneline.sh
grep -qxF "RELEASE_VERSION=\"$VERSION\"" install-oneline.sh ||
    die "failed to stamp $VERSION into install-oneline.sh (RELEASE_VERSION=\"\" line missing from the source)"
chmod +x install-oneline.sh

# Shell files the installer fetches as one checksummed archive.
tar -czf gpy-shell-files.tar.gz -C gpy-release fish zsh bash

cp "$REPO_ROOT/scripts/uninstall.sh" .
chmod +x uninstall.sh

# Binaries were already checksummed in gpy-release/bin/ (sidecars ship in the archive).
PUBLISHED=(gpy-release.tar.gz gpy-release.zip gpy-shell-files.tar.gz install-oneline.sh uninstall.sh)

if [[ -n "$SBOM_SRC" ]]; then
    cp "$SBOM_SRC" gpy-sbom.cdx.json
    PUBLISHED+=(gpy-sbom.cdx.json)
fi

for file in "${PUBLISHED[@]}"; do
    write_sidecar "$file"
done

{
    for file in "${PUBLISHED[@]}"; do
        printf '%s  %s\n' "$(sha256_of "$file")" "$file"
    done
    for asset in "${EXPECTED_ASSETS[@]}"; do
        printf '%s  %s\n' "$(sha256_of "gpy-release/bin/$asset")" "$asset"
    done
} >SHA256SUMS

# Assert against the archives, not the staging directory.
RELEASE_REQUIRED=(
    install.sh
    scripts/uninstall.sh
    fish/core/init.fish
    fish/conf.d/gpy_init.fish
    fish/functions/fish_prompt.fish
    fish/segments/git.fish
    fish/completions/gpy-dynamic.fish
    bash/gpy.bash
    bash/core/init.bash
    bash/segments/git.bash
    zsh/gpy.zsh
    zsh/core/init.zsh
    zsh/segments/git.zsh
    config/config.example.toml
    docs/README.md
    docs/LICENSE
    docs/CHANGELOG.md
    docs/INSTALL.md
    docs/user/configuration-reference.md
)
for asset in "${EXPECTED_ASSETS[@]}"; do
    RELEASE_REQUIRED+=("bin/$asset" "bin/$asset.sha256")
done
[[ -z "$SBOM_SRC" ]] || RELEASE_REQUIRED+=(sbom.cdx.json)

SHELL_FILES_REQUIRED=(
    fish/core/init.fish
    fish/conf.d/gpy_init.fish
    fish/functions/fish_prompt.fish
    fish/segments/git.fish
    fish/completions/gpy-dynamic.fish
    bash/gpy.bash
    bash/core/init.bash
    bash/segments/git.bash
    bash/completions/gpy-dynamic.bash
    zsh/gpy.zsh
    zsh/core/init.zsh
    zsh/segments/git.zsh
    zsh/completions/_gpy-dynamic
)

tar_listing="$(tar -tzf gpy-release.tar.gz)"
zip_listing="$(unzip -Z1 gpy-release.zip 2>/dev/null || zipinfo -1 gpy-release.zip)"
shell_files_listing="$(tar -tzf gpy-shell-files.tar.gz)"

missing=0
require_entry() {
    local listing="$1" archive="$2" entry="$3"
    if ! grep -qxF "$entry" <<<"$listing"; then
        echo "✗ $archive is missing $entry" >&2
        missing=$((missing + 1))
    fi
}

for entry in "${RELEASE_REQUIRED[@]}"; do
    require_entry "$tar_listing" gpy-release.tar.gz "gpy-release/$entry"
    require_entry "$zip_listing" gpy-release.zip "gpy-release/$entry"
done
for entry in "${SHELL_FILES_REQUIRED[@]}"; do
    require_entry "$shell_files_listing" gpy-shell-files.tar.gz "$entry"
done

[[ $missing -eq 0 ]] || die "$missing expected file(s) missing from the release archives"

# A stale digest would make every installer refuse to install.
verify_sidecar() {
    local file="$1" expected actual
    [[ -f "$file.sha256" ]] || die "missing checksum sidecar for $file"
    expected="$(awk '{print $1}' "$file.sha256")"
    actual="$(sha256_of "$file")"
    [[ "$expected" == "$actual" ]] ||
        die "checksum sidecar for $file does not match its contents ($expected vs $actual)"
}

for file in "${PUBLISHED[@]}"; do
    verify_sidecar "$file"
done
for asset in "${EXPECTED_ASSETS[@]}"; do
    verify_sidecar "gpy-release/bin/$asset"
done

expected_manifest_lines=$((${#PUBLISHED[@]} + ${#EXPECTED_ASSETS[@]}))
manifest_lines="$(wc -l <SHA256SUMS | tr -d ' ')"
[[ "$manifest_lines" -eq "$expected_manifest_lines" ]] ||
    die "SHA256SUMS has $manifest_lines entries, expected $expected_manifest_lines"

echo "Packaged $VERSION in $OUT_DIR:"
echo "  gpy-release.tar.gz  ($(tar -tzf gpy-release.tar.gz | wc -l | tr -d ' ') entries)"
echo "  gpy-release.zip     ($(wc -l <<<"$zip_listing" | tr -d ' ') entries)"
echo "  gpy-shell-files.tar.gz ($(wc -l <<<"$shell_files_listing" | tr -d ' ') entries)"
echo "  install-oneline.sh, uninstall.sh"
if [[ -n "$SBOM_SRC" ]]; then
    echo "  gpy-sbom.cdx.json   (CycloneDX)"
else
    echo "  (no SBOM: --no-sbom)"
fi
echo "  SHA256SUMS          ($manifest_lines entries)"
