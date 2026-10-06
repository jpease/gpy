# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# INITIALIZATION
# ============================================================================

# Get the directory containing this script
set -l __prompt_dir (dirname (status --current-filename))
set -g __gpy_core_dir $__prompt_dir

# Early exit if GPY is completely disabled
if test "$GPY_AGENT_ENABLED" = 0 -a "$GPY_AGENT_SUPERVISOR_ENABLED" = 0
    # Set minimal defaults for basic prompt functionality
    set -q __enabled_segments; or set -g __enabled_segments clock duration directory
    set -q __prompt_icons; or set -g __prompt_icons nerd
    set -q __prompt_theme; or set -g __prompt_theme default

    # fish/functions/fish_prompt.fish is autoloaded by Fish independently of
    # this file (it lives under functions/, which Fish's autoloader scans
    # regardless of whether GPY's own init ran) and reads these globals on
    # every render. Without them it hits unset-variable errors on every
    # prompt when GPY is disabled (#452). Each default is guarded so a real
    # theme export (were one to run later) always wins.
    set -q __gpy_is_root; or begin
        set -g __gpy_is_root 0
        test (id -u) -eq 0; and set -g __gpy_is_root 1
    end
    set -q __prompt_color; or set -g __prompt_color green
    set -q __root_prompt_color; or set -g __root_prompt_color red
    set -q __icon_root_prompt; or set -g __icon_root_prompt "!❯!"
    # Mirror __gpy_initialize_icons' nerd/ascii split: constants.fish is not
    # sourced on this path, so the literals are inlined, but an ascii-preferring
    # user must not get nerd glyphs just because GPY is disabled.
    if test "$__prompt_icons" = nerd
        set -q __icon_prompt; or set -g __icon_prompt "❯"
        set -q __icon_status_ok; or set -g __icon_status_ok "✔"
        set -q __icon_status_fail; or set -g __icon_status_fail "✖"
    else
        set -q __icon_prompt; or set -g __icon_prompt ">"
        set -q __icon_status_ok; or set -g __icon_status_ok ok
        set -q __icon_status_fail; or set -g __icon_status_fail err
    end

    return 0
end

# Set default values if not already set
set -q GPY_AGENT_ENABLED; or set -gx GPY_AGENT_ENABLED 1
set -q GPY_AGENT_SUPERVISOR_ENABLED; or set -gx GPY_AGENT_SUPERVISOR_ENABLED 1

# Load util.fish early for logging functions
source $__prompt_dir/util.fish

# Check Fish version requirement (3.6+ for string escape --json and modern features)
set -l fish_version (string split '.' $FISH_VERSION)
set -l fish_major $fish_version[1]
set -l fish_minor $fish_version[2]
if test $fish_major -lt 3; or test $fish_major -eq 3 -a $fish_minor -lt 6
    __gpy_log_error init "GPY requires Fish 3.6+ for modern features. Current version: $FISH_VERSION"
    __gpy_log_error init "Please upgrade Fish: https://fishshell.com/"
    return 1
end

# Load core modules
source $__prompt_dir/constants.fish
source $__prompt_dir/renderer.fish

# Load core modules
source $__prompt_dir/ipc.fish

# Load default configuration when agent is unavailable (defined early)
function __gpy_load_default_config
    # Set sensible defaults when agent is not available
    set -q __enabled_segments; or set -g __enabled_segments clock duration language directory git
    set -q __prompt_icons; or set -g __prompt_icons nerd
    set -q __prompt_theme; or set -g __prompt_theme default
    set -q __git_check_interval; or set -g __git_check_interval 1
    set -q GPY_GIT_SHOW; or set -g GPY_GIT_SHOW 1
    set -q GPY_GIT_SHOW_AHEAD_BEHIND; or set -g GPY_GIT_SHOW_AHEAD_BEHIND 1
    set -q GPY_LANG_SIG_MODE; or set -g GPY_LANG_SIG_MODE fast
    set -q GPY_DEBOUNCE_MS; or set -g GPY_DEBOUNCE_MS 300
    set -q GPY_UI_DIRECTORY_DISPLAY; or set -g GPY_UI_DIRECTORY_DISPLAY basename
    set -q GPY_UI_DIRECTORY_MAX_LENGTH; or set -g GPY_UI_DIRECTORY_MAX_LENGTH 80

    # Duration segment settings
    set -q __duration_threshold; or set -g __duration_threshold 100

    # Git color settings are now controlled by ~/.config/gpy/themes/{theme}.toml
    # The agent provides pre-rendered segments with colors from the theme

    # Initialize base theme variables for renderer (if not already set by theme export)
    set -q __prompt_base_bg; or set -g __prompt_base_bg normal
    set -q __prompt_base_fg; or set -g __prompt_base_fg normal
    set -q __icon_powerline_segment_start; or set -g __icon_powerline_segment_start ""
    set -q __icon_powerline_segment_end; or set -g __icon_powerline_segment_end ""

    # Note: All theme colors, icons, and settings are now loaded via __gpy_load_theme
    # which calls 'gpy-agent theme export --format fish'
