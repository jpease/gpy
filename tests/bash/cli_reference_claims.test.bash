#!/usr/bin/env bash
# tests/bash/cli_reference_claims.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# docs/user/cli-reference.md lists every shipped subcommand (#643).
#
# The reference used to drift from the binary: subcommands and flags shipped
# with no entry, and sample outputs described a different program. This walks
# the real `gpy --help` and `gpy <sub> --help` trees (so hidden commands such
# as `__complete` are never demanded, because clap does not print them) and
# requires each non-hidden subcommand path to be named in the reference as
# a `gpy <sub> [<sub>]` code span or heading. It does the same for the
# `gpy-agent` binary's top-level subcommands, which the shells tell users to
# run. It also pins the documented user-facing options that were missing
# before (#643) and the environment variables the binary reads.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

DOC="$ROOT/docs/user/cli-reference.md"
GPY="$ROOT/gpy-agent/target/debug/gpy"
GPY_AGENT="$ROOT/gpy-agent/target/debug/gpy-agent"
if [ ! -x "$GPY" ] || [ ! -x "$GPY_AGENT" ]; then
    (cd "$ROOT/gpy-agent" && cargo build --quiet --bin gpy --bin gpy-agent) || {
        echo "FAIL: could not build the gpy binaries"
        exit 1
    }
fi

failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
pass() { echo "PASS: $*"; }

# The subcommand names clap prints under "Commands:" for `BIN ARGS... --help`.
subcommands_of() {
    "$@" --help 2>/dev/null | awk '
        /^Commands:/ { in_cmds = 1; next }
        in_cmds && /^[A-Za-z]/ { exit }
        in_cmds && /^  [a-z][a-z0-9_-]*/ { print $1 }
    ' | grep -v '^help$'
}

# `documented BIN SUB...` -- the doc names the command path somewhere.
documented() {
    local path="$*"
    grep -qE -- "\`$path( |\`|\\[|<)" "$DOC"
}

# `walk BIN LABEL SUB...` -- every non-hidden subcommand under the path is documented.
walk() {
    local bin="$1" label="$2"
    shift 2
    local sub
    for sub in $(subcommands_of "$bin" "$@"); do
        if documented "$label" "$@" "$sub"; then
            pass "$label $* $sub is documented"
        else
            fail "$label $* $sub is shipped but docs/user/cli-reference.md never names it"
        fi
        walk "$bin" "$label" "$@" "$sub"
    done
}

echo "--- every gpy subcommand path is in the reference ---"
walk "$GPY" gpy

echo "--- every gpy-agent top-level subcommand is in the reference ---"
for sub in $(subcommands_of "$GPY_AGENT"); do
    if documented gpy-agent "$sub"; then
        pass "gpy-agent $sub is documented"
    else
        fail "gpy-agent $sub is shipped but the reference never names it"
    fi
done

# The reverse direction (#783): a code span naming `BIN <word>` must name a
# subcommand the binary ships. `help` is clap's own and always exists.
# shellcheck disable=SC2016  # the backtick is a markdown code-span delimiter, not an expansion
documented_words() {
    grep -oE -- "\`$1 [a-z][a-z0-9_-]*" "$DOC" | awk '{print $2}' | sort -u
}
check_names_exist() {
    local bin="$1" label="$2" word shipped
    shipped=" $(subcommands_of "$bin" | tr '\n' ' ') help "
    for word in $(documented_words "$label"); do
        case "$shipped" in
            *" $word "*) pass "$label $word exists" ;;
            *) fail "docs name $label $word but the binary ships no such subcommand" ;;
        esac
    done
}
echo "--- every documented gpy / gpy-agent subcommand exists ---"
check_names_exist "$GPY" gpy
check_names_exist "$GPY_AGENT" gpy-agent

echo "--- the options and variables #643 found missing stay documented ---"
# shellcheck disable=SC2016  # the backticks are markdown code spans to find, not expansions
for needle in 'gpy theme use' '`--force`' 'gpy theme import' 'gpy debug paths' 'gpy debug prompt' \
    'GPY_CONFIG_PATH' 'XDG_CONFIG_HOME' 'XDG_CACHE_HOME' 'GPY_NERD_FONT' '`VISUAL`' 'GPY_DEBUG_LOG'; do
    if grep -qF -- "$needle" "$DOC"; then
        pass "reference mentions $needle"
    else
        fail "reference lost $needle"
    fi
done

echo "--- sample outputs match the binary ---"
# The status report's field names are the binary's, not an older design.
for field in "Agent Version:" "Protocol Version:" "Watched Repos:" "Registered Clients:" "Cache Entries:"; do
    if grep -qF -- "$field" "$DOC"; then
        pass "status sample has '$field'"
    else
        fail "status sample lacks '$field'"
    fi
done
for stale in "Requests Handled" "Agent Status: Query" "✅ Agent reloaded configuration" "Fish shell prompt. It provides"; do
    if grep -qF -- "$stale" "$DOC"; then
        fail "reference still carries the stale text '$stale'"
    else
        pass "no stale '$stale'"
    fi
done
# The single reload notice every mutating command prints.
if grep -qF -- "Agent not reloaded; the new config applies the next time it starts." "$DOC"; then
    pass "the reload notice is documented verbatim"
else
    fail "the reload notice text is not in the reference"
fi

if [ "$failures" -gt 0 ]; then
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: cli-reference.md lists every shipped subcommand and matches the binary"
