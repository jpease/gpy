# SPDX-License-Identifier: GPL-3.0-or-later

set -l __prompt_dir (dirname (status --current-filename))
set -g __gpy_core_dir $__prompt_dir

source $__prompt_dir/flags.fish; or return 1

if not __gpy_agent_enabled; and not __gpy_supervisor_enabled
    source $__prompt_dir/defaults.fish; or return 1
    return 0
end

set -q GPY_AGENT_ENABLED; or set -gx GPY_AGENT_ENABLED 1
set -q GPY_AGENT_SUPERVISOR_ENABLED; or set -gx GPY_AGENT_SUPERVISOR_ENABLED 1

source $__prompt_dir/util.fish; or return 1

# 3.6+ for `string escape --json`. __gpy_test_fish_version is a non-exported
# test hook: FISH_VERSION is read-only.
set -l fish_version_text $FISH_VERSION
set -q __gpy_test_fish_version; and set fish_version_text $__gpy_test_fish_version
set -l fish_version (string split '.' $fish_version_text)
set -l fish_major $fish_version[1]
set -l fish_minor $fish_version[2]
if test $fish_major -lt 3; or test $fish_major -eq 3 -a $fish_minor -lt 6
    echo "gpy: GPY needs Fish 3.6 or newer (this is $fish_version_text); see https://fishshell.com/" >&2
    return 1
end

# fish_transient_prompt arrived in Fish 4.1; 4.2 fixed clearing the extra
# lines when the final rendering is shorter than the prompt (two_line).
set -g __gpy_fish_transient_capable 0
if test $fish_major -gt 4; or test $fish_major -eq 4 -a $fish_minor -ge 2
    set -g __gpy_fish_transient_capable 1
end

set -g __gpy_core_files constants.fish renderer.fish ipc.fish supervisor.fish script.fish
for file in $__gpy_core_files
    source $__prompt_dir/$file; or return 1
end

# GPY's autoloaded functions (gpy-prompt-debug, gpy-prompt-reload, ...). The
# path is built lexically: `path resolve` follows symlinks, so an entry written
# through a symlinked spelling (macOS /tmp, a dotfile-managed ~/.config/fish)
# would never match and be appended a second time.
set -l core $__gpy_core_dir
string match -q '/*' -- $core; or set core $PWD/$core
set -l functions_dir (path normalize $core/../functions)
contains -- $functions_dir $fish_function_path; or set -a fish_function_path $functions_dir

# The segments shown when no theme export loaded (no cache, agent missing or
# failing). Keep equal to DEFAULT_ENABLED_SEGMENTS in
# gpy-agent/src/config/defaults.rs; tests/fixtures/default_segments.txt pins
# this copy, the Bash and Zsh ones and the agent's list to each other.
# Segments that cannot render without the agent omit themselves.
set -g __gpy_fallback_segments clock duration language directory git

function __gpy_load_default_config
    set -q __prompt_icons; or set -g __prompt_icons nerd
    set -q __prompt_theme; or set -g __prompt_theme default

    set -q __prompt_base_bg; or set -g __prompt_base_bg normal
    set -q __prompt_base_fg; or set -g __prompt_base_fg normal
    set -q __icon_powerline_segment_start; or set -g __icon_powerline_segment_start ""
    set -q __icon_powerline_segment_end; or set -g __icon_powerline_segment_end ""

end

# Guarded fallbacks for icons the theme export omits; theme values win.
function __gpy_initialize_icons
    if test "$__prompt_icons" = nerd
        set -q __icon_status_ok; or set -g __icon_status_ok "$__gpy_icon_nerd_status_ok"
        set -q __icon_status_fail; or set -g __icon_status_fail "$__gpy_icon_nerd_status_fail"
        set -q __icon_duration; or set -g __icon_duration "$__gpy_icon_nerd_duration"
        set -q __icon_prompt; or set -g __icon_prompt "$__gpy_icon_nerd_prompt"
        set -q __icon_git_staged; or set -g __icon_git_staged "$__gpy_icon_nerd_git_staged"
        set -q __icon_git_unstaged; or set -g __icon_git_unstaged "$__gpy_icon_nerd_git_unstaged"
        set -q __icon_git_untracked; or set -g __icon_git_untracked "$__gpy_icon_nerd_git_untracked"
    else
        set -q __icon_status_ok; or set -g __icon_status_ok "$__gpy_icon_ascii_status_ok"
        set -q __icon_status_fail; or set -g __icon_status_fail "$__gpy_icon_ascii_status_fail"
        set -q __icon_duration; or set -g __icon_duration "$__gpy_icon_ascii_duration"
        set -q __icon_prompt; or set -g __icon_prompt "$__gpy_icon_ascii_prompt"
        set -q __icon_git_staged; or set -g __icon_git_staged "$__gpy_icon_ascii_git_staged"
        set -q __icon_git_unstaged; or set -g __icon_git_unstaged "$__gpy_icon_ascii_git_unstaged"
        set -q __icon_git_untracked; or set -g __icon_git_untracked "$__gpy_icon_ascii_git_untracked"
    end
