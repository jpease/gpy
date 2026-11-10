#!/usr/bin/env fish
# tests/fish/json_extract.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# __gpy_json_extract_string / __gpy_json_extract_int parse a flat JSON blob
# fork-free (no `| tail -n 1` subprocess), against literal JSON strings. No
# socket, no running agent.

source (dirname (status --filename))/../lib/test_helpers.fish

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

source fish/core/constants.fish
source fish/core/util.fish
source fish/core/ipc.fish

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

test_finish "json-extract tests"
