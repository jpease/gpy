#!/usr/bin/env bash
# tests/bash/shell_c_no_interpolation.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# No test (or test runner) builds a child shell's code string by interpolating
# a variable into a double-quoted `-c` argument (#822).
#
# A `fish -c` whose double-quoted code string embeds the repo root re-parses
# that path as code. A checkout path with a space splits the word, and one with
# an apostrophe or `$` breaks the quoting (or runs something). The zsh/fish/bash
# e2e suites all did this, so none could run from a checkout like `/My Disk/gpy`.
# The fix is to pass paths as data -- `fish -c 'source $argv[1]' -- "$path"`,
# `bash -c 'source "$1"' bash "$path"` -- and the code string stays a constant.
#
# Rule enforced over tests/ and scripts/ (.bash .zsh .fish .sh .md): a
# double-quoted argument to `-c` (or fish's `-C` init command) of
# fish/bash/zsh/sh/dash/ksh/$GPY_BASH/$SHELL, or `emulate MODE -c`, must contain
# no unescaped `$` expansion or backtick.
# `\$` is fine (the child shell expands it, not this one), and the lone form
# `-c "$code"` is fine (code already built as one word, nothing re-parsed).
# Interpreters that only share the flag spelling (`python3 -c`, `grep -c`,
# `od -c`) are not shells and are not matched.
#
# The scanner is proved live: a fixture tree with one bad site per shape must
# be reported file:line, and the compliant spellings must not be.
#
# Text-level on purpose; the fixture snippets below are assembled from pieces
# so that this file does not itself contain the pattern it forbids.

# shellcheck disable=SC2016 # the fixture snippets and messages are literal text

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
pass() { echo "PASS: $*"; }

if ! command -v python3 >/dev/null 2>&1; then
    # shellcheck source=tests/lib/shell_e2e.sh
    . "$ROOT/tests/lib/shell_e2e.sh"
    test_skip "python3 not installed"
fi

SCANNER="$(mktemp "${TMPDIR:-/tmp}/gpy-shell-c-scan.XXXXXX")"
FIXTURE="$(mktemp -d "${TMPDIR:-/tmp}/gpy-shell-c-fixture.XXXXXX")"
trap 'rm -rf "$SCANNER" "$FIXTURE"' EXIT

cat >"$SCANNER" <<'PY'
import os
import re
import sys

SUFFIXES = (".bash", ".zsh", ".fish", ".sh", ".md")
# A shell program, optional flags, then -c (alone or combined, e.g. -ec) and
# the opening double quote of the code string.
START = re.compile(
    r"""(?<![\w.-])
        (?:fish|bash|zsh|sh|dash|ksh|\$\{?GPY_BASH\}?|\$\{?SHELL\}?|emulate\s+\w+)
        (?:\s[^\n|;&()"']*?)?
        \s-[A-Za-z]*[cC]\s+"
    """,
    re.VERBOSE,
)
LONE_VARIABLE = re.compile(r"\$(?:\w+|\{\w+\})")
INTERPOLATION = re.compile(r"(?<!\\)(?:\$[A-Za-z_{(0-9@*#?!$-]|`)")


def code_string(text, start):
    """Body of the double-quoted string opening at `start` (just past the quote)."""
    out = []
    i = start
    while i < len(text):
        ch = text[i]
        if ch == "\\":
            out.append(text[i : i + 2])
            i += 2
            continue
        if ch == '"':
            break
        out.append(ch)
        i += 1
    return "".join(out)


files = 0
for root_dir in sys.argv[1:]:
    for dirpath, dirnames, filenames in os.walk(root_dir):
        dirnames[:] = [d for d in dirnames if d not in ("target", ".git", "node_modules")]
        for name in sorted(filenames):
            if not name.endswith(SUFFIXES):
                continue
            path = os.path.join(dirpath, name)
            with open(path, encoding="utf-8", errors="replace") as handle:
                text = handle.read()
            files += 1
            for match in START.finditer(text):
                body = code_string(text, match.end())
                if LONE_VARIABLE.fullmatch(body) or not INTERPOLATION.search(body):
                    continue
                line = text.count("\n", 0, match.end() - 1) + 1
                print(f"{path}:{line}: {' '.join(match.group(0).split())[-48:]}")
