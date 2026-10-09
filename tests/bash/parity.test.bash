#!/usr/bin/env bash

# tests/bash/parity.test.bash
# Verify Bash implementation matches Fish/Zsh feature set

# shellcheck disable=SC2154
# __gpy_duration_method is assigned by bash/gpy.bash, sourced above.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

# Hermetic when run directly (#820): the developer's theme export
# ($XDG_CACHE_HOME/gpy/theme-export.bash) and exported GPY_* settings (e.g.
# GPY_LANGUAGE_ENABLED=0) would otherwise override the built-in defaults
# these checks assert on. quality-check.sh sets its own XDG dirs; this makes
# a direct `bash tests/bash/parity.test.bash` behave the same.
for gpy_var in $(compgen -e); do
    [[ "$gpy_var" == GPY_* ]] && unset "$gpy_var"
done
unset gpy_var
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
parity_xdg_root="$(mktemp -d "${TMPDIR:-/tmp}/gpy-parity-xdg.XXXXXX")"
# Sourcing the entry point starts a supervised gpy-agent (#750): stop it before
# its socket directory is removed.
trap 'shell_e2e_stop_agent_under "$parity_xdg_root/cache" "$parity_xdg_root/config"; rm -rf "$parity_xdg_root"' EXIT
mkdir -p "$parity_xdg_root/cache" "$parity_xdg_root/config"
export XDG_CACHE_HOME="$parity_xdg_root/cache" XDG_CONFIG_HOME="$parity_xdg_root/config"
unset XDG_RUNTIME_DIR

echo "=== Checking Feature Parity with Fish/Zsh ==="

# List of expected segments
required_segments=(
    directory
    git
    clock
    duration
    language
    status
)

source bash/gpy.bash

for seg in "${required_segments[@]}"; do
    if ! declare -f "__gpy_segment_$seg" &>/dev/null; then
        echo "FAIL: Missing segment: $seg"
        exit 1
    fi
    echo "✓ $seg"
done

# Check signal handling. Bash must NOT trap SIGURG (#678): the doorbell's
# flags are consumed at the next prompt, and a trap only re-rendered a PS1
# readline cannot show, nested renders on bash 5 and aborted `wait`. Read
# the trap via command substitution, NOT a pipe: `trap -p | grep` runs
# `trap -p` on the left of a pipe (a subshell), and bash 3.2 (macOS's system
# bash) resets non-ignored traps in subshells. `$(...)` preserves them on
# both bash 3.2 and 5.
if [[ -n "$(trap -p URG)" ]]; then
    echo "FAIL: SIGURG is trapped: $(trap -p URG)"
    exit 1
fi
if ! declare -F __gpy_consume_shell_flags >/dev/null; then
    echo "FAIL: Missing doorbell flag consumer __gpy_consume_shell_flags"
    exit 1
fi
echo "✓ SIGURG doorbell flags consumed at the prompt, no URG trap"

# Check supervisor
if ! declare -f __gpy_supervisor_start &>/dev/null; then
    echo "FAIL: Missing supervisor"
    exit 1
fi
echo "✓ Supervisor"

# Check duration tracking capability
echo "✓ Duration method: $__gpy_duration_method (Bash ${BASH_VERSINFO[0]}.${BASH_VERSINFO[1]})"

# Exit cleanup: bash/core/signals.bash sets an EXIT trap that sends
# `unregister`, chained behind any existing EXIT trap. Asserted end to end in
# tests/bash/e2e_agent_autostart.test.bash.
echo "✓ Exit cleanup: EXIT trap unregisters the client"

# --- Cache key parity ---
# The hand-copied "reference values computed from the Rust implementation"
# that used to live here could drift from the encoder unnoticed. The key
# encoding is now pinned against a file the agent itself writes, in
# tests/bash/path_parity.test.bash (#647).

# --- Warm cache hit: no IPC roundtrip ---
echo "=== Warm Cache Hit Tests ==="

cache_dir="${XDG_CACHE_HOME:-$HOME/.cache}/gpy/instant-prompts"
mkdir -p "$cache_dir"

# Set up a temp git repo so __gpy_find_git_root succeeds
tmpdir=$(mktemp -d)
git -C "$tmpdir" init -q
git -C "$tmpdir" commit --allow-empty -m init -q \
    --author "Test <test@example.com>" 2>/dev/null || true

# Compute the cache key for the temp repo root (realpath to match the function)
tmpdir_real=$(realpath "$tmpdir" 2>/dev/null || echo "$tmpdir")
# Cache files are keyed by prev_bg token; a read with no prev_bg resolves "none".
cache_key="$(__gpy_path_to_cache_key "$tmpdir_real")"
cache_file="$cache_dir/$cache_key.git.none.bash"

