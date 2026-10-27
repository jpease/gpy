#!/usr/bin/env fish
# Test: hostname/username (bundled G plugins) bg tracking. The plugin's
# stored output decides visibility: no output hides the segment (detect fails,
# __gpy_last_segment_bg untouched), so the next chevron never blends against a
# pill that did not render. Shown, the pill tracks the background it actually
# drew: the variant's (`ssh` / `root`) colour when one is exported, else the
# segment's own.
#
# Sources fish/core/script.fish and drives the stored G state directly; the
# runner itself is covered by script_segment_adapter.test.fish.

source (dirname (status --filename))/../lib/test_helpers.fish

set -l script_dir (path dirname (status --current-filename))
set -l repo_root (cd "$script_dir/../.." && pwd)

source "$repo_root/fish/core/script.fish"

function __gpy_section_pill --argument-names bg fg label
    set -g __test_pill_bg "$bg"
    set -g __test_pill_label "$label"
    printf PILL
end

set -g __gpy_script_segments hostname username
set -g __gpy_script_hostname_name hostname
set -g __gpy_script_username_name username
__gpy_g_sync
__gpy_g_define_segment hostname
__gpy_g_define_segment username

set -g __color_hostname_bg blue
set -g __color_hostname_fg white
set -g __color_hostname_ssh_bg magenta
set -g __icon_hostname ""
set -g __color_username_bg yellow
set -g __color_username_fg black
set -g __color_username_root_bg red
set -g __icon_username ""

# Renders one segment the way the prompt does: detect, then render.
function __test_render --argument-names seg
    set -g __test_pill_bg ""
    set -g __test_pill_label ""
    set -g __gpy_last_segment_bg unchanged
    set -g __test_out ""
    if segment_{$seg}_detect
        set -g __test_out (segment_{$seg}_render "" | string collect)
        segment_{$seg}_render "" >/dev/null
    end
end

# --- no plugin output: hidden, tracker untouched ---

for seg in hostname username
    set -g __gpy_g_t_{$seg}_text ""
    set -g __gpy_g_t_{$seg}_variant ""
    __test_render $seg
    check "$seg: empty plugin output hides the segment" "" "$__test_out"
    check "$seg: empty plugin output leaves __gpy_last_segment_bg alone" unchanged "$__gpy_last_segment_bg"
end

# --- shown without a variant: the segment's own bg ---

set -g __gpy_g_t_hostname_text box
__test_render hostname
check "hostname: shown pill is drawn" PILL "$__test_out"
check "hostname: label has one leading space" " box" "$__test_pill_label"
check "hostname: tracks its own bg" blue "$__gpy_last_segment_bg"

set -g __gpy_g_t_username_text bob
__test_render username
check "username: tracks its own bg" yellow "$__gpy_last_segment_bg"

# --- shown with a variant: the variant's bg is drawn and tracked ---

set -g __gpy_g_t_hostname_variant ssh
__test_render hostname
check "hostname ssh: pill uses the variant bg" magenta "$__test_pill_bg"
check "hostname ssh: tracks the variant bg" magenta "$__gpy_last_segment_bg"

set -g __gpy_g_t_username_text root
set -g __gpy_g_t_username_variant root
__test_render username
check "username root: pill uses the variant bg" red "$__test_pill_bg"
check "username root: tracks the variant bg" red "$__gpy_last_segment_bg"

test_finish "segment bg tracking (plugin output) tests"