print(f"SCANNED {files}", file=sys.stderr)
PY

scan() { python3 "$SCANNER" "$@"; }

# --- the scanner reports every interpolating shape and spares the compliant ones --
Q='"' # a literal double quote, so the snippets below do not match the rule
{
    printf 'fish -c %ssource $root/x.fish%s\n' "$Q" "$Q"                           # line 1: fish
    printf 'bash -u -c %ssource %s$f%s%s\n' "$Q" "'" "'" "$Q"                      # line 2: bash, flag, inner quotes
    printf 'x=$(env A=1 zsh -f -c %s. ${root}/y%s)\n' "$Q" "$Q"                    # line 3: zsh, braces, in a $()
    printf 'emulate sh -c %s. $ROOT/lib.sh%s\n' "$Q" "$Q"                          # line 4: emulate
    printf 'fish --no-config -c %s\n  source $root/a.fish\n  echo \\$status\n%s\n' "$Q" "$Q" # line 5: multi-line
    printf 'sh -c %sid `whoami`%s\n' "$Q" "$Q"                                     # line 9: backtick
    printf 'fish --no-config -i -C %ssource $root/x.fish%s\n' "$Q" "$Q"            # line 10: fish -C init command
} >"$FIXTURE/bad.test.bash"
{
    printf 'fish -c %secho \\$status%s\n' "$Q" "$Q"                                # escaped for the child
    printf 'bash -e -c %s$commands%s\n' "$Q" "$Q"                                  # a single pre-built word
    printf 'fish -c %sset -l n 1%s -- "$path"\n' "$Q" "$Q"                         # constant code
    printf "bash -c 'source \"\$1\"' bash \"\$path\"\n"                            # single-quoted, path as data
    printf 'python3 -c %simport sys; print($n)%s\n' "$Q" "$Q"                      # not a shell
    printf 'grep -c %sPID=$pid,%s file\n' "$Q" "$Q"                                # not a shell
    printf 'od -c %s$file%s\n' "$Q" "$Q"                                           # not a shell
} >"$FIXTURE/good.test.bash"

found="$(scan "$FIXTURE" 2>/dev/null)"
expected_lines="1 2 3 4 5 9 10"
for line in $expected_lines; do
    if printf '%s\n' "$found" | grep -qF -- "bad.test.bash:$line:"; then
        pass "fixture: interpolating -c string at bad.test.bash:$line is reported"
    else
        fail "fixture: bad.test.bash:$line was NOT reported (the scanner is blind to its shape)"
    fi
done
reported="$(printf '%s\n' "$found" | grep -c 'bad.test.bash:' || true)"
if [ "$reported" -eq 7 ]; then
    pass "fixture: exactly the 7 bad sites are reported"
else
    fail "fixture: expected 7 reports for bad.test.bash, got $reported: $found"
fi
if printf '%s\n' "$found" | grep -qF 'good.test.bash'; then
    fail "fixture: a compliant spelling was flagged: $(printf '%s\n' "$found" | grep -F 'good.test.bash')"
else
    pass "fixture: escaped, lone-variable, constant, single-quoted and non-shell -c uses are spared"
fi

# --- the repository itself is clean --------------------------------------------
repo_findings="$(scan tests scripts 2>"$FIXTURE/scanned")"
scanned="$(sed -n 's/^SCANNED //p' "$FIXTURE/scanned")"
if [ "${scanned:-0}" -gt 50 ]; then
    pass "scanned $scanned files under tests/ and scripts/"
else
    fail "scanner saw only ${scanned:-0} files under tests/ and scripts/ -- the walk is broken"
fi
if [ -z "$repo_findings" ]; then
    pass "no shell -c code string in tests/ or scripts/ interpolates a variable"
else
    fail "paths must be passed as data (fish -c 'source \$argv[1]' -- \"\$path\"; bash -c 'source \"\$1\"' bash \"\$path\"), not interpolated into -c code:"
    printf '%s\n' "$repo_findings" | sed 's/^/    /'
fi

if [ "$failures" -gt 0 ]; then
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: no -c code string interpolates a variable"
