#!/usr/bin/env bash
# tests/bash/privacy_patterns.test.bash
#
# Regression test for #498: tracked docs linked to /Users/jpease/... and named
# a private client project, which would have shipped to a public audience at
# first publication.
#
# This exercises scripts/check-privacy-patterns.sh against throwaway git
# repositories rather than re-implementing its scan, so the gate itself is
# under test. Asserting only that the real tree is clean would pass just as
# happily against a scanner that always exits 0.
#
# It asserts:
#   (a) a planted workstation path, private identifier, and hardcoded socket
#       are each detected, and the offending path is named in the output
#   (b) allowlisted synthetic names (/Users/alice, /home/user) are accepted
#   (c) the allowlist matches a whole path component -- /Users/megan is NOT
#       excused by the "me" entry, nor /Users/ursula by "u"
#   (d) untracked files are ignored, since the gate is about published history
#   (e) the real repository is clean

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCANNER="$ROOT/scripts/check-privacy-patterns.sh"

WORKDIR="$(mktemp -d)"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

failures=0
fail() {
    echo "FAIL: $*"
    failures=$((failures + 1))
}

# Build a scratch repo containing $2 as docs/sample.md, run the scanner in it,
# and echo "<exit>|<output>". The scanner resolves the repo root as
# dirname($0)/.., so it is copied to scripts/ inside the scratch repo.
run_scanner_on() {
    local content="$1"
    local repo="$WORKDIR/repo-$RANDOM$RANDOM"

    mkdir -p "$repo/scripts" "$repo/docs"
    cp "$SCANNER" "$repo/scripts/check-privacy-patterns.sh"
    printf '%s\n' "$content" >"$repo/docs/sample.md"

    git -C "$repo" init -q
    git -C "$repo" add -A
    # Commit is not required -- git grep reads the index -- but staging is,
    # so an untracked-file case must skip the `git add` above.

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
expect_flagged "workstation path" 'See [config](/Users/jpease/Developer/gpy/config.toml).'
expect_flagged "linux workstation path" 'Run from /home/jpease/src/gpy.'
expect_flagged "private identifier" 'Observed in the bethel client repository.'
expect_flagged "private codename" 'Reproduced against asapcore only.'
expect_flagged "hardcoded socket" 'socat - UNIX-CONNECT:/Users/jpease/.cache/gpy/gpy.sock'

# (b) deliberate synthetic examples stay usable
echo "--- allowlisted synthetic examples must pass ---"
expect_clean "synthetic macOS user" 'For example /Users/alice/work/project'
expect_clean "synthetic linux user" 'For example /home/user/project'
expect_clean "short synthetic user" 'let root = Path::new("/home/u/dev/x/myrepo");'
expect_clean "underscored synthetic user" 'For example /home/user_name/project'

# (c) the allowlist must match a whole path component, not a prefix.
# A substring allowlist would excuse every real username beginning with an
# allowlisted one, which is the failure mode this case pins down.
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
echo 'leak at /Users/jpease/secret' >"$UNTRACKED_REPO/docs/scratch.md"
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

if [[ $failures -ne 0 ]]; then
    echo "$failures assertion(s) failed"
    exit 1
fi

echo "PASS"
