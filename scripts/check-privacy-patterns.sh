#!/usr/bin/env bash
# Fail if any tracked file leaks maintainer-workstation or private-client
# information: an absolute home directory naming a real user, a private project
# codename, or a hardcoded socket path under someone's home (gpy#498).
#
# Scans tracked files only, via git grep, so build artifacts and local scratch
# files can never trip the gate.

set -euo pipefail
cd "$(dirname "$0")/.."

# Synthetic user names that are deliberate documentation examples and test
# fixtures. Each is matched as a WHOLE path component -- "u" allows /home/u but
# not /home/ursula -- so widening this list cannot silently swallow a real
# username that merely starts with one of these strings.
#
# Every entry below is in current use; verify with
#   git grep -hoE '/(Users|home)/[A-Za-z0-9_-]+' | sort -u
# before adding another.
ALLOWLIST_USERS=(alice foo me u user user_name)

# Private client, employer, and project codenames that must never appear.
PRIVATE_IDENTIFIERS=(bethel asapcore asapbelt fieldjoy)

# This scanner and its test carry the patterns above as data, so they are
# excluded from their own scan.
SELF_PATHS='scripts/check-privacy-patterns\.sh|tests/bash/privacy_patterns\.test\.bash'

failures=0

report() {
    echo "✗ $1"
    printf '%s\n' "$2" | sed 's/^/  /'
    failures=$((failures + 1))
}

# Absolute home-directory paths naming a real user.
#
# Matches are extracted per-occurrence with -o rather than per-line, so a line
# holding both an allowlisted example and a real path still reports the real
# one instead of being dropped wholesale.
scan_user_paths() {
    local allow hits
    allow="$(IFS='|'; echo "${ALLOWLIST_USERS[*]}")"

    hits="$(git grep -n -I -oE '/(Users|home)/[A-Za-z0-9_][A-Za-z0-9_-]*' -- . 2>/dev/null |
        grep -Ev "^($SELF_PATHS):" |
        grep -Ev ":/(Users|home)/($allow)\$" || true)"

    [[ -n "$hits" ]] && report "Absolute home-directory path (allowlist: ${ALLOWLIST_USERS[*]}):" "$hits"
    return 0
}

# Private client/project codenames.
scan_private_identifiers() {
    local ids hits
    ids="$(IFS='|'; echo "${PRIVATE_IDENTIFIERS[*]}")"

    hits="$(git grep -n -I -iE "$ids" -- . 2>/dev/null |
        grep -Ev "^($SELF_PATHS):" || true)"

    [[ -n "$hits" ]] && report "Private project identifier (${PRIVATE_IDENTIFIERS[*]}):" "$hits"
    return 0
}

# Sockets addressed under a specific home directory rather than resolved from
# XDG_CACHE_HOME/HOME at runtime.
scan_socket_paths() {
    local hits
    hits="$(git grep -n -I -E 'UNIX-CONNECT:/(Users|home)/' -- . 2>/dev/null |
        grep -Ev "^($SELF_PATHS):" || true)"

    [[ -n "$hits" ]] && report "Hardcoded workstation socket path:" "$hits"
    return 0
}

echo "Privacy pattern scan"

scan_user_paths
scan_private_identifiers
scan_socket_paths

if [[ $failures -eq 0 ]]; then
    echo "✓ No workstation or private-project references in tracked files"
    exit 0
fi

echo "$failures privacy check(s) failed" >&2
exit 1
