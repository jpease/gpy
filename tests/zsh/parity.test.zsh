#!/usr/bin/env zsh

# tests/zsh/parity.test.zsh
# Verify Zsh implementation matches Fish feature set

ROOT=${0:a:h:h:h}
cd "$ROOT"

# Hermetic when run directly (#820): the developer's theme export
# ($XDG_CACHE_HOME/gpy/theme-export.zsh) and exported GPY_* settings (e.g.
# GPY_LANGUAGE_ENABLED=0) would otherwise override the built-in defaults
# these checks assert on. quality-check.sh sets its own XDG dirs; this makes
# a direct `zsh tests/zsh/parity.test.zsh` behave the same.
for gpy_var in ${(k)parameters[(I)GPY_*]}; do
    unset "$gpy_var"
done
unset gpy_var
parity_xdg_root="$(mktemp -d "${TMPDIR:-/tmp}/gpy-parity-xdg.XXXXXX")"
trap 'rm -rf "$parity_xdg_root"' EXIT
mkdir -p "$parity_xdg_root/cache" "$parity_xdg_root/config"
export XDG_CACHE_HOME="$parity_xdg_root/cache" XDG_CONFIG_HOME="$parity_xdg_root/config"

echo "=== Checking Feature Parity with Fish ==="

# List of expected segments
required_segments=(
    directory
    git
    clock
    duration
    language
    status
)

source zsh/gpy.zsh

for seg in $required_segments; do
    if (( ! $+functions[__gpy_segment_$seg] )); then
        echo "FAIL: Missing segment: $seg"
        exit 1
    fi
    echo "✓ $seg"
done

# Check signal handlers
if (( ! $+functions[TRAPURG] )); then
    echo "FAIL: Missing SIGURG doorbell handler"
    exit 1
fi
echo "✓ SIGURG doorbell handler"

# Check cleanup
if [[ ! " ${zshexit_functions[@]} " =~ " __gpy_zshexit " ]]; then
    echo "FAIL: Missing zshexit cleanup"
    exit 1
fi
echo "✓ Cleanup hook"

# --- Cache key parity ---
# The hand-copied cache-key "reference values from the Rust implementation"
# that used to live here could drift from the encoder unnoticed. The key
# encoding is now pinned against a file the agent itself writes, in
# tests/zsh/path_parity.test.zsh (#647).

# --- Warm cache hit: no IPC roundtrip ---
echo "=== Warm Cache Hit Tests ==="

cache_dir="${XDG_CACHE_HOME:-$HOME/.cache}/gpy/instant-prompts"
mkdir -p "$cache_dir"

# Set up a temp git repo so __gpy_find_git_root succeeds
tmpdir=$(mktemp -d)
git -C "$tmpdir" init -q
git -C "$tmpdir" commit --allow-empty -m init -q \
    --author "Test <test@example.com>" 2>/dev/null || true

tmpdir_real=$(realpath "$tmpdir" 2>/dev/null || echo "$tmpdir")
# Cache files are keyed by prev_bg token; a read with no prev_bg resolves "none".
cache_key="$(__gpy_path_to_cache_key "$tmpdir_real")"
cache_file="$cache_dir/$cache_key.git.none.zsh"

sentinel="cached-git-output-sentinel"
echo "$sentinel" > "$cache_file"

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

# Back-date the file to make it stale (>5s old)
python3 -c "import os, time; os.utime('$cache_file', (time.time()-10, time.time()-10))" 2>/dev/null \
    || touch -t "$(date -v-10S '+%Y%m%d%H%M.%S' 2>/dev/null)" "$cache_file" 2>/dev/null \
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

source zsh/segments/language.zsh

detect_proj=$(mktemp -d)
touch "$detect_proj/Cargo.toml"
mkdir -p "$detect_proj/src/nested/deep"