end

# Source the cached theme export (no fork), else spawn `gpy-agent theme export`.
# The script segment list is erased first, so a failed export never leaves an
# inherited or stale list; __gpy_g_sync adopts whatever the export assigned.
# MODE `fresh` skips the cache file (it may be stale) and always asks the
# agent; the export is fetched before any state is touched, so a failing agent
# leaves the running theme alone.
function __gpy_apply_theme_export --argument-names mode --description 'source the theme export cache or spawn the agent to produce it'
    if test "$mode" != fresh
        __gpy_g_clear_styles
        set -e __gpy_script_segments
    end
    __gpy_source_theme_export $mode
    set -l export_status $status
    if test "$mode" = fresh; and test $export_status -ne 0
        return $export_status
    end
    __gpy_g_sync
    return $export_status
end

function __gpy_source_theme_export --argument-names mode
    set -l cache_path (__gpy_theme_export_cache_path)
    set -l have_cache 0
    test -n "$cache_path" -a -f "$cache_path"; and set have_cache 1

    if test "$mode" != fresh; and test $have_cache -eq 1
        set -g __gpy_theme_export_mtime (__gpy_file_mtime "$cache_path")
        source "$cache_path"
        return $status
    end

    # A stale cache file must not be sourced by the next re-registration
    # (__gpy_reload_if_theme_export_changed): record its mtime as seen.
    if test "$mode" = fresh; and test $have_cache -eq 1
        set -g __gpy_theme_export_mtime (__gpy_file_mtime "$cache_path")
    else
        set -e __gpy_theme_export_mtime
    end

    set -l agent_binary (__gpy_resolve_agent_binary)
    test -n "$agent_binary"; or return 1

    if test "$mode" = fresh
        set -l export_text ("$agent_binary" theme export --shell fish)
        set -l export_status $status
        test $export_status -eq 0; and set -q export_text[1]; or return 1
        __gpy_g_clear_styles
        set -e __gpy_script_segments
        # `source` from a pipe leaves $status as it found it, and erasing an
        # unset variable leaves 4; reset it so the export's own status shows.
        true
        string join \n -- $export_text | source
        return $status
    end

    "$agent_binary" theme export --shell fish 2>/dev/null | source
    return $status
end

function __gpy_load_theme
    if __gpy_apply_theme_export
        return 0
    end

    __gpy_theme_export_binary_missing; and __gpy_log_error init "gpy-agent command not found - cannot load theme"
    return 1
end

# Variable `theme export` sets to a plugin segment's file. The token is the
# segment name lowercased with `-` turned into `_`, nothing else; it must match
# `SegmentName::variable_token` in gpy-agent/src/plugin/manifest.rs.
function __gpy_plugin_segment_file_var --argument-names segment
    printf '__gpy_plugin_segment_file_%s\n' (string replace -a -- - _ "$segment" | string lower)
end

# Source one segment's file: built-in first, then the plugin file variable,
# then a script segment from the theme export (core/script.fish).
function __gpy_source_segment --argument-names segment
    set -l segment_file $__gpy_core_dir/../segments/$segment.fish
    if test -f $segment_file
        source $segment_file
        return 0
    end

    set -l plugin_file_var (__gpy_plugin_segment_file_var "$segment")
    if set -q $plugin_file_var
        set -l plugin_file $$plugin_file_var
        if test -f $plugin_file
            source $plugin_file
            return 0
        end
        __gpy_log_warn init "Plugin segment '$segment' missing file at $plugin_file"
        return 1
    end

    __gpy_g_define_segment "$segment"; and return 0

    __gpy_log_warn init "Segment '$segment' not found at $segment_file"
    return 1