end

# Icon initialization function
#
# Fills in fallback icon variables for any icon the theme export did not provide.
# The agent only exports a subset of `__icon_*` (e.g. `__icon_prompt`, and
# `__icon_status_*` only when the active theme defines status icons). The status
# indicator and the shell-rendered git/duration fallbacks still read these
# constants when the agent is unavailable or the theme omits them, so every icon
# must resolve to a sensible default. Each assignment is guarded with `set -q` so
# theme-exported icons always win; this also makes the function idempotent.
function __gpy_initialize_icons
    # Initialize icons based on theme setting
    if test "$__prompt_icons" = nerd
        # Use nerd font icons
        set -q __icon_status_ok; or set -g __icon_status_ok "$__gpy_icon_nerd_status_ok"
        set -q __icon_status_fail; or set -g __icon_status_fail "$__gpy_icon_nerd_status_fail"
        set -q __icon_duration; or set -g __icon_duration "$__gpy_icon_nerd_duration"
        set -q __icon_prompt; or set -g __icon_prompt "$__gpy_icon_nerd_prompt"
        set -q __icon_git_staged; or set -g __icon_git_staged "$__gpy_icon_nerd_git_staged"
        set -q __icon_git_unstaged; or set -g __icon_git_unstaged "$__gpy_icon_nerd_git_unstaged"
        set -q __icon_git_untracked; or set -g __icon_git_untracked "$__gpy_icon_nerd_git_untracked"
    else
        # Use ASCII fallback icons
        set -q __icon_status_ok; or set -g __icon_status_ok "$__gpy_icon_ascii_status_ok"
        set -q __icon_status_fail; or set -g __icon_status_fail "$__gpy_icon_ascii_status_fail"
        set -q __icon_duration; or set -g __icon_duration "$__gpy_icon_ascii_duration"
        set -q __icon_prompt; or set -g __icon_prompt "$__gpy_icon_ascii_prompt"
        set -q __icon_git_staged; or set -g __icon_git_staged "$__gpy_icon_ascii_git_staged"
        set -q __icon_git_unstaged; or set -g __icon_git_unstaged "$__gpy_icon_ascii_git_unstaged"
        set -q __icon_git_untracked; or set -g __icon_git_untracked "$__gpy_icon_ascii_git_untracked"
    end
end

# Source the agent's pre-rendered theme-export cache file if present (no
# fork); otherwise fall back to spawning `gpy-agent theme export --format
# fish` synchronously and sourcing its output. This is the single "how do we
# get the current theme's fish variables into this shell" primitive shared
# by shell startup (__gpy_load_theme), the manual reload command
# (__gpy_reload_theme / prompt-reload), and the agent's .reload doorbell
# handler (#576) -- it used to be hand-copied three times and drifted.
#
# Returns the status of whichever path ran (source's status, the spawned
# command's status, or 1 if neither a cache file nor an agent binary could
# be found).
function __gpy_apply_theme_export --description 'source the theme export cache or spawn the agent to produce it'
    set -l cache_path (__gpy_theme_export_cache_path)
    if test -n "$cache_path" -a -f "$cache_path"
        source "$cache_path"
        return $status
    end

    set -l agent_binary (__gpy_resolve_agent_binary)
    if test -n "$agent_binary"
        "$agent_binary" theme export --format fish 2>/dev/null | source
        return $status
    end

    return 1
end

# Load theme colors from agent export (TOML-based)
function __gpy_load_theme
    if __gpy_apply_theme_export
        return 0
    end

    # __gpy_apply_theme_export failed. Only log the specific "no agent
    # binary" error when that's actually why -- a cache file that failed to
    # source, or an agent binary that spawned but exited non-zero, should
    # propagate its own (silent) failure status here, same as before this
    # was factored out of this function.
    set -l cache_path (__gpy_theme_export_cache_path)
    if test -z "$cache_path" -o ! -f "$cache_path"
        set -l agent_binary (__gpy_resolve_agent_binary)
        test -z "$agent_binary"; and __gpy_log_error init "gpy-agent command not found - cannot load theme"
    end
    return 1
