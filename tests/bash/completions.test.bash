#!/usr/bin/env bash
# tests/bash/completions.test.bash
#
# Task 4/6 of Epic #326: BASH completions for the `gpy` CLI.
#
# `gpy completions bash` (clap_complete) generates the STRUCTURAL surface
# (subcommand/flag names) into a function named `_gpy`, registered via
# `complete -F _gpy ... gpy`. It has no way to know the live set of themes,
# palettes, or segments installed on this machine -- those come from the
# hidden `gpy __complete <kind>` command (#328).
#
# bash/completions/gpy-dynamic.bash is the checked-in, hand-authored glue
# that supplies those dynamic value lists. It must:
#   1. Fill in the six dynamic value positions (theme/palette use+validate,
#      enable/disable segment) from `gpy __complete <kind>`.
#   2. Delegate to the structural `_gpy` function for every other position
#      (subcommand/flag completion), since only one `complete -F` can win
#      per command in bash.
#
# This test sources the *generated* structural completion (built fresh from
# the debug `gpy` binary, mirroring the "parity test needs binary rebuild"
# gotcha -- `cargo test` does not rebuild bin targets) followed by the
# checked-in dynamic glue, then drives `_gpy_dynamic` directly the same way
# bash's programmable completion would: $1=command, $2=cur, $3=prev, with
# COMP_WORDS/COMP_CWORD set as globals.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

GPY_BIN="$ROOT/gpy-agent/target/debug/gpy"

if [[ ! -x "$GPY_BIN" ]]; then
    echo "Building debug gpy CLI binary..."
    (cd "$ROOT/gpy-agent" && cargo build --bin gpy) || {
        echo "FAIL: could not build gpy CLI binary"
        exit 1
    }
fi

export PATH="$ROOT/gpy-agent/target/debug:$PATH"

GENERATED_COMPLETIONS="$(mktemp -t gpy-completions-bash.XXXXXX)"
trap 'rm -f "$GENERATED_COMPLETIONS"' EXIT

if ! gpy completions bash >"$GENERATED_COMPLETIONS"; then
    echo "FAIL: gpy completions bash failed to generate structural completions"
    exit 1
fi

echo "Sourcing generated structural completions..."
# shellcheck source=/dev/null
source "$GENERATED_COMPLETIONS"

if ! declare -F _gpy &>/dev/null; then
    echo "FAIL: structural _gpy function not defined after sourcing generated completions"
    exit 1
fi

echo "Sourcing checked-in dynamic completions glue..."
source "$ROOT/bash/completions/gpy-dynamic.bash"

if ! declare -F _gpy_dynamic &>/dev/null; then
    echo "FAIL: _gpy_dynamic function not defined"
    exit 1
fi

# contains NEEDLE ARRAY_ELEMENTS...
contains() {
    local needle="$1"
    shift
    local item
    for item in "$@"; do
        if [[ "$item" == "$needle" ]]; then
            return 0
        fi
    done
    return 1
}

failures=0

check_dynamic() {
    local label="$1"
    shift
    local expected="$1"
    shift
    # Remaining args: COMP_WORDS to set, then invocation args after "--".
    local -a words=()
    while [[ "$1" != "--" ]]; do
        words+=("$1")
        shift
    done
    shift # drop the --
    local cword="$1"
    shift

    COMP_WORDS=("${words[@]}")
    COMP_CWORD="$cword"
    COMPREPLY=()
    _gpy_dynamic "$@"

    if contains "$expected" "${COMPREPLY[@]}"; then
        echo "PASS: $label completes '$expected'"
    else
        echo "FAIL: $label did not complete '$expected'; got: ${COMPREPLY[*]}"
        failures=$((failures + 1))
    fi
}

echo "Checking theme use..."
check_dynamic "gpy theme use" "default" gpy theme use "" -- 3 gpy "" use
check_dynamic "gpy theme use" "starship" gpy theme use "" -- 3 gpy "" use
check_dynamic "gpy theme use" "text" gpy theme use "" -- 3 gpy "" use

echo "Checking theme validate..."
check_dynamic "gpy theme validate" "default" gpy theme validate "" -- 3 gpy "" validate

echo "Checking palette use..."
check_dynamic "gpy palette use" "default" gpy palette use "" -- 3 gpy "" use
check_dynamic "gpy palette use" "nord" gpy palette use "" -- 3 gpy "" use

echo "Checking palette validate..."
check_dynamic "gpy palette validate" "nord" gpy palette validate "" -- 3 gpy "" validate

echo "Checking enable..."
check_dynamic "gpy enable" "git" gpy enable "" -- 2 gpy "" enable
check_dynamic "gpy enable" "status" gpy enable "" -- 2 gpy "" enable

echo "Checking disable..."
check_dynamic "gpy disable" "git" gpy disable "" -- 2 gpy "" disable
check_dynamic "gpy disable" "status" gpy disable "" -- 2 gpy "" disable

echo "Checking top-level delegation to structural completions..."
COMP_WORDS=(gpy "")
COMP_CWORD=1
COMPREPLY=()
_gpy_dynamic gpy "" gpy

if contains "theme" "${COMPREPLY[@]}" && contains "enable" "${COMPREPLY[@]}"; then
    echo "PASS: top-level completion delegates to structural _gpy (theme, enable present)"
else
    echo "FAIL: top-level completion did not delegate to structural _gpy; got: ${COMPREPLY[*]}"
    failures=$((failures + 1))
fi

