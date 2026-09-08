#!/usr/bin/env zsh

# tests/zsh/completions.test.zsh
#
# Task 5/6 of Epic #326: ZSH completions for the `gpy` CLI.
#
# `gpy completions zsh` (clap_complete) generates the STRUCTURAL surface
# (subcommand/flag names) into a function named `_gpy`. It has no way to
# know the live set of themes, palettes, or segments installed on this
# machine -- value positions for those args fall back to `_default` (plain
# file completion). The hidden `gpy __complete <kind>` command supplies the
# live names (#328).
#
# zsh/completions/_gpy-dynamic is the checked-in, hand-authored glue that
# fills in the six dynamic value positions (theme/palette use+validate,
# enable/disable segment) and delegates to the generated `_gpy` for every
# other position, since zsh (like bash) allows only ONE function to be
# registered as a command's completer at a time (unlike fish, which merges
# `complete -c` rules from every sourced file).
#
# Testing zsh completion functions for real is fiddly: `_describe`,
# `compadd`, and friends refuse to run outside an actual completion-widget
# invocation ("can only be called from completion function"), which only
# exists when zsh's line editor (zle) is driving a real completion --
# there's no way to fake that context with plain function calls. This test
# drives REAL candidate capture using the `zsh/zpty` module: it spawns a
# nested interactive zsh in a pty, wires up the completions exactly as
# `zsh/gpy.zsh` does, types a partial command, and reads back the rendered
# completion listing zle prints below the prompt. `zsh/zpty` is what zsh's
# own test suite (Test/*.ztst) uses for exactly this reason.

ROOT=${0:a:h:h:h}
cd "$ROOT"

GPY_BIN="$ROOT/gpy-agent/target/debug/gpy"

if [[ ! -x "$GPY_BIN" ]]; then
    echo "Building debug gpy CLI binary..."
    (cd "$ROOT/gpy-agent" && cargo build --bin gpy) || {
        echo "FAIL: could not build gpy CLI binary"
        exit 1
    }
fi

export PATH="$ROOT/gpy-agent/target/debug:$PATH"

echo "Checking files parse cleanly under zsh..."
if ! zsh -n "$ROOT/zsh/completions/_gpy-dynamic"; then
    echo "FAIL: zsh/completions/_gpy-dynamic does not parse"
    exit 1
fi
if ! zsh -n "$ROOT/zsh/gpy.zsh"; then
    echo "FAIL: zsh/gpy.zsh does not parse"
    exit 1
fi

echo "Generating structural completions from the debug gpy binary..."
GENERATED_DIR=$(mktemp -d -t gpy-completions-zsh.XXXXXX)
if ! gpy completions zsh >"$GENERATED_DIR/_gpy"; then
    echo "FAIL: gpy completions zsh failed to generate structural completions"
    rm -rf "$GENERATED_DIR"
    exit 1
fi

failures=0

zmodload zsh/zpty

# contains NEEDLE HAYSTACK
contains() {
    [[ "$2" == *"$1"* ]]
}

# Drain whatever the pty has produced so far, polling briefly rather than
# sleeping a fixed amount -- the pty is fast (local process, no network) but
# completion rendering + zle redraw is not perfectly synchronous.
drain() {
    local out="" line
    local i=0
    while (( i < 60 )); do
        if zpty -r -t COMPPTY line 2>/dev/null; then
            out+="$line"
        else
            sleep 0.05
        fi
        i=$((i + 1))
    done
    print -r -- "$out"
}

send() {
    zpty -w COMPPTY "$1"$'\n'
}

cleanup() {
    zpty -d COMPPTY 2>/dev/null
    rm -rf "$GENERATED_DIR"
    rm -f "$CALLS_FILE" "$THEME_FILE" "$SEGMENT_FILE"
}
trap cleanup EXIT

echo "Spawning nested zsh in a pty and wiring completions..."
zpty COMPPTY "zsh -f"
export COLUMNS=200 LINES=50 TERM=xterm
drain >/dev/null
send "PROMPT='TESTPROMPT> '"
drain >/dev/null
send "autoload -Uz compinit"
drain >/dev/null
send "compinit -u -d $(mktemp -u -t gpy-zcompdump.XXXXXX)"
drain >/dev/null
# Mirror zsh/gpy.zsh's wiring exactly: add the repo's checked-in completions
# dir plus the freshly generated structural completions dir to fpath,
# autoload both, and register the dynamic wrapper as gpy's completer.
send "fpath=($ROOT/zsh/completions $GENERATED_DIR \$fpath)"
drain >/dev/null
send "autoload -Uz _gpy-dynamic; autoload -Uz _gpy; compdef _gpy-dynamic gpy"
out=$(drain)
if [[ "$out" == *"not found"* || "$out" == *"error"* ]]; then
    echo "FAIL: setting up completions in the pty produced an error: $out"
    failures=$((failures + 1))
fi

echo "Verifying _gpy-dynamic is registered as gpy's completer..."
send 'print -r -- ${_comps[gpy]}'
out=$(drain)
if contains "_gpy-dynamic" "$out"; then
    echo "PASS: gpy's completer is _gpy-dynamic"
else
    echo "FAIL: gpy's completer is not _gpy-dynamic; pty said: $out"
    failures=$((failures + 1))
fi

check_candidates() {
    local label="$1" cmdline="$2"
    shift 2
    local expected

    zpty -w -n COMPPTY "$cmdline"
    zpty -w -n COMPPTY $'\t'
    local out=$(drain)
    # Clear the typed line back out so the next command starts clean.
    send ""
    drain >/dev/null

    for expected in "$@"; do
        if contains "$expected" "$out"; then
            echo "PASS: $label completes '$expected'"
        else
            echo "FAIL: $label did not complete '$expected'; pty said: $out"
            failures=$((failures + 1))
        fi
    done
}

