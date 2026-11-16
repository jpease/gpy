#!/usr/bin/env fish
# tests/fish/segment_bg_empty_cache_entry.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The language and git pills move __gpy_last_segment_bg on only when they
# printed something, like Bash and Zsh: an instant-cache read that yields no
# text (a missing variant, an empty entry) must leave the next segment's
# chevron on the colour of the pill actually drawn before it.

source (dirname (status --filename))/../lib/test_helpers.fish

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
source "$repo_root/fish/core/constants.fish"
source "$repo_root/fish/core/util.fish"
source "$repo_root/fish/core/ipc.fish"
source "$repo_root/fish/segments/language.fish"
source "$repo_root/fish/segments/git.fish"

set -g __color_language_bg yellow
set -g __test_cache_text ""
set -g __test_cache_status 0
function __gpy_read_instant_cache
    printf '%s' "$__test_cache_text"
    return $__test_cache_status
end
function __gpy_maybe_refresh
end
function __gpy_git_segment_bg
    printf gitbg
end

for seg in language git
    set -g __test_cache_text ""
    set -g __gpy_last_segment_bg blue
    set -l out (segment_{$seg}_render "" "" | string collect)
    check "$seg: an empty cache entry prints nothing" "" "$out"
    segment_{$seg}_render "" "" >/dev/null
    check "$seg: an empty cache entry leaves __gpy_last_segment_bg alone" blue "$__gpy_last_segment_bg"
end

set -g __test_cache_text PILL
set -g __gpy_last_segment_bg blue
segment_language_render "" "" >/dev/null
check "language: a printed pill tracks its bg" yellow "$__gpy_last_segment_bg"
set -g __gpy_last_segment_bg blue
segment_git_render "" "" >/dev/null
check "git: a printed pill tracks the agent-recorded bg" gitbg "$__gpy_last_segment_bg"

test_finish "segment bg tracking (empty cache entry) tests"
