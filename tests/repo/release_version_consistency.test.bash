#!/usr/bin/env bash
# tests/repo/release_version_consistency.test.bash
# Every version declaration agrees with the newest released CHANGELOG
# section, and scripts/release-notes.sh renders notes for it.
# Asserts: (a) Cargo.toml and the CHANGELOG agree; (b) the heading
# has a tag link reference; (c) its date is ISO or YYYY-MM-DD; (d) notes render;
# (e) SECURITY.md names one series, the CHANGELOG minor.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

CHANGELOG="CHANGELOG.md"
MANIFEST="gpy-agent/Cargo.toml"

if [[ ! -f "$CHANGELOG" ]]; then
    fail "$CHANGELOG not found"
    test_finish
fi

release_heading="$(grep -m1 -E '^## \[[0-9]+\.[0-9]+\.[0-9]+\]' "$CHANGELOG" || true)"
CHANGELOG_VERSION="$(printf '%s\n' "$release_heading" |
    sed -nE 's/^## \[([0-9]+\.[0-9]+\.[0-9]+)\].*/\1/p')"
if [[ -z "$CHANGELOG_VERSION" ]]; then
    fail "could not read a released version from $CHANGELOG"
    test_finish
fi

echo "--- version declarations agree ---"
CARGO_VERSION="$(grep -m1 '^version = ' "$MANIFEST" | sed 's/version = "\(.*\)"/\1/')"

if [[ -z "$CARGO_VERSION" ]]; then
    fail "could not read a version from $MANIFEST"
elif [[ "$CARGO_VERSION" != "$CHANGELOG_VERSION" ]]; then
    fail "$MANIFEST declares $CARGO_VERSION but $CHANGELOG declares $CHANGELOG_VERSION"
fi

echo "--- CHANGELOG links the released version to its tag ---"
grep -qE "^\[$CHANGELOG_VERSION\]:.*releases/tag/v$CHANGELOG_VERSION" "$CHANGELOG" ||
    fail "$CHANGELOG has no [$CHANGELOG_VERSION] link reference pointing at the v$CHANGELOG_VERSION release tag"

echo "--- release heading carries a date or the placeholder ---"
if [[ ! "$release_heading" =~ ^\#\#\ \[[0-9]+\.[0-9]+\.[0-9]+\]\ -\ (YYYY-MM-DD|[0-9]{4}-[0-9]{2}-[0-9]{2})$ ]]; then
    fail "release heading '$release_heading' is neither an ISO date nor the YYYY-MM-DD placeholder"
fi

echo "--- release notes render ---"
notes="$(scripts/release-notes.sh --version "v$CHANGELOG_VERSION" 2>&1)" || {
    fail "scripts/release-notes.sh failed for v$CHANGELOG_VERSION: $notes"
    notes=""
}
if [[ -n "$notes" ]]; then
    grep -qF "# GPY v$CHANGELOG_VERSION" <<<"$notes" ||
        fail "release notes for v$CHANGELOG_VERSION have no version heading"
fi

echo "--- SECURITY.md names the released minor series ---"
expected="${CHANGELOG_VERSION%.*}.x"
mapfile -t rows < <(grep -E '^[|] [0-9]+\.[0-9]+\.x ' SECURITY.md || true)
if [[ ${#rows[@]} -ne 1 ]]; then
    fail "SECURITY.md must have exactly one '| N.N.x |' row, found ${#rows[@]}"
else
    listed="$(printf '%s\n' "${rows[0]}" | sed -nE 's/^[|] ([0-9]+\.[0-9]+\.x) .*/\1/p')"
    if [[ "$listed" != "$expected" ]]; then
        fail "SECURITY.md lists $listed but $CHANGELOG declares $CHANGELOG_VERSION (expected $expected)"
    fi
fi

test_finish "version declarations agree with CHANGELOG $CHANGELOG_VERSION"
