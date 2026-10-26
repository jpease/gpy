#!/usr/bin/env bash
# Fail if any license declaration in the repo disagrees with the project
# license (GPL-3.0-or-later).

set -euo pipefail
cd "$(dirname "$0")/.."

EXPECTED="GPL-3.0-or-later"
failures=0

check_field() {
    local label="$1" file="$2" pattern="$3"
    if [[ ! -f "$file" ]]; then
        echo "FAIL: $label: $file not found"
        failures=$((failures + 1))
        return
    fi
    local found
    found="$(grep -oE "$pattern" "$file" | head -1 || true)"
    if [[ "$found" == *"$EXPECTED"* ]]; then
        echo "ok: $label ($file)"
    else
        echo "FAIL: $label ($file): expected \"$EXPECTED\", found \"${found:-<no match>}\""
        failures=$((failures + 1))
    fi
}

check_field "Cargo.toml" "gpy-agent/Cargo.toml" '^license = "[^"]*"'

if [[ ! -f LICENSE ]] || ! grep -q "GNU GENERAL PUBLIC LICENSE" LICENSE || ! grep -q "Version 3" LICENSE; then
    echo "FAIL: LICENSE: missing or not GPLv3 text"
    failures=$((failures + 1))
else
    echo "ok: LICENSE (GPLv3 text present)"
fi

# Non-GPL declarations outside dependency manifests (deny.toml lists *dependency* licenses).
stray="$(find . \
    \( -name target -o -name .git -o -name node_modules \) -prune -o \
    \( -name '*.json' -o -name '*.toml' -o -name '*.rb' \) -print | \
    grep -v -e 'Cargo.lock' -e 'deny.toml' | \
    xargs grep -nE '"license"[[:space:]]*:[[:space:]]*"(MIT|Apache-2.0|BSD-[23]-Clause)"|^license = "(MIT|Apache-2.0|BSD-[23]-Clause)"' 2>/dev/null || true)"
if [[ -n "$stray" ]]; then
    echo "FAIL: Stray non-GPL license declaration(s) found:"
    echo "$stray"
    failures=$((failures + 1))
fi

if [[ $failures -eq 0 ]]; then
    echo "ok: all license declarations agree on $EXPECTED"
    exit 0
else
    echo "$failures license metadata check(s) failed"
    exit 1
fi
