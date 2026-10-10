#!/usr/bin/env bash
# Build the GPY release archives from downloaded build artifacts.
#
# Extracted from .github/workflows/release.yml (gpy#492) so the packaging
# logic is testable locally (tests/bash/release_packaging.test.bash) instead
# of only being exercised by a tag push. The workflow's inline version copied
# root-level fish paths that moved into fish/ long ago, and every copy was
# suffixed with `2>/dev/null || echo "... may be missing"`, so a release could
# ship an archive with no shell files at all and still go green.
#
# Everything here is fail-closed: a missing source file, a missing build
# artifact, or an archive that does not contain the expected entries aborts
# the run.
#
# Usage:
#   scripts/package-release.sh --version v0.1.0 \
#       --artifacts release-artifacts [--out .] \
#       (--sbom sbom.cdx.json | --no-sbom)
#
# --artifacts points at the directory actions/download-artifact@v4 wrote into,
# where each artifact is its own subdirectory named after the release asset:
#   release-artifacts/gpy-agent-linux-x86_64/gpy-agent
#   release-artifacts/gpy-linux-x86_64/gpy
#
# --sbom takes the CycloneDX document from scripts/generate-sbom.sh. It is
# required for a real release; --no-sbom exists for local runs that only care
# about archive contents. Nothing is optional by omission (gpy#494).
#
# Produces, in --out:
#   gpy-release.tar.gz / gpy-release.zip  (binaries + all three shells + docs)
#   fisher-gpy.tar.gz                     (Fish plugin tree for Fisher)
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

BARE_VERSION="${VERSION#v}"

ARTIFACTS_DIR="$(cd "$ARTIFACTS_DIR" && pwd)"
[[ -z "$SBOM_SRC" ]] || SBOM_SRC="$(cd "$(dirname "$SBOM_SRC")" && pwd)/$(basename "$SBOM_SRC")"
mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"

for tool in tar zip; do
    command -v "$tool" >/dev/null 2>&1 || die "required tool not found: $tool"
done

# Checksums are the whole point of the verified-download contract (#494), so
# resolve the hashing tool once and fail here rather than silently shipping a
# release with no verification metadata -- which is exactly the state the
# installers used to tolerate.
if command -v sha256sum >/dev/null 2>&1; then
    sha256_of() { sha256sum "$1" | awk '{print $1}'; }
elif command -v shasum >/dev/null 2>&1; then
    sha256_of() { shasum -a 256 "$1" | awk '{print $1}'; }
else
    die "no SHA-256 tool found (need sha256sum or shasum)"
fi

# Sidecars use the coreutils two-space format and name the file without a
# directory component, so `sha256sum -c <asset>.sha256` works from the
# directory the asset lives in, and `awk '{print $1}'` (what the installers
# use) yields the bare digest.
write_sidecar() {
    local file="$1"
    printf '%s  %s\n' "$(sha256_of "$file")" "$(basename "$file")" >"$file.sha256"
}

# Release assets, one per row of the build matrix in release.yml. Each name is
# both the upload-artifact name (so the download lands in a directory of that
# name) and the filename install.sh looks for under bin/ -- install.sh resolves
# "bin/gpy-agent-linux-x86_64", not "bin/gpy-agent", so the binaries have to be
# renamed to their asset names on the way into the package.
#
# This list is not the authority for those names, and nothing here is allowed
# to drift from the matrix or the installers: tests/bash/release_asset_contract.test.bash
# parses all four files and diffs them against each other (#493).
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
FISHER_STAGE="$OUT_DIR/fisher-gpy"
rm -rf "$STAGE" "$FISHER_STAGE"
mkdir -p "$STAGE"/{bin,config,docs}

# --- binaries ---------------------------------------------------------------