# Write a sentinel value into the cache file
sentinel="cached-git-output-sentinel"
echo "$sentinel" > "$cache_file"

# Read via __gpy_read_instant_cache — should return sentinel without IPC
result="$(__gpy_read_instant_cache "git" "$tmpdir")"
if [[ "$result" != "$sentinel" ]]; then
    echo "FAIL: warm cache read returned wrong value"
    echo "  expected: $sentinel"
    echo "  actual:   $result"
    rm -rf "$tmpdir"
    rm -f "$cache_file"
    exit 1
fi
echo "✓ warm cache hit returns cached content"

# --- Serve-stale: old cache is served, not deleted ---
echo "=== Serve-Stale Tests ==="

# Back-date the cache file to make it stale (>5s old)
touch -t "$(date -v-10S "+%Y%m%d%H%M.%S" 2>/dev/null || date -d "10 seconds ago" "+%Y%m%d%H%M.%S" 2>/dev/null)" \
    "$cache_file" 2>/dev/null \
    || python3 -c "import os, time; os.utime('$cache_file', (time.time()-10, time.time()-10))" 2>/dev/null \
    || true

stale_result="$(__gpy_read_instant_cache "git" "$tmpdir")"
stale_status=$?
if [[ "$stale_result" != "$sentinel" ]]; then
    echo "FAIL: stale cache was not served (expected sentinel, got '$stale_result')"
    rm -rf "$tmpdir"
    rm -f "$cache_file"
    exit 1
fi
echo "✓ stale cache is served (not deleted)"

if [[ ! -f "$cache_file" ]]; then
    echo "FAIL: stale cache file was deleted (should be preserved for serve-stale)"
    rm -rf "$tmpdir"
    exit 1
fi
echo "✓ stale cache file preserved after serve"

# Staleness is now carried entirely by the exit code (#614: bit 2 set --
# status 2 or 6 -- signals staleness); no more `__gpy_git_stale_$$` temp file.
if (( (stale_status & 2) != 0 )); then
    echo "✓ stale status bit set to signal background refresh needed"
else
    echo "FAIL: stale status bit not set (got $stale_status) — background refresh cannot be triggered"
    rm -rf "$tmpdir"
    rm -f "$cache_file"
    exit 1
fi

# Cleanup
rm -rf "$tmpdir"
rm -f "$cache_file"

# --- Language segment detection walks ancestors (#174) ---
echo "=== Language Subdir Detection Tests ==="

source bash/segments/language.bash
# The pre-filter iterates the agent-exported marker list (#785); this block
# tests the ancestor walk, so a one-entry list suffices. The full export is
# covered by language_marker_prefilter.test.bash.
__gpy_lang_marker_files=(Cargo.toml)

detect_proj=$(mktemp -d)
touch "$detect_proj/Cargo.toml"
mkdir -p "$detect_proj/src/nested/deep"

assert_detect() {
    local label="$1" expected="$2" dir="$3"
    (cd "$dir" && __gpy_segment_language_detect)
    local got=$?
    if [[ "$got" -eq "$expected" ]]; then
        echo "✓ $label"
    else
        echo "FAIL: $label (expected $expected, got $got)"
        rm -rf "$detect_proj"
        exit 1
    fi
}

assert_detect "detects at project root" 0 "$detect_proj"
assert_detect "detects in src/ subdir" 0 "$detect_proj/src"
assert_detect "detects in deeply nested subdir" 0 "$detect_proj/src/nested/deep"

detect_empty=$(mktemp -d)
assert_detect "not detected in plain dir" 1 "$detect_empty"

rm -rf "$detect_proj" "$detect_empty"

# --- Non-Git language cache hit (#173) ---
echo "=== Non-Git Language Cache Tests ==="

nongit=$(mktemp -d)
touch "$nongit/package.json"
nongit_real=$(realpath "$nongit" 2>/dev/null || echo "$nongit")
nongit_key="$(__gpy_path_to_cache_key "$nongit_real")"

lang_file="$cache_dir/$nongit_key.lang.none.bash"
lang_sentinel="cached-lang-output-sentinel"
echo "$lang_sentinel" > "$lang_file"

lang_result="$(__gpy_read_instant_cache "lang" "$nongit")"
if [[ "$lang_result" != "$lang_sentinel" ]]; then
    echo "FAIL: non-git lang cache not served (expected '$lang_sentinel', got '$lang_result')"
    rm -rf "$nongit"; rm -f "$lang_file"
    exit 1
fi
echo "✓ language cache served in non-git project"

# Git cache must NOT use the path fallback (requires a real Git root).
git_nongit_file="$cache_dir/$nongit_key.git.none.bash"
echo "should-not-be-served" > "$git_nongit_file"
git_nongit_result="$(__gpy_read_instant_cache "git" "$nongit")"
if [[ -n "$git_nongit_result" ]]; then
    echo "FAIL: git cache wrongly served in non-git project (got '$git_nongit_result')"
    rm -rf "$nongit"; rm -f "$lang_file" "$git_nongit_file"
    exit 1