end

# Config and theme change detection is handled by the agent's file watcher.
# The agent watches config.toml and theme files; on change it creates this
# shell's .reload flag and rings SIGURG. The doorbell handler (core/ipc.fish)
# loads fresh theme variables and calls force-repaint directly.
# This ensures a single, unified hot-reload path for all changes
# (config, theme, git, clock all use the same signal-based mechanism).

# Segment loading function
function __gpy_load_segments --argument-names prompt_dir
    # Take all remaining arguments as the list of segments
    set -l enabled_segments $argv[2..-1]
    for segment in $enabled_segments
        set -l segment_file $prompt_dir/../segments/$segment.fish
        if test -f $segment_file
            source $segment_file
        else
            set -l token (string lower (string replace -a '-' '_' -- $segment))
            set -l plugin_file_var "__gpy_plugin_segment_file_$token"
            if set -q $plugin_file_var
                set -l plugin_file $$plugin_file_var
                if test -f $plugin_file
                    source $plugin_file
                else
                    __gpy_log_warn init "Plugin segment '$segment' missing file at $plugin_file"
                end
            else
                __gpy_log_warn init "Segment '$segment' not found at $segment_file"
            end
        end
    end
end

# Lazily sources segment implementation files and runs one-time init for any
# segment newly present in __enabled_segments (e.g. added by a theme switch)
# that was not loaded at shell init. Segment files are only sourced once at
# init (__gpy_load_segments above); fish_prompt silently skips any segment
# whose detect function doesn't exist, so a theme/config reload that enables
# a segment the shell didn't start with must load it here. Called only from
# the agent reload path (__gpy_apply_agent_reload) and __gpy_reload_theme — never the per-prompt hot path.
function __gpy_ensure_segments_loaded --description 'source segment files newly added to __enabled_segments'
    for segment in $__enabled_segments
        functions -q segment_{$segment}_detect; and continue

        set -l segment_file $__gpy_core_dir/../segments/$segment.fish
        if test -f $segment_file
            source $segment_file
        else
            set -l token (string lower (string replace -a '-' '_' -- $segment))
            set -l plugin_file_var "__gpy_plugin_segment_file_$token"
            if set -q $plugin_file_var; and test -f $$plugin_file_var
                source $$plugin_file_var
            else
                __gpy_log_warn init "Segment '$segment' not found at $segment_file"
            end
        end

        functions -q segment_{$segment}_init; and segment_{$segment}_init
    end
    return 0
end

# Load theme from agent (TOML-based)
__gpy_load_theme >/dev/null

# Adjust segments based on GPY_SHOW_LANGUAGES
if test "$GPY_SHOW_LANGUAGES" = 0
    set -g __enabled_segments clock duration directory git
end

# Apply segment overrides for testing BEFORE loading segments
if set -q GPY_MINIMAL_SEGMENTS
    set -g __enabled_segments directory
else if set -q GPY_TEST_SEGMENTS
    set -g __enabled_segments (string split ' ' $GPY_TEST_SEGMENTS)
end

# Load all enabled segments
__gpy_load_segments $__prompt_dir $__enabled_segments

# Load developer tools segment if present
set -l devtools_file $__prompt_dir/../segments/devtools.fish
if test -f $devtools_file
    source $devtools_file
end

# Initialize segments that have init functions
for segment in $__enabled_segments
    if functions -q segment_{$segment}_init
        segment_{$segment}_init
    end
end

# Load advanced configuration and debugging (optional)
set -l advanced_files advanced_config.fish debug.fish
for file in $advanced_files
    set -l path $__prompt_dir/$file
    if test -f $path
        source $path
    end
end

# Default toggles are now set earlier to allow early exit logic to work

# Register helpful aliases
function prompt-config --wraps prompt_config_segments --description 'configure GPY segments'
    prompt_config_segments $argv
end

function prompt-theme --wraps prompt_config_theme --description 'configure GPY theme'
    prompt_config_theme $argv
end

function prompt-debug --wraps prompt_debug --description 'GPY debug helpers'
    prompt_debug $argv
end