for asset in "${EXPECTED_ASSETS[@]}"; do
    src_dir="$ARTIFACTS_DIR/$asset"
    [[ -d "$src_dir" ]] || die "missing build artifact directory: $src_dir"

    # upload-artifact uploaded exactly one file per asset; take it whatever
    # the compiler named it (gpy-agent, gpy-agent.exe, ...) and rename to the
    # asset name. Read with a while loop rather than mapfile so this still runs
    # under macOS's system bash 3.2.
    found_count=0
    found_file=""
    while IFS= read -r candidate; do
        found_count=$((found_count + 1))
        found_file="$candidate"
    done < <(find "$src_dir" -type f | sort)
    [[ $found_count -eq 1 ]] || die "expected exactly 1 file in $src_dir, found $found_count"

    cp "$found_file" "$STAGE/bin/$asset"
    chmod +x "$STAGE/bin/$asset"

    # One sidecar per binary, written into bin/ so the same file serves both
    # consumers: it travels inside the archive (install.sh verifies
    # bin/<asset>.sha256 before installing) and the release job uploads
    # bin/* wholesale, which is where install-oneline.sh fetches
    # <asset>.sha256 from (#494).
    write_sidecar "$STAGE/bin/$asset"
done

# --- shell integrations -----------------------------------------------------

# Copied whole: install.sh expects the
# fish/{core,segments,functions,conf.d,completions} tree intact, and the
# bash/zsh trees are self-contained entry point + core/ + segments/.
for shell_dir in fish bash zsh; do
    [[ -d "$REPO_ROOT/$shell_dir" ]] || die "missing shell directory: $shell_dir/"
    cp -R "$REPO_ROOT/$shell_dir" "$STAGE/$shell_dir"
done

# --- configuration ----------------------------------------------------------

# Only the example config ships. The built-in themes and palettes under
# config/{themes,palettes} are compiled into the agent via include_str!
# (gpy-agent/src/config/defaults.rs), so shipping copies would invite users to
# edit files the binary never reads.
cp "$REPO_ROOT/config/config.example.toml" "$STAGE/config/config.example.toml"

# --- documentation ----------------------------------------------------------

for doc in README.md CHANGELOG.md LICENSE SECURITY.md CONTRIBUTING.md CODE_OF_CONDUCT.md; do
    [[ -f "$REPO_ROOT/$doc" ]] || die "missing documentation file: $doc"
    cp "$REPO_ROOT/$doc" "$STAGE/docs/$doc"
done
[[ -f "$REPO_ROOT/docs/INSTALL.md" ]] || die "missing documentation file: docs/INSTALL.md"
cp "$REPO_ROOT/docs/INSTALL.md" "$STAGE/docs/INSTALL.md"
[[ -d "$REPO_ROOT/docs/user" ]] || die "missing documentation directory: docs/user/"
cp -R "$REPO_ROOT/docs/user" "$STAGE/docs/user"

# --- SBOM --------------------------------------------------------------------

# Shipped both inside the archive and as a standalone release asset: an
# archive user should be able to answer "what is in this build" from the files
# they already have, without going back to the release page (#494).
if [[ -n "$SBOM_SRC" ]]; then
    cp "$SBOM_SRC" "$STAGE/sbom.cdx.json"
fi

# --- installer --------------------------------------------------------------

# install.sh (not install-oneline.sh) is the archive installer: it installs
# from the package's own bin/ and fish/ directories. install-oneline.sh is the
# curl|sh entry point and downloads everything from GitHub, so it has no use
# inside an archive that already contains the files.
cp "$REPO_ROOT/install.sh" "$STAGE/install.sh"
chmod +x "$STAGE/install.sh"

# The uninstallers ship beside the installer so a user who installed from
# the archive has the complete removal path (#642). uninstall.fish is the
# Fish one; uninstall.sh handles the Zsh/Bash integration.
mkdir -p "$STAGE/scripts"
cp "$REPO_ROOT/scripts/uninstall.fish" "$REPO_ROOT/scripts/uninstall.sh" "$STAGE/scripts/"
chmod +x "$STAGE/scripts/uninstall.fish" "$STAGE/scripts/uninstall.sh"

