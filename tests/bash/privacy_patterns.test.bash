#!/usr/bin/env bash
# tests/bash/privacy_patterns.test.bash
# Tracked docs must not contain workstation paths or private client names.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCANNER="$ROOT/scripts/check-privacy-patterns.sh"

WORKDIR="$(mktemp -d)"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

# Synthetic private identifiers, so this test publishes nothing real.
IDENTIFIERS_FILE="$WORKDIR/private-identifiers"
printf '# comment\n\nacme-client\nprojectzeta\n' >"$IDENTIFIERS_FILE"
export GPY_PRIVATE_IDENTIFIERS_FILE="$IDENTIFIERS_FILE"

# Run the scanner in a scratch repo with $2 as docs/sample.md; echo "<exit>|<output>".
run_scanner_on() {
    local content="$1"
    local repo="$WORKDIR/repo-$RANDOM$RANDOM"

    mkdir -p "$repo/scripts" "$repo/docs"
    cp "$SCANNER" "$repo/scripts/check-privacy-patterns.sh"
    printf '%s\n' "$content" >"$repo/docs/sample.md"

    git -C "$repo" init -q
    git -C "$repo" add -A
    # git grep reads the index: staging is required, committing is not.

    local out status
    out="$(cd "$repo" && bash scripts/check-privacy-patterns.sh 2>&1)" && status=0 || status=$?
    printf '%s|%s' "$status" "$out"
}

expect_flagged() {
    local label="$1" content="$2" result
    result="$(run_scanner_on "$content")"
    [[ "${result%%|*}" -ne 0 ]] || fail "$label: scanner exited 0, expected a failure"
    [[ "${result#*|}" == *"docs/sample.md"* ]] ||
        fail "$label: output does not name the offending file"
}

expect_clean() {
    local label="$1" content="$2" result
    result="$(run_scanner_on "$content")"
    [[ "${result%%|*}" -eq 0 ]] ||
        fail "$label: scanner exited nonzero on acceptable content: ${result#*|}"
}

# (a) each pattern class is detected
echo "--- planted leaks must be flagged ---"
expect_flagged "workstation path" 'See [config](/Users/mallory/Developer/gpy/config.toml).'
expect_flagged "linux workstation path" 'Run from /home/mallory/src/gpy.'
expect_flagged "private identifier" 'Observed in the acme-client repository.'
expect_flagged "private codename" 'Reproduced against ProjectZeta only.'

expect_flagged "private tracker reference" 'Measured in jpease/gpy-archive#977.'
expect_flagged "private scratch prefix" 'mktemp -d /tmp/gpy619.XXXXXX'

# Without the identifiers file the check is skipped rather than failing.
GPY_PRIVATE_IDENTIFIERS_FILE="$WORKDIR/missing" expect_clean "no identifiers file" 'Observed in the acme-client repository.'
expect_flagged "hardcoded socket" 'socat - UNIX-CONNECT:/Users/mallory/.cache/gpy/gpy.sock'

# (b) deliberate synthetic examples stay usable
echo "--- allowlisted synthetic examples must pass ---"
expect_clean "synthetic macOS user" 'For example /Users/alice/work/project'
expect_clean "synthetic linux user" 'For example /home/user/project'
expect_clean "short synthetic user" 'let root = Path::new("/home/u/dev/x/myrepo");'
expect_clean "underscored synthetic user" 'For example /home/user_name/project'

# (c) the allowlist matches whole path components, not prefixes.
echo "--- allowlist must not leak via prefix matching ---"
expect_flagged "megan vs me" 'Config lives at /Users/megan/.config/gpy.'
expect_flagged "ursula vs u" 'Config lives at /Users/ursula/.config/gpy.'
expect_flagged "foobar vs foo" 'Config lives at /Users/foobar/.config/gpy.'
expect_flagged "username vs user" 'Config lives at /home/username/.config/gpy.'

# (d) untracked files are out of scope: the gate guards published history.
echo "--- untracked files are ignored ---"
UNTRACKED_REPO="$WORKDIR/untracked"
mkdir -p "$UNTRACKED_REPO/scripts" "$UNTRACKED_REPO/docs"
cp "$SCANNER" "$UNTRACKED_REPO/scripts/check-privacy-patterns.sh"
git -C "$UNTRACKED_REPO" init -q
echo 'leak at /Users/mallory/secret' >"$UNTRACKED_REPO/docs/scratch.md"
if ! (cd "$UNTRACKED_REPO" && bash scripts/check-privacy-patterns.sh >/dev/null 2>&1); then
    fail "scanner flagged an untracked file; it must scan tracked files only"
fi

# (e) the real repository is clean
echo "--- the repository itself must be clean ---"
if ! bash "$SCANNER" >/dev/null 2>&1; then
    echo "--- scanner output ---"
    bash "$SCANNER" 2>&1 | sed 's/^/  /'
    fail "the repository has privacy-pattern violations"
fi

test_finish
