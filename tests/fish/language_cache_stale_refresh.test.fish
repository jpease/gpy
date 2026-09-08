#!/usr/bin/env fish
# Test: stale language instant cache still renders but sets the stale marker

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"

set -g pass_count 0
set -g fail_count 0

function check --argument-names label result
    if test "$result" = pass
        set -g pass_count (math $pass_count + 1)
        echo "PASS: $label"
    else
        set -g fail_count (math $fail_count + 1)
        echo "FAIL: $label"
    end
end

# --- Setup: fake cache directory and git root ---
set -l tmp_dir (mktemp -d)
set -l fake_git_root "$tmp_dir/myproject"
mkdir -p "$fake_git_root/.git"

set -gx XDG_CACHE_HOME "$tmp_dir/cache"
set -l cache_dir "$tmp_dir/cache/gpy/instant-prompts"
mkdir -p "$cache_dir"

# Derive cache key via the shared helper so this stays in lockstep with the
# Rust implementation. path resolve canonicalises symlinks (e.g. /tmp ->
# /private/tmp on macOS), matching what __gpy_find_git_root returns.
set -l resolved_root (path resolve -- "$fake_git_root" 2>/dev/null)
if test -z "$resolved_root"
    set resolved_root $fake_git_root
end
set -l cache_key (__gpy_path_to_cache_key "$resolved_root")
set -l cache_file "$cache_dir/$cache_key.lang.none.ansi"

# Write a cache file with an old mtime (well past the 30s TTL)
printf '\e[32m ruby 3.4.0 \e[0m' >"$cache_file"
touch -t 200001010000 "$cache_file"

# Use current time snapshot (normally set by fish_prompt)
set -g __gpy_prompt_now (date +%s)
set -g GPY_LANGUAGE_CACHE_TTL_SECONDS 30

set -l content (__gpy_read_instant_cache lang "$fake_git_root")
set -l content_status $status

# Content should still be served despite staleness
if test -n "$content"
    check "stale cache content is served" pass
else
    check "stale cache content is served" fail
end

# #612: staleness is now the exit-code status (2/6), not a side-effect global.
if __gpy_cache_status_stale $content_status
    check "stale marker is set when cache exceeds TTL" pass
else
    check "stale marker is set when cache exceeds TTL" fail
end

# --- Fresh cache should NOT set the stale marker ---
touch "$cache_file" # reset mtime to now

set -l content2 (__gpy_read_instant_cache lang "$fake_git_root")
set -l content2_status $status

if test -n "$content2"
    check "fresh cache content is served" pass
else
    check "fresh cache content is served" fail
end

if not __gpy_cache_status_stale $content2_status
    check "stale marker is NOT set for fresh cache" pass
else
    check "stale marker is NOT set for fresh cache" fail
end

# --- #458: the stale path must actually reach the agent ---
#
# Detecting staleness is only half the self-heal: the refresh it fires has to
# leave the shell. `__gpy_request` short-circuits on any instant-cache hit —
# correct for rendering, fatal here, since the stale entry it would serve is
# exactly the one being replaced. `__gpy_trigger_data_refresh` exists to
# bypass that. These tests mock `__gpy_ipc_send` and assert on what actually
# reaches it, so no running agent is needed.
source "$repo_root/fish/segments/language.fish"

set -g __gpy_test_ipc_marker "$tmp_dir/ipc_sent"

# Stand in for a resolvable agent binary: both segment_language_render and
# __gpy_trigger_data_refresh gate their refresh on this.
function __gpy_resolve_agent_binary
    echo "$__gpy_test_ipc_marker.binary"
    return 0
end

# Record every payload that would leave the shell. Echoing a non-empty reply
# keeps __gpy_request off its oneshot-fallback path, which would otherwise try
# to exec the fake agent binary above and spray errors over the test output.
function __gpy_ipc_send
    echo "$argv[1]" >>"$__gpy_test_ipc_marker"
    echo '{}'
end

# Wait for the backgrounded (and disowned, so unwaitable) refresh to record a
# payload. Returns 0 as soon as one lands, 1 on timeout.
function await_ipc_send
    set -l deadline (math (date +%s) + 5)
    while test (date +%s) -lt $deadline
        test -s "$__gpy_test_ipc_marker"; and return 0
        sleep 0.05
    end
    test -s "$__gpy_test_ipc_marker"
end

cd "$fake_git_root"

# Scenario 1: stale-but-present cache.
rm -f "$__gpy_test_ipc_marker"
printf '\e[32m ruby 3.4.0 \e[0m' >"$cache_file"
touch -t 200001010000 "$cache_file"
# Reset prev_bg every scenario: a render leaves __gpy_last_segment_bg set to
# the language segment's own background, and a non-`none` prev_bg would make
# the next read serve the `.none` file as a variant fallback — a different
# refresh path with its own throttle, which would confound these assertions.
set -e __gpy_last_segment_bg
set -g __gpy_prompt_now (date +%s)
# Clear any throttle state so this render is not suppressed.
for throttle_var in (set -n | string match '__gpy_lang_refresh_ms_*')
    set -e $throttle_var
end

segment_language_render >/dev/null

if await_ipc_send
    check "stale cache triggers an IPC send" pass
else
    check "stale cache triggers an IPC send" fail
end

set -l stale_payload (cat "$__gpy_test_ipc_marker" 2>/dev/null | string collect)
if string match -q '*"op":"lang"*' -- "$stale_payload"
    check "stale-path payload is a lang data request" pass
else
    check "stale-path payload is a lang data request" fail
    echo "  payload was: [$stale_payload]"
end

# Scenario 2: cold miss (no cache file at all) must keep sending, unchanged.
rm -f "$__gpy_test_ipc_marker"
rm -f "$cache_file"
set -e __gpy_last_segment_bg
for throttle_var in (set -n | string match '__gpy_lang_refresh_ms_*')
    set -e $throttle_var
end

segment_language_render >/dev/null

if await_ipc_send
    check "cold cache miss still triggers an IPC send" pass
else
    check "cold cache miss still triggers an IPC send" fail
end

# Scenario 3: the 500ms per-path/suffix throttle still suppresses a second
# stale refresh fired immediately after the first.
rm -f "$__gpy_test_ipc_marker"
printf '\e[32m ruby 3.4.0 \e[0m' >"$cache_file"
touch -t 200001010000 "$cache_file"
set -e __gpy_last_segment_bg
# Deliberately leave the throttle state from scenario 2 in place.
segment_language_render >/dev/null
sleep 0.3

if test -s "$__gpy_test_ipc_marker"
    check "throttled stale refresh sends nothing within 500ms" fail
    echo "  payload was: ["(cat "$__gpy_test_ipc_marker" | string collect)"]"
else
    check "throttled stale refresh sends nothing within 500ms" pass
end

cd "$repo_root"

# Cleanup
rm -rf "$tmp_dir"

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed, $pass_count passed"
    exit 1
end
echo "PASS: all $pass_count language cache stale tests passed"
