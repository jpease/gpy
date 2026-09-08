#!/usr/bin/env bash
# tests/bash/test_readme_claims.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The test READMEs describe the suites that exist (#655).
#
# tests/fish/README.md named a test that did not exist and a CI workflow
# that did not exist, and listed a quarter of the suite; nothing checked.
# Two directions, for tests/fish, tests/bash, tests/zsh and gpy-agent/tests:
#   (a) every `*.test.fish|bash|zsh` / `*.rs` file name a README mentions in
#       a code span exists in that suite's directory
#   (b) every test file in the suite's directory is mentioned in its README
#       (by full name, or by its bare stem inside a code span, which is how
#       the Bash/Zsh tier tables cite them)
# plus the concrete claims that were wrong: no `postexec_async_update`, no
# `.github/workflows/test.yml`, no `tests/manual/`, and `fresh_install` is
# described as sourcing the checkout, not as running an installer.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
pass() { echo "PASS: $*"; }

# `check_suite README DIR EXT` -- both directions for one suite.
check_suite() {
    local readme="$1" dir="$2" ext="$3" name stem
    local listed_missing=0 unlisted=0

    # (a) every file name of this suite's kind that the README cites exists.
    while IFS= read -r name; do
        [ -n "$name" ] || continue
        # A file may also live one level down (gpy-agent/tests/common/*.rs).
        if [ ! -f "$dir/$name" ] && ! ls "$dir"/*/"$name" >/dev/null 2>&1; then
            fail "$readme names $name, which is not in $dir"
            listed_missing=$((listed_missing + 1))
        fi
    done < <(grep -oE "\`[A-Za-z0-9_./-]+\.$ext\`" "$readme" | tr -d '`' | sed 's#.*/##' | sort -u)
    [ "$listed_missing" -eq 0 ] && pass "$readme: every $ext file it names exists"

    # (b) every file in the directory is mentioned.
    for path in "$dir"/*."$ext"; do
        [ -e "$path" ] || continue
        name="$(basename "$path")"
        stem="${name%%.*}"
        if ! grep -qE "\`([A-Za-z0-9_./-]*/)?($name|$stem)\`" "$readme"; then
            fail "$readme does not mention $name"
            unlisted=$((unlisted + 1))
        fi
    done
    [ "$unlisted" -eq 0 ] && pass "$readme: every $ext file in $dir is mentioned"
}

echo "--- the shell suites ---"
check_suite tests/fish/README.md tests/fish "test.fish"
check_suite tests/bash/README.md tests/bash "test.bash"
check_suite tests/zsh/README.md tests/zsh "test.zsh"

echo "--- the Rust integration targets ---"
check_suite gpy-agent/tests/README.md gpy-agent/tests "rs"

echo "--- the claims that were wrong ---"
for readme in tests/fish/README.md tests/bash/README.md tests/zsh/README.md gpy-agent/tests/README.md; do
    for stale in "postexec_async_update" "workflows/test.yml" "tests/manual/"; do
        # A mention is fine only as a denial ("There is no ...").
        if grep -qF -- "$stale" "$readme" && ! grep -F -- "$stale" "$readme" | grep -qiE "no (\`|\`\.github/|tests/)"; then
            fail "$readme still refers to $stale"
        fi
    done
done
pass "no README refers to postexec_async_update, workflows/test.yml or tests/manual/ as existing"
if grep -qF -- "Runs no installer" tests/fish/README.md; then
    pass "fresh_install is described as sourcing the checkout, not running an installer"
else
    fail "tests/fish/README.md must say fresh_install runs no installer"
fi

if [ "$failures" -gt 0 ]; then
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: the test READMEs describe the suites that exist"