# --- Fisher plugin ----------------------------------------------------------

# Fisher installs from the plugin tree itself (conf.d/, functions/,
# completions/), so the package is fish/ hoisted to the top level rather than a
# hand-rolled directory list. fisher.json is metadata only; its version is
# stamped from the release tag so the published plugin never claims a stale
# version the way the workflow's previous hardcoded "1.0.0" manifest did.
cp -R "$REPO_ROOT/fish" "$FISHER_STAGE"
cp "$REPO_ROOT/README.md" "$REPO_ROOT/LICENSE" "$FISHER_STAGE/"

sed -i.bak "s/\"version\": *\"[^\"]*\"/\"version\": \"$BARE_VERSION\"/" "$FISHER_STAGE/fisher.json"
rm -f "$FISHER_STAGE/fisher.json.bak"
grep -q "\"version\": \"$BARE_VERSION\"" "$FISHER_STAGE/fisher.json" ||
    die "failed to stamp version $BARE_VERSION into fisher.json"
grep -q '"license": "GPL-3.0-or-later"' "$FISHER_STAGE/fisher.json" ||
    die "fisher.json does not declare GPL-3.0-or-later"

# --- archives ---------------------------------------------------------------

cd "$OUT_DIR"
rm -f gpy-release.tar.gz gpy-release.zip fisher-gpy.tar.gz gpy-sbom.cdx.json SHA256SUMS
rm -f ./*.sha256
tar -czf gpy-release.tar.gz gpy-release/
zip -qr gpy-release.zip gpy-release/
tar -czf fisher-gpy.tar.gz fisher-gpy/

# --- checksums ---------------------------------------------------------------

# Published files fall into two groups. The archives and the SBOM are checksummed
# here, after they exist; the binaries were checksummed in place under
# gpy-release/bin/ above, because their sidecars have to be inside the archive.
# SHA256SUMS aggregates every published file so a user can verify a whole
# release with one `sha256sum -c`, and so nothing downloadable is left without
# verification metadata.
PUBLISHED=(gpy-release.tar.gz gpy-release.zip fisher-gpy.tar.gz)

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

# --- verification -----------------------------------------------------------

# Assert against the archives, not the staging directory: the point is that
# what ships contains what the installers reach for.
RELEASE_REQUIRED=(
    install.sh
    scripts/uninstall.fish
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

FISHER_REQUIRED=(
    fisher.json
    conf.d/gpy_init.fish
    core/init.fish
    functions/fish_prompt.fish
    segments/git.fish
    completions/gpy-dynamic.fish
    LICENSE
)

tar_listing="$(tar -tzf gpy-release.tar.gz)"
zip_listing="$(unzip -Z1 gpy-release.zip 2>/dev/null || zipinfo -1 gpy-release.zip)"
fisher_listing="$(tar -tzf fisher-gpy.tar.gz)"

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
for entry in "${FISHER_REQUIRED[@]}"; do
    require_entry "$fisher_listing" fisher-gpy.tar.gz "fisher-gpy/$entry"
done

[[ $missing -eq 0 ]] || die "$missing expected file(s) missing from the release archives"

# Re-verify every sidecar against the file it describes. The installers now
# refuse to install anything whose sidecar does not match, so a packaging bug
# that wrote a stale or truncated digest would brick every install path
# instead of merely weakening it (#494).
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
echo "  fisher-gpy.tar.gz   ($(tar -tzf fisher-gpy.tar.gz | wc -l | tr -d ' ') entries)"
if [[ -n "$SBOM_SRC" ]]; then
    echo "  gpy-sbom.cdx.json   (CycloneDX)"
else
    echo "  (no SBOM: --no-sbom)"
fi
echo "  SHA256SUMS          ($manifest_lines entries)"