fi
echo "✓ git cache correctly misses in non-git project"

rm -rf "$nongit"
rm -f "$lang_file" "$git_nongit_file"

# --- Two-line layout (opt-in, Starship-style) ---
echo "=== Two-Line Layout Tests ==="

__enabled_segments=""
__prompt_color=green
__icon_prompt="❯"

# Pin the blank-line separator off for this block. It defaults on (matching
# fish), and these cases assert on the presence of *any* newline in the
# prompt, so leaving it enabled would make the two_line=0 case fail on a
# newline that has nothing to do with two-line layout.
__gpy_add_newline=0

__gpy_two_line=0
__gpy_render_prompt 0
single_prompt="$PS1"
case "$single_prompt" in
    *$'\n'*)
        echo "FAIL: single-line prompt (two_line=0) unexpectedly contains a newline"
        exit 1
        ;;
esac
echo "✓ single-line prompt has no newline (no regression)"

__gpy_two_line=1
__gpy_render_prompt 0
two_line_prompt="$PS1"
case "$two_line_prompt" in
    *$'\n'*)
        echo "✓ two-line prompt inserts newline before character"
        ;;
    *)
        echo "FAIL: two-line prompt (two_line=1) missing newline before character"
        exit 1
        ;;
esac

# --- prev_bg powerline parity (#220, #222) ---
echo "=== prev_bg Powerline Parity Tests ==="

# Token derivation must match the Rust prev_bg_token contract.
[[ "$(__gpy_prev_bg_token "")" == "none" ]] || { echo "FAIL: empty prev_bg token"; exit 1; }
[[ "$(__gpy_prev_bg_token "blue")" == "blue" ]] || { echo "FAIL: plain prev_bg token"; exit 1; }
[[ "$(__gpy_prev_bg_token "#5277C3")" == "_5277C3" ]] || { echo "FAIL: hex prev_bg token"; exit 1; }
echo "✓ prev_bg token derivation matches Rust"

# Cache read selects the prev_bg-specific file, with a fallback to the none file.
pb_repo=$(mktemp -d)
git -C "$pb_repo" init -q
pb_real=$(realpath "$pb_repo" 2>/dev/null || echo "$pb_repo")
pb_key="$(__gpy_path_to_cache_key "$pb_real")"
echo "blue-render" > "$cache_dir/$pb_key.git.blue.bash"
echo "none-render" > "$cache_dir/$pb_key.git.none.bash"

got_blue="$(__gpy_read_instant_cache "git" "$pb_repo" "blue")"
[[ "$got_blue" == "blue-render" ]] || { echo "FAIL: prev_bg=blue did not read .git.blue.bash (got '$got_blue')"; rm -rf "$pb_repo"; exit 1; }
echo "✓ cache read selects prev_bg-specific file"

rm -f "$cache_dir/$pb_key.git.blue.bash"
got_fallback="$(__gpy_read_instant_cache "git" "$pb_repo" "blue")"
[[ "$got_fallback" == "none-render" ]] || { echo "FAIL: missing context file did not fall back to none (got '$got_fallback')"; rm -rf "$pb_repo"; exit 1; }
echo "✓ cache read falls back to none render when context file absent"

rm -rf "$pb_repo"
rm -f "$cache_dir/$pb_key.git.none.bash"

# Request payloads MUST carry prev_bg (the bug: bash omitted it). Stub the
# transport to capture the JSON to a file — duration/character send inside a
# command-substitution subshell, so an in-memory var would not propagate. The
# stub also echoes a response so request wrappers skip the oneshot fallback.
__gpy_capture_file=$(mktemp)
__gpy_send_json() { printf '%s' "$1" > "$__gpy_capture_file"; echo "stubbed-response"; return 0; }

assert_payload_has_prev_bg() {
    local label="$1" expected="$2" captured
    captured="$(cat "$__gpy_capture_file" 2>/dev/null)"
    case "$captured" in
        *"\"prev_bg\":\"$expected\""*) echo "✓ $label payload includes prev_bg" ;;
        *) echo "FAIL: $label payload missing prev_bg: $captured"; rm -f "$__gpy_capture_file"; exit 1 ;;
    esac
}

: > "$__gpy_capture_file"
__gpy_trigger_data_refresh "git" "$PWD" "false" "magenta"
assert_payload_has_prev_bg "data refresh" "magenta"

: > "$__gpy_capture_file"
__gpy_request "directory" "$PWD" "bash-prompt" "false" "cyan" >/dev/null 2>&1
assert_payload_has_prev_bg "directory request" "cyan"

