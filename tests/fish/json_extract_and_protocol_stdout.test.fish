#!/usr/bin/env fish
# tests/fish/json_extract_and_protocol_stdout.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression/contract tests for #612:
#   1. __gpy_json_extract_string / __gpy_json_extract_int parse a flat JSON
#      blob fork-free (no `| tail -n 1` subprocess), against a literal JSON
#      string -- covers the exact shape __gpy_get_agent_version parses from
#      the agent's `{"op":"status"}` response.
#   2. __gpy_get_agent_version, wired through those helpers, still populates
#      __gpy_agent_version / __gpy_agent_protocol_version correctly.
#   3. __gpy_check_protocol_version prints NOTHING on stdout on a mismatch
#      (previously an errant `echo ""` leaked a blank line to stdout from
#      inside a fish_prompt event handler, ipc.fish ~#821); only stderr
#      should carry the warning.
#
# No socket, no running agent: __gpy_ipc_send / __gpy_get_agent_version are
# redefined to return canned data.

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

source fish/core/constants.fish
source fish/core/util.fish
source fish/core/ipc.fish

set -g pass_count 0
set -g fail_count 0

function check --argument-names label expected actual
    if test "$actual" = "$expected"
        set -g pass_count (math $pass_count + 1)
        echo "PASS: $label"
    else
        set -g fail_count (math $fail_count + 1)
        echo "FAIL: $label (expected [$expected], got [$actual])"
    end
end

# --- 1. __gpy_json_extract_string / __gpy_json_extract_int: literal JSON ---

set -l status_json '{"AgentStatus":{"version":"1.2.3","protocol_version":7,"watched_repos":5,"registered_clients":2,"cache_entries":9}}'

check "extract_string: version field" \
    1.2.3 (__gpy_json_extract_string version $status_json)

check "extract_int: protocol_version field" \
    7 (__gpy_json_extract_int protocol_version $status_json)

check "extract_int: another integer field (watched_repos)" \
    5 (__gpy_json_extract_int watched_repos $status_json)

set -l absent_str_result (__gpy_json_extract_string missing_field $status_json)
set -l absent_str_status $status
check "extract_string: absent field returns empty" "" "$absent_str_result"
check "extract_string: absent field exits 1" 1 "$absent_str_status"

set -l absent_int_result (__gpy_json_extract_int missing_field $status_json)
set -l absent_int_status $status
check "extract_int: absent field exits 1" 1 "$absent_int_status"

# A version string containing digits must not confuse the int extractor into
# matching the wrong field (protocol_version, not version, is numeric-only).
set -l tricky_json '{"version":"0.10.99","protocol_version":1}'
check "extract_string: numeric-looking version string" \
    0.10.99 (__gpy_json_extract_string version $tricky_json)
check "extract_int: protocol_version alongside a numeric-looking version" \
    1 (__gpy_json_extract_int protocol_version $tricky_json)

# --- 2. __gpy_get_agent_version end-to-end against the extractors above ---

set -gx GPY_AGENT_ENABLED 1
function __gpy_ipc_send
    echo '{"AgentStatus":{"version":"9.9.9","protocol_version":42,"watched_repos":0,"registered_clients":0,"cache_entries":0}}'
end

set -l printed (__gpy_get_agent_version)
check "get_agent_version: prints the version" 9.9.9 "$printed"
check "get_agent_version: sets __gpy_agent_version" 9.9.9 "$__gpy_agent_version"
check "get_agent_version: sets __gpy_agent_protocol_version" 42 "$__gpy_agent_protocol_version"

# --- 3. __gpy_check_protocol_version emits nothing on stdout on a mismatch ---

functions -e __gpy_get_agent_version 2>/dev/null
function __gpy_get_agent_version
    set -g __gpy_agent_protocol_version 999
    echo mismatched-agent
    return 0
end

set -l tmp_dir (mktemp -d)
set -l stdout_file "$tmp_dir/stdout"
set -l stderr_file "$tmp_dir/stderr"

__gpy_check_protocol_version >"$stdout_file" 2>"$stderr_file"
set -l check_status $status

check "check_protocol_version: reports failure on mismatch" 1 "$check_status"

set -l stdout_bytes (wc -c <"$stdout_file" | string trim)
check "check_protocol_version: stdout is empty on mismatch" 0 "$stdout_bytes"

if grep -q 'Protocol Version Mismatch' "$stderr_file"
    check "check_protocol_version: warning still reaches stderr" pass pass
else
    check "check_protocol_version: warning still reaches stderr" pass fail
end

rm -rf "$tmp_dir"

if test $fail_count -gt 0
    echo "RESULT: $fail_count failed, $pass_count passed"
    exit 1
end
echo "PASS: all $pass_count json-extract/protocol-stdout tests passed"
