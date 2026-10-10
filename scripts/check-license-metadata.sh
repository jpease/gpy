#!/usr/bin/env bash
# Fail if any license declaration in the repo disagrees with the project
# license (GPL-3.0-or-later). Catches drift like fish/fisher.json or the
# release workflow's generated Fisher manifest reverting to a stale
# MIT/Apache-2.0 string (gpy#491).

set -euo pipefail
cd "$(dirname "$0")/.."

EXPECTED="GPL-3.0-or-later"
failures=0

check_field() {
    local label="$1" file="$2" pattern="$3"
    if [[ ! -f "$file" ]]; then
        echo "✗ $label: $file not found"
        failures=$((failures + 1))
        return
    fi
    local found
    found="$(grep -oE "$pattern" "$file" | head -1 || true)"
    if [[ "$found" == *"$EXPECTED"* ]]; then
        echo "✓ $label ($file)"
    else
        echo "✗ $label ($file): expected \"$EXPECTED\", found \"${found:-<no match>}\""
        failures=$((failures + 1))
    fi
}

check_field "Cargo.toml" "gpy-agent/Cargo.toml" '^license = "[^"]*"'
check_field "Fisher manifest" "fish/fisher.json" '"license": *"[^"]*"'
# The release workflow no longer generates its own Fisher manifest (#492): it
# ships the committed fish/fisher.json, and scripts/package-release.sh refuses
# to build the Fisher archive unless that manifest still declares GPL.
check_field "Release packaging license guard" "scripts/package-release.sh" '"license": *"[^"]*"'

if [[ ! -f LICENSE ]] || ! grep -q "GNU GENERAL PUBLIC LICENSE" LICENSE || ! grep -q "Version 3" LICENSE; then
    echo "✗ LICENSE: missing or not GPLv3 text"
    failures=$((failures + 1))
else
    echo "✓ LICENSE (GPLv3 text present)"
fi

# Any other MIT/Apache/BSD-style declaration outside dependency manifests
# (Cargo.lock, target/, deny.toml's allowlist of *dependency* licenses) means
# something declares a license other than the project's.
stray="$(find . \
    \( -name target -o -name .git -o -name node_modules \) -prune -o \
    \( -name '*.json' -o -name '*.toml' -o -name '*.rb' \) -print | \
    grep -v -e 'Cargo.lock' -e 'deny.toml' | \
    xargs grep -nE '"license"[[:space:]]*:[[:space:]]*"(MIT|Apache-2.0|BSD-[23]-Clause)"|^license = "(MIT|Apache-2.0|BSD-[23]-Clause)"' 2>/dev/null || true)"
if [[ -n "$stray" ]]; then
    echo "✗ Stray non-GPL license declaration(s) found:"
    echo "$stray"
    failures=$((failures + 1))
fi

if [[ $failures -eq 0 ]]; then
    echo "All license declarations agree on $EXPECTED"
    exit 0
else
    echo "$failures license metadata check(s) failed"
    exit 1
fi
