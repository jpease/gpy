#!/usr/bin/env bash
# Fail if any tracked file (git grep) leaks a real user's home directory, a
# private project codename, a reference to the private issue tracker, or a
# hardcoded socket path under someone's home.

set -euo pipefail
cd "$(dirname "$0")/.."

# Synthetic example/fixture users, matched as a WHOLE path component ("u"
# allows /home/u, not /home/ursula).
ALLOWLIST_USERS=(alice foo me u user user_name)

# Private codenames are never stored in the repo (listing them would publish
# them): one per line from $GPY_PRIVATE_IDENTIFIERS_FILE, default the untracked
# .git/info/private-identifiers. Without the file the check is skipped.
PRIVATE_IDENTIFIERS=()
IDENTIFIERS_FILE="${GPY_PRIVATE_IDENTIFIERS_FILE:-$(git rev-parse --git-path info/private-identifiers)}"
if [[ -f "$IDENTIFIERS_FILE" ]]; then
    while IFS= read -r line; do
        [[ -z "$line" || "$line" == \#* ]] && continue
        PRIVATE_IDENTIFIERS+=("$line")
    done <"$IDENTIFIERS_FILE"
fi

# The patterns above are data in these files.
SELF_PATHS='scripts/check-privacy-patterns\.sh|tests/bash/privacy_patterns\.test\.bash'

failures=0

report() {
    echo "✗ $1"
    printf '%s\n' "$2" | sed 's/^/  /'
    failures=$((failures + 1))
}

# -o per occurrence, so an allowlisted example cannot mask a real path on the same line.
scan_user_paths() {
    local allow hits
    allow="$(IFS='|'; echo "${ALLOWLIST_USERS[*]}")"

    hits="$(git grep -n -I -oE '/(Users|home)/[A-Za-z0-9_][A-Za-z0-9_-]*' -- . 2>/dev/null |
        grep -Ev "^($SELF_PATHS):" |
        grep -Ev ":/(Users|home)/($allow)\$" || true)"

    [[ -n "$hits" ]] && report "Absolute home-directory path (allowlist: ${ALLOWLIST_USERS[*]}):" "$hits"
    return 0
}

scan_private_identifiers() {
    local ids hits
    [[ ${#PRIVATE_IDENTIFIERS[@]} -eq 0 ]] && return 0
    ids="$(IFS='|'; echo "${PRIVATE_IDENTIFIERS[*]}")"

    hits="$(git grep -n -I -iE "$ids" -- . 2>/dev/null |
        grep -Ev "^($SELF_PATHS):" || true)"

    # Never echo the names themselves: CI logs and terminals may be shared.
    [[ -n "$hits" ]] && report "Private project identifier (${#PRIVATE_IDENTIFIERS[@]} configured):" "$hits"
    return 0
}

scan_socket_paths() {
    local hits
    hits="$(git grep -n -I -E 'UNIX-CONNECT:/(Users|home)/' -- . 2>/dev/null |
        grep -Ev "^($SELF_PATHS):" || true)"

    [[ -n "$hits" ]] && report "Hardcoded workstation socket path:" "$hits"
    return 0
}

# References to the private tracker or to its per-issue scratch names.
scan_private_refs() {
    local hits
    hits="$(git grep -n -I -E 'gpy-archive|gpy[0-9]{3}[.A-Za-z]' -- . 2>/dev/null |
        grep -Ev "^($SELF_PATHS):" || true)"

    [[ -n "$hits" ]] && report "Reference to the private issue tracker:" "$hits"
    return 0
}

echo "Privacy pattern scan"

scan_user_paths
scan_private_identifiers
scan_socket_paths
scan_private_refs

if [[ $failures -eq 0 ]]; then
    echo "✓ No workstation or private-project references in tracked files"
    exit 0
fi

echo "$failures privacy check(s) failed" >&2
exit 1
