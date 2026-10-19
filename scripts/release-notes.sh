#!/usr/bin/env bash
# Generate GitHub release notes for a GPY tag.
#
# Everything below is derived from files in the repository at the released
# commit: the body text comes from the CHANGELOG section for the version, and
# the asset table is built from the artifacts actually downloaded for the
# release. There is nothing left to go stale silently -- a missing CHANGELOG
# section fails the run.
#
# Usage:
#   scripts/release-notes.sh --version v0.1.0 \
#       [--assets gpy-release/bin] [--output release_notes.md]
#
# --assets points at a directory of release binaries named after their assets
# (what scripts/package-release.sh writes into gpy-release/bin).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_URL="https://github.com/jpease/gpy"

VERSION=""
ASSETS_DIR=""
OUTPUT=""

die() {
    echo "release-notes: $*" >&2
    exit 1
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --version) VERSION="${2:-}"; shift 2 ;;
        --assets) ASSETS_DIR="${2:-}"; shift 2 ;;
        --output) OUTPUT="${2:-}"; shift 2 ;;
        -h|--help) sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 2 ;;
        *) die "unknown argument: $1" ;;
    esac
done

[[ -n "$VERSION" ]] || die "--version is required (e.g. --version v0.1.0)"
[[ "$VERSION" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "invalid version: $VERSION (expected vMAJOR.MINOR.PATCH)"

BARE_VERSION="${VERSION#v}"
CHANGELOG="$REPO_ROOT/CHANGELOG.md"
[[ -f "$CHANGELOG" ]] || die "CHANGELOG.md not found"

# No fallback to [Unreleased]: a tag without a CHANGELOG section must fail.
section="$(awk -v want="## [$BARE_VERSION]" '
    index($0, want) == 1 { grab = 1; next }
    grab && /^## / { exit }
    grab { print }
' "$CHANGELOG")"

# Trim leading/trailing blank lines.
section="$(printf '%s\n' "$section" | awk '
    { lines[NR] = $0 }
    END {
        start = 1; while (start <= NR && lines[start] ~ /^[[:space:]]*$/) start++
        end = NR;  while (end >= start && lines[end] ~ /^[[:space:]]*$/) end--
        for (i = start; i <= end; i++) print lines[i]
    }
')"

[[ -n "$section" ]] || die "CHANGELOG.md has no content under '## [$BARE_VERSION]' -- add the release section before tagging"

assets=""
if [[ -n "$ASSETS_DIR" ]]; then
    [[ -d "$ASSETS_DIR" ]] || die "assets directory not found: $ASSETS_DIR"
    while IFS= read -r asset_path; do
        asset_name="$(basename "$asset_path")"
        case "$asset_name" in
            *.sha256|SHA256SUMS) continue ;;
        esac
        case "$asset_name" in
            *linux*) asset_os="Linux" ;;
            *macos*) asset_os="macOS" ;;
            *windows*) asset_os="Windows" ;;
            *) die "cannot classify release asset: $asset_name" ;;
        esac
        case "$asset_name" in
            *x86_64*) asset_arch="x86_64" ;;
            *aarch64*) asset_arch="aarch64" ;;
            *) die "cannot classify release asset architecture: $asset_name" ;;
        esac
        case "$asset_name" in
            gpy-agent-*) asset_kind="Agent daemon" ;;
            gpy-*) asset_kind="CLI" ;;
            *) die "cannot classify release asset kind: $asset_name" ;;
        esac
        assets+="| \`$asset_name\` | $asset_os | $asset_arch | $asset_kind |"$'\n'
    done < <(find "$ASSETS_DIR" -mindepth 1 -maxdepth 1 -type f | sort)
    [[ -n "$assets" ]] || die "no release assets found under $ASSETS_DIR"
fi

render() {
    cat <<EOF
# GPY $VERSION

## Changes

$section

## Installation

### One-line installer (Linux, macOS)

\`\`\`sh
curl -fsSL $REPO_URL/releases/download/$VERSION/install-oneline.sh | sh
\`\`\`

The installer is an asset of this release, with this release's tag built in, so
the command above installs exactly the code this release was cut from. Its
downloads are checked against the checksums published in the same release; see
SECURITY.md for what that does and does not protect against, and for verifying
the build-provenance attestation (\`gh attestation verify\`) before running it.

Detects Fish, Zsh, or Bash and installs the agent, the \`gpy\` CLI, and the
shell integration for the detected shell.

### From the release archive

\`\`\`sh
curl -fsSLO $REPO_URL/releases/download/$VERSION/gpy-release.tar.gz
tar -xzf gpy-release.tar.gz
cd gpy-release && ./install.sh
\`\`\`

Full instructions, including building from source and uninstalling, are in the
[installation guide]($REPO_URL/blob/$VERSION/docs/INSTALL.md).

## Verifying this release

Both installers verify every binary they install against its published
\`.sha256\` sidecar and abort if the checksum is missing or does not match. To
check a download by hand:

\`\`\`sh
curl -fsSLO $REPO_URL/releases/download/$VERSION/SHA256SUMS
curl -fsSLO $REPO_URL/releases/download/$VERSION/gpy-release.tar.gz
sha256sum --ignore-missing -c SHA256SUMS
\`\`\`

Every artifact also carries a GitHub build-provenance attestation, which ties
it to the workflow run and commit that produced it:

\`\`\`sh
gh attestation verify gpy-release.tar.gz --repo jpease/gpy
\`\`\`

\`gpy-sbom.cdx.json\` is a CycloneDX SBOM of the Rust dependency graph the
binaries were built from. GPY binaries are **not** code-signed or notarized;
see [SECURITY.md]($REPO_URL/blob/$VERSION/SECURITY.md) for the rationale and
for what that means on macOS and Windows.
EOF

    if [[ -n "$assets" ]]; then
        cat <<EOF

## Binary assets

| Asset | OS | Architecture | Binary |
|-------|----|--------------|--------|
$assets
EOF
    fi

    cat <<EOF

## Documentation

- [Installation guide]($REPO_URL/blob/$VERSION/docs/INSTALL.md)
- [Configuration reference]($REPO_URL/blob/$VERSION/docs/user/configuration-reference.md)
- [Theme customization]($REPO_URL/blob/$VERSION/docs/user/theme-customization.md)
- [Troubleshooting]($REPO_URL/blob/$VERSION/docs/user/troubleshooting.md)
- [Changelog]($REPO_URL/blob/$VERSION/CHANGELOG.md)

Problems with this release? Open an issue at $REPO_URL/issues.
EOF
}

if [[ -n "$OUTPUT" ]]; then
    render >"$OUTPUT"
    echo "release-notes: wrote $OUTPUT for $VERSION"
else
    render
fi
