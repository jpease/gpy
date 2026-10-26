#!/usr/bin/env fish
# `__gpy_maybe_refresh` throttles each (op, cache suffix, prev_bg token, root)
# context to one refresh per 500 ms. Contexts that differ only in punctuation
# (/tmp/a-b, /tmp/a_b, /tmp/a.b) must not share a bucket, or a refresh for one
# suppresses the others. The detached send is replaced by a recorder; the
# throttle and bucket naming under test are the real functions. The clock is
# frozen as fish_prompt freezes it per render: without a render timestamp the
# fallback clock has whole-second resolution, so a repeat that crossed a second
# boundary would read as 1000 ms later and escape the throttle.

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/tests/lib/test_helpers.fish"
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"

print_test_header "Fish refresh-throttle buckets are per context"

set -l tmp (mktemp -d)
printf '%s\n' '#!/bin/sh' >$tmp/stub-agent
chmod +x $tmp/stub-agent
set -gx GPY_AGENT_BINARY_PATH $tmp/stub-agent

set -g sent_payloads
function __gpy_register_with_agent
end
function __gpy_ipc_send_detached
    set -ga sent_payloads $argv[1]
end
function expect_sends --argument-names name expected
    set -l got (count $sent_payloads)
    if test $got -eq $expected
        pass "$name"
    else
        fail "$name" "expected $expected refreshes, got $got: $sent_payloads"
    end
end

set -g __gpy_prompt_now (date +%s)

for root in /tmp/a-b /tmp/a_b /tmp/a.b
    __gpy_maybe_refresh lang $root lang "" black ""
end
expect_sends "roots differing only in punctuation refresh independently" 3

__gpy_maybe_refresh lang /tmp/a-b lang "" black ""
expect_sends "a repeat refresh of the same context is throttled" 3

set -g sent_payloads
for root in /tmp/a-b /tmp/a_b /tmp/a.b
    __gpy_maybe_refresh lang $root lang "" black "" black
end
expect_sends "per-prev_bg variant buckets are per root too" 3

rm -rf $tmp

test_finish "Fish refresh-throttle buckets are per context"