function prompt-perf --wraps prompt_config_performance --description 'configure GPY performance'
    prompt_config_performance $argv
end

# Internal helper for programmatic theme reload (used by tests and tooling)
# Arguments: $argv[1] = 1 for verbose output, 0 for silent
function __gpy_reload_theme --description 'reload GPY theme from config (internal helper)'
    set -l verbose (test (count $argv) -gt 0; and test "$argv[1]" = "1"; and echo 1; or echo 0)

    if not __gpy_apply_theme_export
        # Only claim "command not found" when that's actually why: a cache
        # file that failed to source, or an agent binary that spawned but
        # exited non-zero, is a different failure and must not be
        # misreported as a missing binary (mirrors __gpy_load_theme's same
        # distinction above).
        set -l cache_path (__gpy_theme_export_cache_path)
        if test -z "$cache_path" -o ! -f "$cache_path"
            set -l agent_binary (__gpy_resolve_agent_binary)
            if test -z "$agent_binary"
                test $verbose -eq 1; and echo "Error: gpy-agent command not found" >&2
                return 1
            end
        end
        test $verbose -eq 1; and echo "Error: failed to reload theme export" >&2
        return 1
    end

    # The export erases status icons the theme leaves unset; refill the
    # nerd/ascii fallbacks from the current __prompt_icons (#791).
    __gpy_initialize_icons

    # Load implementation files for any segment newly added to __enabled_segments
    __gpy_ensure_segments_loaded

    # Clear language detection caches to force re-detection
    for var in (set -n | string match '__gpy_lang_cache_*')
        set -e $var
    end

    # Clear character/directory render caches (#343): re-exporting the SAME
    # theme name after its content changed (e.g. editing the active theme's
    # colors in place, then running prompt-reload) must not leave a stale
    # cached render behind, even though the cache key -- which is keyed on
    # theme NAME, not content -- would otherwise still match (#576: this
    # clear was missing here even though the agent reload path already had
    # it, so a manual reload silently under-invalidated compared to the
    # automatic one).
    set -e __gpy_char_cache_key
    set -e __gpy_char_cache_val
    set -e __gpy_dir_cache_key
    set -e __gpy_dir_cache_val

    # Trigger repaint if in interactive mode
    if status is-interactive
        commandline -f force-repaint 2>/dev/null
    end

    test $verbose -eq 1; and echo "Theme reloaded successfully"
    return 0
end

# Manual reload helper for debugging or when agent is disabled
function prompt-reload --description 'manually reload GPY theme and config (for debugging)'
    if __gpy_reload_theme 1
        echo "Note: For automatic hot-reload, ensure the agent is running with 'gpy-agent start'"
    else
        echo "Error: Failed to reload theme. Please ensure GPY is installed correctly."
        return 1
    end
end

# --- Signal Handling for Live Repainting ---
# The SIGURG doorbell handler is defined in core/ipc.fish

# Initialize configuration - use defaults to avoid hanging during startup
# Agent communication is handled lazily when needed, not during initialization
set -g __gpy_config_initialized 1

# Load default configuration immediately to ensure prompt works
__gpy_load_default_config

# Fill in any icon the theme export did not provide (e.g. the default theme
# omits `__icon_status_*`). Runs after default config so `__prompt_icons` is set,
# and after theme load so theme-exported icons take precedence.
__gpy_initialize_icons

# NOTE: fish_prompt is defined in functions/fish_prompt.fish
# Do not define it here to avoid conflicts

# Cache root check once — eliminates per-prompt `id -u` fork
set -g __gpy_is_root 0
if test (id -u) -eq 0
    set -g __gpy_is_root 1
end

# Cache sudo-session detection once — pure env-var check, no fork. sudo exports
# SUDO_USER (the invoking user) into the elevated shell's environment; a
# non-empty value means this shell was entered via sudo. Feeds the username
# segment's root/sudo visibility gate (#252).
set -g __gpy_is_sudo 0
if test -n "$SUDO_USER"
    set -g __gpy_is_sudo 1
end

# Cache SSH-session detection once — pure env-var checks, no fork.
# Non-empty checks (not `set -q`) to match bash/zsh, which use `-n`: a
# defined-but-empty SSH_* var does not count as an SSH session.
set -g __gpy_is_ssh 0
if test -n "$SSH_CONNECTION"; or test -n "$SSH_CLIENT"; or test -n "$SSH_TTY"
    set -g __gpy_is_ssh 1
end

# Ensure successful exit
true
