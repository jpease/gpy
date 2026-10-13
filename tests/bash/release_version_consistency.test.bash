#!/usr/bin/env bash
# tests/bash/release_version_consistency.test.bash
#
# Every version declaration agrees with the newest released CHANGELOG
# section, and scripts/release-notes.sh renders notes for it.
#
# Asserts:
#   (a) gpy-agent/Cargo.toml, fish/fisher.json and the newest released
#       CHANGELOG heading declare the same version
#   (b) that heading has a link reference pointing at its release tag
#   (c) its date is an ISO date or the literal YYYY-MM-DD placeholder the
#       release procedure replaces
#   (d) scripts/release-notes.sh renders non-empty notes for that version

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

CHANGELOG="CHANGELOG.md"
MANIFEST="gpy-agent/Cargo.toml"
FISHER="fish/fisher.json"

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

# --- (a) every version declaration agrees -----------------------------------
echo "--- version declarations agree ---"
CARGO_VERSION="$(grep -m1 '^version = ' "$MANIFEST" | sed 's/version = "\(.*\)"/\1/')"
FISHER_VERSION="$(grep -m1 '"version"' "$FISHER" | sed 's/.*"version": *"\([^"]*\)".*/\1/')"

for pair in "$MANIFEST:$CARGO_VERSION" "$FISHER:$FISHER_VERSION"; do
    where="${pair%%:*}"
    what="${pair#*:}"
    if [[ -z "$what" ]]; then
        fail "could not read a version from $where"
    elif [[ "$what" != "$CHANGELOG_VERSION" ]]; then
        fail "$where declares $what but $CHANGELOG declares $CHANGELOG_VERSION"
    fi
done

# --- (b) the release heading has a link reference ---------------------------
echo "--- CHANGELOG links the released version to its tag ---"
grep -qE "^\[$CHANGELOG_VERSION\]:.*releases/tag/v$CHANGELOG_VERSION" "$CHANGELOG" ||
    fail "$CHANGELOG has no [$CHANGELOG_VERSION] link reference pointing at the v$CHANGELOG_VERSION release tag"

# --- (c) the release date is an ISO date or the placeholder -----------------
echo "--- release heading carries a date or the placeholder ---"
if [[ ! "$release_heading" =~ ^\#\#\ \[[0-9]+\.[0-9]+\.[0-9]+\]\ -\ (YYYY-MM-DD|[0-9]{4}-[0-9]{2}-[0-9]{2})$ ]]; then
    fail "release heading '$release_heading' is neither an ISO date nor the YYYY-MM-DD placeholder"
fi

# --- (d) release notes render for that version ------------------------------
echo "--- release notes render ---"
notes="$(scripts/release-notes.sh --version "v$CHANGELOG_VERSION" 2>&1)" || {
    fail "scripts/release-notes.sh failed for v$CHANGELOG_VERSION: $notes"
    notes=""
}
if [[ -n "$notes" ]]; then
    grep -qF "# GPY v$CHANGELOG_VERSION" <<<"$notes" ||
        fail "release notes for v$CHANGELOG_VERSION have no version heading"
fi

test_finish "version declarations agree with CHANGELOG $CHANGELOG_VERSION"