_assert_detect() {
    local label=$1 expected=$2 dir=$3
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

_assert_detect "detects at project root" 0 "$detect_proj"
_assert_detect "detects in src/ subdir" 0 "$detect_proj/src"
_assert_detect "detects in deeply nested subdir" 0 "$detect_proj/src/nested/deep"

detect_empty=$(mktemp -d)
_assert_detect "not detected in plain dir" 1 "$detect_empty"

rm -rf "$detect_proj" "$detect_empty"

# --- Non-Git language cache hit (#173) ---
echo "=== Non-Git Language Cache Tests ==="

nongit=$(mktemp -d)
touch "$nongit/package.json"
nongit_real=$(realpath "$nongit" 2>/dev/null || echo "$nongit")
nongit_key="$(__gpy_path_to_cache_key "$nongit_real")"

lang_file="$cache_dir/$nongit_key.lang.none.zsh"
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
git_nongit_file="$cache_dir/$nongit_key.git.none.zsh"
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

__enabled_segments=(directory)
__prompt_color=green
__icon_prompt="❯"

# Pin the blank-line separator off for this block. It defaults on (matching
# fish), and these cases assert on the presence of *any* newline in the
# prompt, so leaving it enabled would make the two_line=0 case fail on a
# newline that has nothing to do with two-line layout.
__gpy_add_newline=0

__gpy_two_line=0
single_prompt=$(__gpy_render_prompt 0)
if [[ "$single_prompt" == *$'\n'* ]]; then
    echo "FAIL: single-line prompt (two_line=0) unexpectedly contains a newline"
    exit 1
fi
echo "✓ single-line prompt has no newline (no regression)"

__gpy_two_line=1
two_line_prompt=$(__gpy_render_prompt 0)
if [[ "$two_line_prompt" != *$'\n'* ]]; then
    echo "FAIL: two-line prompt (two_line=1) missing newline before character"
    exit 1
fi
echo "✓ two-line prompt inserts newline before character"

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
echo "blue-render" > "$cache_dir/$pb_key.git.blue.zsh"
echo "none-render" > "$cache_dir/$pb_key.git.none.zsh"

got_blue="$(__gpy_read_instant_cache "git" "$pb_repo" "blue")"
[[ "$got_blue" == "blue-render" ]] || { echo "FAIL: prev_bg=blue did not read .git.blue.zsh (got '$got_blue')"; rm -rf "$pb_repo"; exit 1; }
echo "✓ cache read selects prev_bg-specific file"

rm -f "$cache_dir/$pb_key.git.blue.zsh"
got_fallback="$(__gpy_read_instant_cache "git" "$pb_repo" "blue")"
[[ "$got_fallback" == "none-render" ]] || { echo "FAIL: missing context file did not fall back to none (got '$got_fallback')"; rm -rf "$pb_repo"; exit 1; }
echo "✓ cache read falls back to none render when context file absent"

rm -rf "$pb_repo"
rm -f "$cache_dir/$pb_key.git.none.zsh"

# Request payloads MUST carry prev_bg (the bug: zsh omitted it). Stub the
# transport to capture the JSON to a file — duration/character send inside a
# command-substitution subshell, so an in-memory var would not propagate. The
# stub also echoes a response so request wrappers skip the oneshot fallback.
__gpy_capture_file=$(mktemp)
__gpy_send_json() { printf '%s' "$1" > "$__gpy_capture_file"; echo "stubbed-response"; return 0; }

_assert_payload_has_prev_bg() {
    local label=$1 expected=$2 captured
    captured="$(cat "$__gpy_capture_file" 2>/dev/null)"
    case "$captured" in
        *"\"prev_bg\":\"$expected\""*) echo "✓ $label payload includes prev_bg" ;;
        *) echo "FAIL: $label payload missing prev_bg: $captured"; rm -f "$__gpy_capture_file"; exit 1 ;;
    esac
}

: > "$__gpy_capture_file"
__gpy_trigger_data_refresh "git" "$PWD" "false" "magenta"
_assert_payload_has_prev_bg "data refresh" "magenta"

: > "$__gpy_capture_file"
__gpy_request "directory" "$PWD" "zsh-prompt" "false" "cyan" >/dev/null 2>&1
_assert_payload_has_prev_bg "directory request" "cyan"

: > "$__gpy_capture_file"
__gpy_request_duration "1500" "false" "yellow" >/dev/null 2>&1
_assert_payload_has_prev_bg "duration request" "yellow"

: > "$__gpy_capture_file"
__gpy_request_character "1" "true" "green" >/dev/null 2>&1
_assert_payload_has_prev_bg "character request" "green"

rm -f "$__gpy_capture_file"

# Render loop must advance prev_bg across segments (subshell-safe tracking). This
# is the #220 regression: before the fix zsh segments received no prev_bg at all.
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
__enabled_segments=(directory git)
__gpy_render_prompt 0 >/dev/null
seen_dir="$(sed -n '1p' "$__gpy_seen_file")"
seen_git="$(sed -n '2p' "$__gpy_seen_file")"
rm -f "$__gpy_seen_file"
[[ "$seen_dir" == "dir:black" ]] || { echo "FAIL: first segment prev_bg not 'black' ($seen_dir)"; exit 1; }
[[ "$seen_git" == "git:dircolor" ]] || { echo "FAIL: second segment prev_bg not advanced to first segment bg ($seen_git)"; exit 1; }
echo "✓ render loop threads prev_bg and advances it across segments"

echo "=== Parity Check Passed ==="