end

function __gpy_load_segments
    for segment in $argv
        __gpy_source_segment $segment
    end
end

# Load segments enabled after init (theme switch); reload paths only, never per prompt.
function __gpy_ensure_segments_loaded --description 'source segment files newly added to __enabled_segments'
    for segment in $__enabled_segments
        functions -q segment_{$segment}_detect; and continue
        __gpy_source_segment $segment
        functions -q segment_{$segment}_init; and segment_{$segment}_init
    end
    return 0
end

# Succeeds when a failed export is due to a missing agent binary (no cache either).
function __gpy_theme_export_binary_missing
    set -l cache_path (__gpy_theme_export_cache_path)
    test -n "$cache_path" -a -f "$cache_path"; and return 1
    test -z (__gpy_resolve_agent_binary)
end

__gpy_load_theme >/dev/null

# An export that set the list (even to nothing) wins; the fallback only fills
# in when none did, and before the segment files load.
set -q __enabled_segments; or set -g __enabled_segments $__gpy_fallback_segments

# Test hook (non-exported, never set by the agent or the theme).
if set -q __gpy_test_segments[1]
    set -g __enabled_segments $__gpy_test_segments
end

# Deprecated Fish-only switches. They keep working for one minor release and
# warn once per name at the first prompt.
set -g __gpy_deprecated_switches
if test "$GPY_SHOW_LANGUAGES" = 0
    set -a __gpy_deprecated_switches GPY_SHOW_LANGUAGES
    set -g __enabled_segments clock duration directory git
end

if set -q GPY_MINIMAL_SEGMENTS
    set -a __gpy_deprecated_switches GPY_MINIMAL_SEGMENTS
    set -g __enabled_segments directory
end

if set -q GPY_TEST_SEGMENTS
    set -a __gpy_deprecated_switches GPY_TEST_SEGMENTS
    set -q GPY_MINIMAL_SEGMENTS; or set -g __enabled_segments (string split ' ' $GPY_TEST_SEGMENTS)
end

if set -q __gpy_deprecated_switches[1]
    function __gpy_warn_deprecated --on-event fish_prompt
        for name in $__gpy_deprecated_switches
            echo "gpy: $name is deprecated; use gpy enable or gpy disable instead" >&2
        end
        set -e __gpy_deprecated_switches
        functions -e __gpy_warn_deprecated
    end
end

__gpy_load_segments $__enabled_segments

for segment in $__enabled_segments
    if functions -q segment_{$segment}_init
        segment_{$segment}_init
    end
end

# Reload the theme and config into the running shell. MODE `fresh` re-reads
# config.toml through `gpy-agent theme export` instead of the cache file; any
# other value is ignored.
function __gpy_reload_theme --argument-names mode --description 'reload GPY theme from config (internal helper)'
    __gpy_apply_theme_export $mode; or return 1

    # The export erases status icons the theme leaves unset; refill the
    # nerd/ascii fallbacks from the current __prompt_icons.
    __gpy_initialize_icons

    __gpy_ensure_segments_loaded
    __gpy_clear_render_memos

    if status is-interactive
        commandline -f force-repaint 2>/dev/null
    end

    return 0
end

set -g __gpy_config_initialized 1

__gpy_load_default_config

# After default config (sets __prompt_icons) and theme load (theme icons win).
__gpy_initialize_icons

set -g __gpy_is_root 0
if test (id -u) -eq 0
    set -g __gpy_is_root 1
end

# sudo exports SUDO_USER into the elevated shell.
set -g __gpy_is_sudo 0
if test -n "$SUDO_USER"
    set -g __gpy_is_sudo 1
end

# Cache SSH-session detection once — pure env-var checks, no fork.
# Non-empty checks, as bash/zsh: a defined-but-empty SSH_* var is not SSH.
set -g __gpy_is_ssh 0
if test -n "$SSH_CONNECTION"; or test -n "$SSH_CLIENT"; or test -n "$SSH_TTY"
    set -g __gpy_is_ssh 1
end

true