echo "Checking gpy theme use <TAB>..."
check_candidates "gpy theme use" "gpy theme use " default starship text

echo "Checking gpy palette use <TAB>..."
check_candidates "gpy palette use" "gpy palette use " default nord

echo "Checking gpy enable <TAB>..."
check_candidates "gpy enable" "gpy enable " git status

echo "Checking gpy disable <TAB>..."
check_candidates "gpy disable" "gpy disable " git status

echo "Checking top-level completion delegates to structural _gpy..."
check_candidates "gpy (top-level)" "gpy " theme enable palette

# ---------------------------------------------------------------------------
# Per-session caching (#346): __gpy_complete_cached must fork `gpy` at most
# once per kind per session, and must never cache an empty/failed result.
#
# The checks above already drove `_gpy-dynamic` through a real TAB press, so
# `__gpy_complete_cached` (a nested function -- see the comment in
# zsh/completions/_gpy-dynamic on why it can't be a second top-level function
# in that file) is now defined globally in the pty's nested zsh and can be
# called directly, without going through `_describe`/`compadd`, which refuse
# to run outside a real completion-widget invocation.
#
# The stub `gpy` function's call counter can't be a plain shell variable
# incremented from inside the stub: production code invokes `gpy` via
# `$(gpy ...)` inside `__gpy_complete_cached`, and command substitution
# always forks a subshell, so a variable increment made there would vanish
# once that subshell exits. Instead the stub appends a line to a file on
# every call, and candidate lists are `cat`'d from pre-written files rather
# than embedding `printf '...\n...'` escapes in text typed through the pty --
# both counter and candidates are real filesystem state, which (unlike a
# shell variable) survives the subshell.
# ---------------------------------------------------------------------------
echo "Checking per-session caching (#346): gpy __complete forks at most once per kind..."

CALLS_FILE=$(mktemp -t gpy-zsh-cache-calls.XXXXXX)
THEME_FILE=$(mktemp -t gpy-zsh-cache-theme.XXXXXX)
SEGMENT_FILE=$(mktemp -t gpy-zsh-cache-segment.XXXXXX)
printf 'default\nstarship\ntext\n' >"$THEME_FILE"
printf 'clock\ngit\n' >"$SEGMENT_FILE"

call_count() {
    local n
    n=$(wc -l <"$CALLS_FILE")
    print -r -- "${n// /}"
}

send "unset __gpy_complete_cache_theme __gpy_complete_cache_segment"
drain >/dev/null
send "gpy() { echo call >> $CALLS_FILE; case \$2 in theme) cat $THEME_FILE ;; segment) cat $SEGMENT_FILE ;; esac }"
drain >/dev/null

: >"$CALLS_FILE"
send "__gpy_complete_cached theme; print -r -- CACHE1:\$__gpy_complete_result"
out1=$(drain)
send "__gpy_complete_cached theme; print -r -- CACHE2:\$__gpy_complete_result"
out2=$(drain)

if [[ "$(call_count)" == "1" ]]; then
    echo "PASS: second warm theme lookup reuses the cache instead of re-forking gpy"
else
    echo "FAIL: expected exactly 1 gpy invocation for two theme lookups, got $(call_count)"
    failures=$((failures + 1))
fi
if contains "default" "$out1" && contains "default" "$out2"; then
    echo "PASS: cached candidates correct across both calls"
else
    echo "FAIL: cached candidates incorrect; out1=$out1 out2=$out2"
    failures=$((failures + 1))
fi

echo "Checking a different kind's cache miss does not disturb an already-warm kind..."
: >"$CALLS_FILE"
send "__gpy_complete_cached segment; print -r -- SEG:\$__gpy_complete_result"
seg_out=$(drain)
send "__gpy_complete_cached theme; print -r -- THEME_AGAIN:\$__gpy_complete_result"
theme_out=$(drain)

if [[ "$(call_count)" == "1" ]]; then
    echo "PASS: segment miss forks once, warm theme cache adds no extra fork"
else
    echo "FAIL: expected exactly 1 gpy invocation (segment miss only), got $(call_count)"
    failures=$((failures + 1))
fi
if contains "git" "$seg_out" && contains "default" "$theme_out"; then
    echo "PASS: independent per-kind caches return correct candidates"
else
    echo "FAIL: per-kind candidates incorrect; seg_out=$seg_out theme_out=$theme_out"
    failures=$((failures + 1))
fi

echo "Checking an empty/failed gpy __complete is never cached..."
: >"$CALLS_FILE"
send "unset __gpy_complete_cache_theme; gpy() { echo call >> $CALLS_FILE; return 1 }"
drain >/dev/null
send "__gpy_complete_cached theme; print -r -- FAIL1:\$__gpy_complete_result:"
fail_out1=$(drain)
send "__gpy_complete_cached theme; print -r -- FAIL2:\$__gpy_complete_result:"
fail_out2=$(drain)

if [[ "$(call_count)" == "2" ]]; then
    echo "PASS: failed/empty gpy __complete retries instead of caching the failure"
else
    echo "FAIL: expected 2 gpy invocations (failure must never be cached), got $(call_count)"
    failures=$((failures + 1))
fi
if contains "FAIL1::" "$fail_out1" && contains "FAIL2::" "$fail_out2"; then
    echo "PASS: failed lookups yield no candidates"
else
    echo "FAIL: expected empty result on repeated failure; fail_out1=$fail_out1 fail_out2=$fail_out2"
    failures=$((failures + 1))
fi

if [[ "$failures" -ne 0 ]]; then
    echo "FAIL: $failures completion check(s) failed"
    exit 1
fi

echo "PASS"
