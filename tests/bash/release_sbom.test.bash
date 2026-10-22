#!/usr/bin/env bash
# tests/bash/release_sbom.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# scripts/generate-sbom.sh: a well-formed but wrong SBOM (empty components,
# dev-dependencies listed, wrong version) is worse than none.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

for tool in cargo jq; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "SKIP: $tool not available; SBOM generation needs it"
        exit 0
    fi
done

# An empty RUSTC_WRAPPER overrides a global `build.rustc-wrapper` (as CI does),
# so a user's wrapper (e.g. a shim unresolvable under the gate's sandboxed
# XDG_CONFIG_HOME) cannot break `cargo metadata`.
export RUSTC_WRAPPER=

WORKDIR="$(mktemp -d)"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

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


expect_field() {
    local path="$1" want="$2" got
    got="$(jq -r "$path" "$SBOM")"
    [[ "$got" == "$want" ]] || fail "$path is '$got', expected '$want'"
}

expect_field '.bomFormat' 'CycloneDX'
expect_field '.specVersion' '1.5'
expect_field '.metadata.component.name' 'gpy-agent'
expect_field '.metadata.component.type' 'application'

# --version wins over Cargo.toml so the document describes the tag.
expect_field '.metadata.component.version' '9.9.9'
expect_field '.metadata.component.purl' 'pkg:cargo/gpy-agent@9.9.9'

[[ "$(jq -r '.metadata.timestamp' "$SBOM")" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T ]] ||
    fail "metadata.timestamp is not an ISO-8601 UTC timestamp"


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

# Dev-dependencies are not shipped; tempfile and tokio-test are dev-only.
for dev_only in tempfile tokio-test; do
    if ! grep -qE "^$dev_only" <<<"$(sed -n '/^\[dev-dependencies\]/,/^\[/p' "$ROOT/gpy-agent/Cargo.toml")"; then
        continue
    fi
    [[ "$(jq -r --arg n "$dev_only" '[.components[] | select(.name == $n)] | length' "$SBOM")" -eq 0 ]] ||
        fail "$dev_only is a dev-dependency but appears in the SBOM"
done

# The root package describes itself in metadata.component, not in components[].
[[ "$(jq -r '[.components[] | select(.name == "gpy-agent")] | length' "$SBOM")" -eq 0 ]] ||
    fail "gpy-agent is listed as its own dependency"


# Same SOURCE_DATE_EPOCH must give byte-identical output.
SBOM2="$WORKDIR/sbom2.cdx.json"
SOURCE_DATE_EPOCH=1700000000 "$ROOT/scripts/generate-sbom.sh" --version v9.9.9 --output "$SBOM" >/dev/null
SOURCE_DATE_EPOCH=1700000000 "$ROOT/scripts/generate-sbom.sh" --version v9.9.9 --output "$SBOM2" >/dev/null
cmp -s "$SBOM" "$SBOM2" || fail "two runs at the same SOURCE_DATE_EPOCH produced different documents"


echo "--- invalid version (must fail) ---"
if "$ROOT/scripts/generate-sbom.sh" --version 9.9.9 --output "$WORKDIR/bad.json" >/dev/null 2>&1; then
    fail "generate-sbom.sh accepted a version without the leading v"
fi

test_finish