: > "$__gpy_capture_file"
__gpy_request_duration "1500" "false" "yellow" >/dev/null 2>&1
assert_payload_has_prev_bg "duration request" "yellow"

: > "$__gpy_capture_file"
__gpy_request_character "1" "true" "green" >/dev/null 2>&1
assert_payload_has_prev_bg "character request" "green"

rm -f "$__gpy_capture_file"

# #729: `lang` requests forward $VIRTUAL_ENV, else $CONDA_PREFIX for a non-base
# conda env; `git` requests never carry virtual_env. Both send paths are checked
# through the stubbed __gpy_send_json above.
echo "=== virtual_env Forwarding ==="
__gpy_capture_venv() {
    local sender="$1" op="$2" venv="$3" conda="$4" conda_env="$5"
    : > "$__gpy_capture_file"
    (
        unset VIRTUAL_ENV CONDA_PREFIX CONDA_DEFAULT_ENV
        [[ -n "$venv" ]] && export VIRTUAL_ENV="$venv"
        [[ -n "$conda" ]] && export CONDA_PREFIX="$conda"
        [[ -n "$conda_env" ]] && export CONDA_DEFAULT_ENV="$conda_env"
        if [[ "$sender" == "refresh" ]]; then
            __gpy_trigger_data_refresh "$op" "$PWD" "false" ""
        else
            __gpy_request "$op" "$PWD" "json" "false" "" >/dev/null 2>&1
        fi
    )
    cat "$__gpy_capture_file"
}
_assert_venv_forwarding() {
    local label="$1" expected="$2" captured="$3"
    if [[ -n "$expected" ]]; then
        case "$captured" in
            *"\"virtual_env\":\"$expected\""*) echo "✓ $label forwards virtual_env=$expected" ;;
            *) echo "FAIL: $label should forward virtual_env=$expected: $captured"; rm -f "$__gpy_capture_file"; exit 1 ;;
        esac
    else
        case "$captured" in
            *virtual_env*) echo "FAIL: $label should not forward virtual_env: $captured"; rm -f "$__gpy_capture_file"; exit 1 ;;
            *) echo "✓ $label omits virtual_env" ;;
        esac
    fi
}
for sender in refresh request; do
    _assert_venv_forwarding "$sender lang with conda env" "/opt/conda/envs/ml" \
        "$(__gpy_capture_venv "$sender" lang "" /opt/conda/envs/ml ml)"
    _assert_venv_forwarding "$sender lang VIRTUAL_ENV beats CONDA_PREFIX" "/proj/.venv" \
        "$(__gpy_capture_venv "$sender" lang /proj/.venv /opt/conda/envs/ml ml)"
    _assert_venv_forwarding "$sender lang conda base" "" \
        "$(__gpy_capture_venv "$sender" lang "" /opt/conda base)"
    _assert_venv_forwarding "$sender git with VIRTUAL_ENV" "" \
        "$(__gpy_capture_venv "$sender" git /proj/.venv /opt/conda/envs/ml ml)"
done
rm -f "$__gpy_capture_file"
echo "✓ virtual_env forwarding: conda env, precedence, base skip, git omits"

# Render loop must advance prev_bg across segments (subshell-safe tracking). This
# is the #220 regression: before the fix bash segments received no prev_bg at all.
# Segments run inside command substitution, so the stubs record what they receive
# to a file (an in-memory var would not survive the subshell — the very reason the
# render loop, not the segment, owns the tracker).
echo "=== Render Loop prev_bg Threading ==="
__color_directory_bg="dircolor"
__color_git_clean_bg="gitcolor"
__gpy_seen_file=$(mktemp)
__gpy_segment_directory() { echo "dir:${2:-EMPTY}" >> "$__gpy_seen_file"; echo "DIR"; }
__gpy_segment_git() { echo "git:${2:-EMPTY}" >> "$__gpy_seen_file"; echo "GIT"; }
__gpy_segment_directory_detect() { return 0; }
__gpy_segment_git_detect() { return 0; }
__gpy_request_character() { echo ""; }   # avoid socket on the character path
__enabled_segments="directory git"
__gpy_render_prompt 0 >/dev/null
seen_dir="$(sed -n '1p' "$__gpy_seen_file")"
seen_git="$(sed -n '2p' "$__gpy_seen_file")"
rm -f "$__gpy_seen_file"
[[ "$seen_dir" == "dir:black" ]] || { echo "FAIL: first segment prev_bg not 'black' ($seen_dir)"; exit 1; }
[[ "$seen_git" == "git:dircolor" ]] || { echo "FAIL: second segment prev_bg not advanced to first segment bg ($seen_git)"; exit 1; }
echo "✓ render loop threads prev_bg and advances it across segments"

echo "=== Parity Check Passed ==="