echo "Checking delegation degrades gracefully when _gpy is undefined..."
(
    unset -f _gpy
    COMP_WORDS=(gpy "")
    COMP_CWORD=1
    COMPREPLY=()
    _gpy_dynamic gpy "" gpy
    if [[ ${#COMPREPLY[@]} -eq 0 ]]; then
        echo "PASS: missing structural _gpy degrades to empty COMPREPLY"
    else
        echo "FAIL: expected empty COMPREPLY when _gpy is undefined; got: ${COMPREPLY[*]}"
        exit 1
    fi
) || failures=$((failures + 1))

# ---------------------------------------------------------------------------
# Per-session caching (#346): __gpy_complete_cached must fork `gpy` at most
# once per kind per session, and must never cache an empty/failed result.
# A `gpy` shell function shadows the real binary for the rest of this
# subshell (same stub approach as the checks above). The call counter can't
# be a plain shell variable incremented from inside the stub: production code
# invokes `gpy` via `$(gpy ...)` inside `__gpy_complete_cached`, and command
# substitution always forks a subshell, so a variable increment made there
# would vanish when that subshell exits. Instead the stub appends a byte to
# a temp file on every call; the file's size (not a variable) is the call
# count, since file writes are a real OS-level side effect that survives the
# subshell. Runs in `(...)` subshells so the stub/counter/cache-var resets
# never leak into the checks above or below.
# ---------------------------------------------------------------------------
gpy_calls_file="$(mktemp -t gpy-cache-test-calls.XXXXXX)"
trap 'rm -f "$gpy_calls_file"' EXIT

gpy_call_count() {
    wc -l <"$gpy_calls_file" | tr -d ' '
}

echo "Checking warm cache does not re-fork gpy..."
(
    unset __gpy_complete_cache_theme __gpy_complete_cache_segment
    : >"$gpy_calls_file"
    gpy() {
        echo call >>"$gpy_calls_file"
        if [[ "$1" == "__complete" ]]; then
            case "$2" in
                theme) printf 'default\nstarship\ntext\n' ;;
                segment) printf 'clock\ngit\n' ;;
            esac
        fi
    }

    COMP_WORDS=(gpy theme use "")
    COMP_CWORD=3
    COMPREPLY=()
    _gpy_dynamic gpy "" use
    first_reply=("${COMPREPLY[@]}")

    COMPREPLY=()
    _gpy_dynamic gpy "" use
    second_reply=("${COMPREPLY[@]}")

    count="$(gpy_call_count)"
    if [[ "$count" -ne 1 ]]; then
        echo "FAIL: expected exactly 1 gpy invocation after a warm second lookup, got $count"
        exit 1
    fi
    if ! contains "default" "${first_reply[@]}" || ! contains "default" "${second_reply[@]}"; then
        echo "FAIL: cached candidates missing 'default'; first=${first_reply[*]} second=${second_reply[*]}"
        exit 1
    fi
    echo "PASS: second warm completion reuses the cache instead of re-forking gpy"
) || failures=$((failures + 1))

echo "Checking a different kind's cache miss does not disturb an already-warm kind..."
(
    unset __gpy_complete_cache_theme __gpy_complete_cache_segment
    : >"$gpy_calls_file"
    gpy() {
        echo call >>"$gpy_calls_file"
        if [[ "$1" == "__complete" ]]; then
            case "$2" in
                theme) printf 'default\n' ;;
                segment) printf 'clock\ngit\n' ;;
            esac
        fi
    }

    COMP_WORDS=(gpy theme use "")
    COMP_CWORD=3
    COMPREPLY=()
    _gpy_dynamic gpy "" use # warms theme cache (1st fork)

    COMP_WORDS=(gpy enable "")
    COMP_CWORD=2
    COMPREPLY=()
    _gpy_dynamic gpy "" enable # segment cache miss (2nd fork)
    segment_reply=("${COMPREPLY[@]}")

    COMP_WORDS=(gpy theme use "")
    COMP_CWORD=3
    COMPREPLY=()
    _gpy_dynamic gpy "" use # must stay a theme cache hit -- no 3rd fork

    count="$(gpy_call_count)"
    if [[ "$count" -ne 2 ]]; then
        echo "FAIL: expected exactly 2 gpy invocations (1 per kind), got $count"
        exit 1
    fi
    if ! contains "git" "${segment_reply[@]}"; then
        echo "FAIL: segment candidates missing 'git'; got: ${segment_reply[*]}"
        exit 1
    fi
    echo "PASS: independent per-kind caches don't force extra forks"
) || failures=$((failures + 1))

echo "Checking an empty/failed gpy __complete is never cached..."
(
    unset __gpy_complete_cache_theme
    : >"$gpy_calls_file"
    gpy() {
        echo call >>"$gpy_calls_file"
        return 1
    }

    COMP_WORDS=(gpy theme use "")
    COMP_CWORD=3
    COMPREPLY=()
    _gpy_dynamic gpy "" use
    first_reply=("${COMPREPLY[@]}")

    COMPREPLY=()
    _gpy_dynamic gpy "" use
    second_reply=("${COMPREPLY[@]}")

    count="$(gpy_call_count)"
    if [[ "$count" -ne 2 ]]; then
        echo "FAIL: expected 2 gpy invocations (failure must never be cached), got $count"
        exit 1
    fi
    if [[ ${#first_reply[@]} -ne 0 || ${#second_reply[@]} -ne 0 ]]; then
        echo "FAIL: expected empty COMPREPLY on repeated failure; first=${first_reply[*]} second=${second_reply[*]}"
        exit 1
    fi
    echo "PASS: failed/empty gpy __complete retries instead of caching the failure"
) || failures=$((failures + 1))

if [[ "$failures" -ne 0 ]]; then
    echo "FAIL: $failures completion check(s) failed"
    exit 1
fi

echo "PASS"
