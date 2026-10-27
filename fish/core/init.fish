# SPDX-License-Identifier: GPL-3.0-or-later

set -l __prompt_dir (dirname (status --current-filename))
set -g __gpy_core_dir $__prompt_dir

source $__prompt_dir/flags.fish

if not __gpy_agent_enabled; and not __gpy_supervisor_enabled
    source $__prompt_dir/defaults.fish
    return 0
end

set -q GPY_AGENT_ENABLED; or set -gx GPY_AGENT_ENABLED 1
set -q GPY_AGENT_SUPERVISOR_ENABLED; or set -gx GPY_AGENT_SUPERVISOR_ENABLED 1

source $__prompt_dir/util.fish

# 3.6+ for `string escape --json`.
set -l fish_version (string split '.' $FISH_VERSION)
set -l fish_major $fish_version[1]
set -l fish_minor $fish_version[2]
if test $fish_major -lt 3; or test $fish_major -eq 3 -a $fish_minor -lt 6
    __gpy_log_error init "GPY requires Fish 3.6+ for modern features. Current version: $FISH_VERSION"
    __gpy_log_error init "Please upgrade Fish: https://fishshell.com/"
    return 1
end

# fish_transient_prompt arrived in Fish 4.1; 4.2 fixed clearing the extra
# lines when the final rendering is shorter than the prompt (two_line).
set -g __gpy_fish_transient_capable 0
if test $fish_major -gt 4; or test $fish_major -eq 4 -a $fish_minor -ge 2
    set -g __gpy_fish_transient_capable 1
end

source $__prompt_dir/constants.fish
source $__prompt_dir/renderer.fish

source $__prompt_dir/ipc.fish
source $__prompt_dir/supervisor.fish
source $__prompt_dir/script.fish

function __gpy_load_default_config
    set -q __enabled_segments; or set -g __enabled_segments clock duration language directory git
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
function __gpy_apply_theme_export --description 'source the theme export cache or spawn the agent to produce it'
    __gpy_g_clear_styles
    set -e __gpy_script_segments
    __gpy_source_theme_export
    set -l export_status $status
    __gpy_g_sync
    return $export_status
end

function __gpy_source_theme_export
    set -l cache_path (__gpy_theme_export_cache_path)
    if test -n "$cache_path" -a -f "$cache_path"
        set -g __gpy_theme_export_mtime (__gpy_file_mtime "$cache_path")
        source "$cache_path"
        return $status
    end

    set -e __gpy_theme_export_mtime
    set -l agent_binary (__gpy_resolve_agent_binary)
    if test -n "$agent_binary"
        "$agent_binary" theme export --format fish 2>/dev/null | source
        return $status
    end

    return 1
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

if test "$GPY_SHOW_LANGUAGES" = 0
    set -g __enabled_segments clock duration directory git
end

if set -q GPY_MINIMAL_SEGMENTS
    set -g __enabled_segments directory
else if set -q GPY_TEST_SEGMENTS
    set -g __enabled_segments (string split ' ' $GPY_TEST_SEGMENTS)
end

__gpy_load_segments $__enabled_segments

for segment in $__enabled_segments
    if functions -q segment_{$segment}_init
        segment_{$segment}_init
    end
end

source $__prompt_dir/debug.fish

function __gpy_reload_theme --description 'reload GPY theme from config (internal helper)'
    set -l verbose (test (count $argv) -gt 0; and test "$argv[1]" = "1"; and echo 1; or echo 0)

    if not __gpy_apply_theme_export
        if __gpy_theme_export_binary_missing
            test $verbose -eq 1; and echo "Error: gpy-agent command not found" >&2
            return 1
        end
        test $verbose -eq 1; and echo "Error: failed to reload theme export" >&2
        return 1
    end

    # The export erases status icons the theme leaves unset; refill the
    # nerd/ascii fallbacks from the current __prompt_icons.
    __gpy_initialize_icons

    __gpy_ensure_segments_loaded
    __gpy_clear_render_memos

    if status is-interactive
        commandline -f force-repaint 2>/dev/null
    end

    test $verbose -eq 1; and echo "Theme reloaded successfully"
    return 0
end

function prompt-reload --description 'manually reload GPY theme and config (for debugging)'
    if __gpy_reload_theme 1
        echo "Note: For automatic hot-reload, ensure the agent is running with 'gpy-agent start'"
    else
        echo "Error: Failed to reload theme. Please ensure GPY is installed correctly."
        return 1
    end
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
