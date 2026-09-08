#!/usr/bin/env bash
# tests/bash/release_sbom.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Covers scripts/generate-sbom.sh (#494). The SBOM is a release asset that
# downstream consumers use to answer "is GPY affected by CVE-X", so a document
# that is well-formed but wrong -- an empty component list, dev-dependencies
# presented as shipped code, a version that disagrees with the tag -- is worse
# than none at all.
#
# Generating it from `cargo metadata` rather than a network-installed tool is
# what makes this test possible: the release step runs on every quality-check,
# not only on a tag push.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

for tool in cargo jq; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "SKIP: $tool not available; SBOM generation needs it"
        exit 0
    fi
done

WORKDIR="$(mktemp -d)"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

failures=0
fail() {
    echo "FAIL: $*"
    failures=$((failures + 1))
}

SBOM="$WORKDIR/sbom.cdx.json"

echo "--- generating an SBOM for v9.9.9 ---"
if ! "$ROOT/scripts/generate-sbom.sh" --version v9.9.9 --output "$SBOM" >/dev/null; then
    echo "FAIL: generate-sbom.sh exited non-zero"
    exit 1
fi

jq empty "$SBOM" 2>/dev/null || {
    echo "FAIL: the SBOM is not valid JSON"
    exit 1
}

# --- CycloneDX shape ----------------------------------------------------------

expect_field() {
    local path="$1" want="$2" got
    got="$(jq -r "$path" "$SBOM")"
    [[ "$got" == "$want" ]] || fail "$path is '$got', expected '$want'"
}

expect_field '.bomFormat' 'CycloneDX'
expect_field '.specVersion' '1.5'
expect_field '.metadata.component.name' 'gpy-agent'
expect_field '.metadata.component.type' 'application'

# --version wins over Cargo.toml so the document describes the tag it ships
# with, not whatever the manifest said at the time.
expect_field '.metadata.component.version' '9.9.9'
expect_field '.metadata.component.purl' 'pkg:cargo/gpy-agent@9.9.9'

[[ "$(jq -r '.metadata.timestamp' "$SBOM")" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T ]] ||
    fail "metadata.timestamp is not an ISO-8601 UTC timestamp"

# --- component coverage -------------------------------------------------------

component_count="$(jq '.components | length' "$SBOM")"
[[ "$component_count" -gt 20 ]] ||
    fail "SBOM lists only $component_count components; the resolve graph was not walked"

# Every component needs the fields a consumer matches advisories on.
missing_purl="$(jq '[.components[] | select(.purl == null or .name == null or .version == null)] | length' "$SBOM")"
[[ "$missing_purl" -eq 0 ]] || fail "$missing_purl component(s) are missing name, version or purl"

bad_purl="$(jq -r '[.components[] | select(.purl != ("pkg:cargo/" + .name + "@" + .version))] | length' "$SBOM")"
[[ "$bad_purl" -eq 0 ]] || fail "$bad_purl component(s) have a purl that disagrees with their name/version"

# A shipped runtime dependency must be present.
[[ "$(jq -r '[.components[] | select(.name == "tokio")] | length' "$SBOM")" -eq 1 ]] ||
    fail "tokio (a normal dependency) is missing from the SBOM"

# Dev-dependencies are not linked into a shipped binary and must not be listed
# as if they were. tempfile and tokio-test are dev-only in gpy-agent/Cargo.toml.
for dev_only in tempfile tokio-test; do
    if ! grep -qE "^$dev_only" <<<"$(sed -n '/^\[dev-dependencies\]/,/^\[/p' "$ROOT/gpy-agent/Cargo.toml")"; then
        # The manifest changed; don't assert on a crate that is no longer dev-only.
        continue
    fi
    [[ "$(jq -r --arg n "$dev_only" '[.components[] | select(.name == $n)] | length' "$SBOM")" -eq 0 ]] ||
        fail "$dev_only is a dev-dependency but appears in the SBOM"
done

# The root package describes itself in metadata.component, not in components[].
[[ "$(jq -r '[.components[] | select(.name == "gpy-agent")] | length' "$SBOM")" -eq 0 ]] ||
    fail "gpy-agent is listed as its own dependency"

# --- determinism --------------------------------------------------------------

# Two runs at the same SOURCE_DATE_EPOCH must be byte-identical, so a rebuild
# can be diffed against a published SBOM.
SBOM2="$WORKDIR/sbom2.cdx.json"
SOURCE_DATE_EPOCH=1700000000 "$ROOT/scripts/generate-sbom.sh" --version v9.9.9 --output "$SBOM" >/dev/null
SOURCE_DATE_EPOCH=1700000000 "$ROOT/scripts/generate-sbom.sh" --version v9.9.9 --output "$SBOM2" >/dev/null
cmp -s "$SBOM" "$SBOM2" || fail "two runs at the same SOURCE_DATE_EPOCH produced different documents"

# --- input validation ---------------------------------------------------------

echo "--- invalid version (must fail) ---"
if "$ROOT/scripts/generate-sbom.sh" --version 9.9.9 --output "$WORKDIR/bad.json" >/dev/null 2>&1; then
    fail "generate-sbom.sh accepted a version without the leading v"
fi

if [[ $failures -ne 0 ]]; then
    echo "$failures assertion(s) failed"
    exit 1
fi

echo "PASS"
