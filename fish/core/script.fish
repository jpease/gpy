# SPDX-License-Identifier: GPL-3.0-or-later
# Script ("G") segments: plugin segments whose text comes from running a
# `command` script (docs/dev/plugins.md "Script Segments"). Theme export
# writes what is needed per segment (`__gpy_script_*`, contract in
# gpy-agent/src/theme/export/script_segments.rs); nothing here asks the agent.
#
# Placement. Scripts run from the fish_prompt *event*, once per command, and
# never from the fish_prompt *function*: Fish calls the function again on
# every repaint (the SIGURG doorbell, `commandline -f repaint`,
# --final-rendering), and a Ctrl-C during a run inside it leaves no prompt
# until the next Enter. The event is emitted once per new prompt, before the
# function. A run is due on the first prompt, after a command (fish_postexec,
# never emitted for an empty Enter) and when PWD changed without one (a key
# binding that cds). Each segment's _detect and _render only read the stored
# text, so a repaint runs nothing.
#
# Runner. Fish has no subshell, sub-second timer or read timeout, so the
# runner is bash/core/g_runner.bash (`gpy setup` installs it as
# core/g_runner.bash; a checkout uses the repository copy), run in one Bash
# child per prompt: the first Bash 4 or later on PATH, else
# /bin/bash, resolved once. Command substitution returns when that direct
# child exits, so a leftover holding the pipe cannot hold the prompt; Fish
# starts no background job, so $last_pid and the job table never change. The
# child caps, strips and checks the output (4096 bytes, C0/DEL stripped, C1
# hides) before Fish holds it; Fish prints it with printf '%s' and needs no
# escape. Declared values reach the child, and from there the script, through
# the environment only (`__gpy_g_v<k>_<NAME>`), never argv.
#
# State lives in shell variables only (no files): `__gpy_g_*`, erased at load
# and assigned before any read, so a value inherited from the parent
# environment is never used.

function __gpy_g_reset_namespace --description 'Erase inherited __gpy_g_* and __gpy_script_* globals'
    for name in (set -n | string match -- '__gpy_g_*') (set -n | string match -- '__gpy_script_*')
        set -e -g $name
    end
end
__gpy_g_reset_namespace

set -g __gpy_g_tokens
set -g __gpy_g_due 1
set -g __gpy_g_pwd ""
set -g __gpy_g_ran 0
set -g __gpy_g_runner_state unresolved
# Consecutive timeouts or over-cap outputs that disable a segment for this shell.
set -g __gpy_g_strikes_max 2
set -g __gpy_g_bash ""
set -g __gpy_g_src ""

# Erase every per-segment state variable of one token and assign defaults.
function __gpy_g_reset_token --argument-names tok
    set -g __gpy_g_t_{$tok}_text ""
    set -g __gpy_g_t_{$tok}_variant ""
    set -g __gpy_g_t_{$tok}_ckey ""
    set -g __gpy_g_t_{$tok}_ctext ""
    set -g __gpy_g_t_{$tok}_cvariant ""
    set -g __gpy_g_t_{$tok}_at 0
    set -g __gpy_g_t_{$tok}_runfails 0
    set -g __gpy_g_t_{$tok}_scanfails 0
    set -g __gpy_g_t_{$tok}_off 0
end

# Before a theme (re)load: erase the style variables of the current plugin
# segments, so a field the next export drops does not linger.
function __gpy_g_clear_styles
    for tok in $__gpy_g_tokens
        for name in (set -n | string match -- "__gpy_segment_$tok"_'*') (set -n | string match -- "__color_$tok"_'*') (set -n | string match -- "__icon_$tok"_'*')
            set -e -g $name
        end
        set -e -g __icon_$tok
    end
end

# After every theme export source (load and reload doorbell): adopt the new
# token list, reset per-segment state of tokens no longer listed, reset every
# circuit breaker and make the next prompt due. A changed `key` is a cache
# miss by itself (the stored key no longer matches).
function __gpy_g_sync --description 'Adopt __gpy_script_segments after a theme export'
    set -l tokens
    for tok in $__gpy_script_segments
        string match -qr '\A[a-z0-9_]+\z' -- $tok; or continue
        contains -- $tok $tokens; and continue
        set -a tokens $tok
    end
    for tok in $__gpy_g_tokens
        contains -- $tok $tokens; or __gpy_g_reset_token $tok
    end
    for tok in $tokens
        contains -- $tok $__gpy_g_tokens; or __gpy_g_reset_token $tok
        set -g __gpy_g_t_{$tok}_runfails 0
        set -g __gpy_g_t_{$tok}_scanfails 0
        set -g __gpy_g_t_{$tok}_off 0
    end
    set -g __gpy_g_tokens $tokens
    set -g __gpy_g_due 1
end

# Define segment_<name>_detect/_render for a listed script segment. Called
# by the segment loader after the built-in and `.fish` plugin lookups.
function __gpy_g_define_segment --argument-names segment
    string match -qr '\A[a-z0-9_-]+\z' -- $segment; or return 1
    set -l tok (string replace -a -- - _ $segment)
    contains -- $tok $__gpy_g_tokens; or return 1
    set -l name_var __gpy_script_{$tok}_name
    test "$$name_var" = "$segment"; or return 1
    function segment_{$segment}_detect --inherit-variable tok
        set -l text_var __gpy_g_t_{$tok}_text
        test -n "$$text_var"
    end
    function segment_{$segment}_render --inherit-variable tok
        __gpy_g_render $tok $argv
    end
    return 0
end

# Pill from the stored text and the exported styles: `__color_<tok>_{bg,fg}`,
# the variant's `__color_<tok>_<variant>_{bg,fg}`, `__icon_<tok>` (or the
# variant's non-empty `__icon_<tok>_<variant>`) and the optional
# `__gpy_segment_<tok>_{open,close}_*` caps. tok and variant were validated
# when stored, so every name below is an identifier.
function __gpy_g_render --argument-names tok is_last is_first
    set -l v __gpy_g_t_{$tok}_text
    set -l text $$v
    set v __gpy_g_t_{$tok}_variant
    set -l variant $$v
    set v __color_{$tok}_bg
    set -l bg "$$v"
    set v __color_{$tok}_fg
    set -l fg "$$v"
    set v __icon_{$tok}
    set -l icon "$$v"
    if test -n "$variant"
        set v __color_{$tok}_{$variant}_bg
        set -q $v; and set bg "$$v"
        set v __color_{$tok}_{$variant}_fg
        set -q $v; and set fg "$$v"
        set v __icon_{$tok}_{$variant}
        test -n "$$v"; and set icon "$$v"
    end
    set -l label " $text"
    test -n "$icon"; and set label " $icon $text"

    __gpy_section_pill "$bg" "$fg" "$label" "$is_last" __gpy_segment_$tok
    set -g __gpy_last_segment_bg $bg
end

# fish_postexec is emitted for a command, never for an empty Enter.
function __gpy_g_on_postexec --on-event fish_postexec
    set -g __gpy_g_due 1
    set -g __gpy_g_ran 1
end

function __gpy_g_on_prompt --on-event fish_prompt
    set -q __gpy_g_tokens[1]; or return 0
    if test "$PWD" != "$__gpy_g_pwd"
        set -g __gpy_g_due 1
        set -g __gpy_g_pwd $PWD
    end
    test "$__gpy_g_due" = 1; or return 0
    # Cleared before the run, so an interrupted run is not started again.
    set -g __gpy_g_due 0
    __gpy_g_run_due
end

# Resolve the runner child and its source once per shell. The child is the
# first Bash 4 or later found on PATH (Ctrl-C ends its runs at once), else
# /bin/bash: Bash 3.2 ignores SIGINT inside `<( )` and runs traps only after
# `read -t` returns, so there Ctrl-C waits for the timeout (the run still
# ends and its group is killed).
function __gpy_g_resolve_runner
    test "$__gpy_g_runner_state" = unresolved; or return 0
    set -g __gpy_g_runner_state none
    # The installed tree holds a copy; a checkout, a release staging dir and
    # Homebrew share/gpy hold the Bash tree beside this one. `path normalize`
    # resolves `..` lexically, so a symlinked tree still finds its sibling.
    set -g __gpy_g_runner_file $__gpy_core_dir/g_runner.bash
    if not test -r $__gpy_g_runner_file
        set -l sibling (path normalize $__gpy_core_dir/../../bash/core/g_runner.bash)
        test -r $sibling; and set -g __gpy_g_runner_file $sibling
    end
    if not test -r $__gpy_g_runner_file
        __gpy_log_warn script "Runner $__gpy_g_runner_file missing - script segments disabled"
        return 1
    end
    set -l bash ""
    for candidate in (command -sa bash)
        string match -q '/*' -- $candidate; or continue
        test "$candidate" = /bin/bash; and continue
        set -l major (command $candidate -p -c 'printf %s "${BASH_VERSINFO[0]}"' </dev/null 2>/dev/null)
        if string match -qr '\A[0-9]+\z' -- "$major"; and test $major -ge 4
            set bash $candidate
            break
        end
    end
    test -z "$bash"; and test -x /bin/bash; and set bash /bin/bash
    if test -z "$bash"
        __gpy_log_warn script "No bash found - script segments disabled"
        return 1
    end
    set -g __gpy_g_src (string collect < $__gpy_g_runner_file)
    set -g __gpy_g_bash $bash
    set -g __gpy_g_runner_state ready
end

# One timeout or over-cap result (kind: run or scan) for token tok; each kind
# keeps its own count (a completed scan resets the scan count, a successful
# run the run count) and the second in a row disables the segment for this
# shell, with one line on stderr.
function __gpy_g_trip --argument-names tok kind
    set -l v __gpy_g_t_{$tok}_{$kind}fails
    set -l trips $$v
    string match -qr '\A[0-9]+\z' -- "$trips"; or set trips 0
    set trips (math $trips + 1)
    set -g $v $trips
    test $trips -ge $__gpy_g_strikes_max; or return 0
    set v __gpy_g_t_{$tok}_off
    test "$$v" = 1; and return 0
    set -g __gpy_g_t_{$tok}_off 1
    set v __gpy_script_{$tok}_name
    printf 'gpy: script segment %s disabled for this shell after %s failures (timeout or output over 4096 bytes)\n' "$$v" $__gpy_g_strikes_max >&2
end

# A failed trigger drops the segment's cache entry, so a later prompt whose
# trigger holds again runs the command instead of reusing stale output.
function __gpy_g_uncache --argument-names tok
    set -g __gpy_g_t_{$tok}_ckey ""
    set -g __gpy_g_t_{$tok}_ctext ""
    set -g __gpy_g_t_{$tok}_cvariant ""
    set -g __gpy_g_t_{$tok}_at 0
end

# Run every due script segment: require_env, then the file scan, then the
# cache, then the batch (require_command and command) under one deadline.
function __gpy_g_run_due
    # Emptied first: a run cut short hides its segments.
    for tok in $__gpy_g_tokens
        set -g __gpy_g_t_{$tok}_text ""
        set -g __gpy_g_t_{$tok}_variant ""
    end
    __gpy_g_resolve_runner
    test "$__gpy_g_runner_state" = ready; or return 0
    string match -q '/*' -- "$__gpy_script_sh"; or return 0

    set -l now (__gpy_now_seconds)
    string match -qr '\A[0-9]+\z' -- "$now"; or set now 0
    set -l scan_args
    set -l scan_toks
    set -l pass_toks

    for tok in $__gpy_g_tokens
        set -l v __gpy_g_t_{$tok}_off
        test "$$v" = 1; and continue

        set v __gpy_script_{$tok}_command
        set -l cmd "$$v"
        set v __gpy_script_{$tok}_reqcommand
        set -l req "$$v"
        set v __gpy_script_{$tok}_root
        set -l root "$$v"
        set v __gpy_script_{$tok}_ttl
        set -l ttl "$$v"
        set v __gpy_script_{$tok}_timeout
        set -l timeout "$$v"
        set v __gpy_script_{$tok}_scope
        set -l scope "$$v"
        string match -q '/*' -- "$cmd"; or continue
        string match -q '/*' -- "$root"; or continue
        test -z "$req"; or string match -q '/*' -- "$req"; or continue
        string match -qr '\A[0-9]{1,8}\z' -- "$ttl"; or continue
        string match -qr '\A[0-9]{1,3}\z' -- "$timeout"; or continue
        test $timeout -ge 1 -a $timeout -le 500; or continue

        # require_env: set and non-empty, fork-free.
        set -l ok 1
        set v __gpy_script_{$tok}_reqenv
        for name in $$v
            if not string match -qr '\A[A-Za-z_][A-Za-z0-9_]*\z' -- $name; or not set -q $name; or test -z "$$name"
                set ok 0
                break
            end
        end
        test $ok = 1; or begin
            __gpy_g_uncache $tok
            continue
        end

        # Inputs, declared env and settings: the names go to the runner in
        # argv, the values in the environment; the values also key the cache.
        set -l names
        set -l vals
        set -l dkey
        set v __gpy_script_{$tok}_inputs
        for word in $$v
            set -l name
            set -l val
            switch $word
                case pwd
                    set name GPY_PWD
                    set val "$PWD"
                case is_root
                    set name GPY_IS_ROOT
                    set val "$__gpy_is_root"
                case is_sudo
                    set name GPY_IS_SUDO
                    set val "$__gpy_is_sudo"
                case is_ssh
                    set name GPY_IS_SSH
                    set val "$__gpy_is_ssh"
                case cmd_duration
                    set name GPY_CMD_DURATION
                    set val ""
                    test "$__gpy_g_ran" = 1; and set val "$CMD_DURATION"
                case prompt_icons
                    set name GPY_PROMPT_ICONS
                    set val "$__prompt_icons"
                case '*'
                    set ok 0
                    break
            end
            set -a names $name
            set -a vals "$val"
            set dkey "$dkey"(string length -- "$val")":$val"
        end
        set v __gpy_script_{$tok}_env
        for name in $$v
            if not string match -qr '\A[A-Za-z_][A-Za-z0-9_]*\z' -- $name; or string match -q 'GPY_*' -- $name
                set ok 0
                break
            end
            if set -q $name
                # Quoted: one string, joined as Fish exports it.
                set -l val "$$name"
                set -a names $name
                set -a vals "$val"
                set dkey "$dkey"s(string length -- "$val")":$val"
            else
                set dkey "$dkey"u
            end
        end
        set v __gpy_script_{$tok}_opts
        for entry in $$v
            set -l kv (string split -m1 = -- "$entry")
            if test (count $kv) -ne 2; or not string match -qr '\AGPY_OPT_[a-z_][a-z0-9_]{0,63}\z' -- $kv[1]
                set ok 0
                break
            end
            set -a names $kv[1]
            set -a vals "$kv[2]"
        end
        test $ok = 1; or continue

        set v __gpy_script_{$tok}_key
        set -l key "$$v"\x1f"$dkey"
        set -l hit 0
        if test $ttl -gt 0
            set v __gpy_g_t_{$tok}_ckey
            set -l ckey "$$v"
            set v __gpy_g_t_{$tok}_at
            set -l at "$$v"
            if test -n "$ckey"; and test "$ckey" = "$key"; and string match -qr '\A[0-9]+\z' -- "$at"
                test (math "$now - $at") -lt $ttl; and set hit 1
            end
        end

        # Working directory: $PWD for a folder-scoped segment (the child is
        # started there, so "" keeps it out of argv), else the plugin root.
        set -l cwd "$root"
        test "$scope" = folder; and set cwd ""

        set -f __gpy_g_r_{$tok}_names $names
        set -f __gpy_g_r_{$tok}_vals $vals
        set -f __gpy_g_r_{$tok}_key "$key"
        set -f __gpy_g_r_{$tok}_hit $hit
        set -f __gpy_g_r_{$tok}_cwd "$cwd"
        set -f __gpy_g_r_{$tok}_cmd "$cmd"
        set -f __gpy_g_r_{$tok}_req "$req"
        set -f __gpy_g_r_{$tok}_timeout $timeout
        set -f __gpy_g_r_{$tok}_ttl $ttl

        set -l rf __gpy_script_{$tok}_reqfiles
        set -l rd __gpy_script_{$tok}_reqfolders
        set -l re __gpy_script_{$tok}_reqexts
        if test (count $$rf $$rd $$re) -gt 0
            set -a scan_toks $tok
            set -a scan_args $tok (count $$rf) (count $$rd) (count $$re) $$rf $$rd $$re
        else
            set -a pass_toks $tok
        end
    end

    # While a child runs a SIGINT handler exists, so Ctrl-C does not cancel
    # this handler; the child gets the INT too, kills its groups and reports.
    function __gpy_g_sigint --on-signal SIGINT
    end

    # The file scan: one bounded child for every segment that needs it.
    if set -q scan_toks[1]
        set -l res (command $__gpy_g_bash -p -c "$__gpy_g_src"\n'__gpy_g_scan "$@"' gpy $scan_args 2>/dev/null)
        for rec in (string split -n \x1e -- $res)
            set -l f (string split \x1f -- $rec)
            contains -- "$f[1]" $scan_toks; or continue
            set -l tok $f[1]
            switch "$f[2]"
                case hit
                    set -g __gpy_g_t_{$tok}_scanfails 0
                    set -a pass_toks $tok
                case miss
                    set -g __gpy_g_t_{$tok}_scanfails 0
                case timeout
                    __gpy_g_trip $tok scan
            end
        end
        for tok in $scan_toks
            contains -- $tok $pass_toks; or __gpy_g_uncache $tok
        end
    end

    # Cache hits show their stored text; the rest run in one batch (the
    # runner reads at most 40 segments per call; more stay hidden).
    set -l batch_args
    set -l batch_toks
    set -l k 0
    for tok in $__gpy_g_tokens
        contains -- $tok $pass_toks; or continue
        set -l v __gpy_g_r_{$tok}_hit
        if test "$$v" = 1
            set v __gpy_g_t_{$tok}_ctext
            set -g __gpy_g_t_{$tok}_text "$$v"
            set v __gpy_g_t_{$tok}_cvariant
            set -g __gpy_g_t_{$tok}_variant "$$v"
            continue
        end
        test $k -lt 40; or continue
        set -l nv __gpy_g_r_{$tok}_names
        set -l vv __gpy_g_r_{$tok}_vals
        set -l i 1
        for name in $$nv
            set -fx __gpy_g_v{$k}_$name "$$vv[1][$i]"
            set i (math $i + 1)
        end
        set -l cwd_v __gpy_g_r_{$tok}_cwd
        set -l cmd_v __gpy_g_r_{$tok}_cmd
        set -l req_v __gpy_g_r_{$tok}_req
        set -l t_v __gpy_g_r_{$tok}_timeout
        set -a batch_args $tok $$t_v "$$cwd_v" "$$cmd_v" "$$req_v" (count $$nv) $$nv
        set -a batch_toks $tok
        set k (math $k + 1)
    end

    if set -q batch_toks[1]
        # stdin stays the terminal: Fish then makes the child the foreground
        # group, so Ctrl-C reaches it (scripts themselves get /dev/null).
        set -l res (command $__gpy_g_bash -p -c "$__gpy_g_src"\n'__gpy_g_batch "$@"' gpy "$__gpy_script_sh" $batch_args 2>/dev/null)
        for rec in (string split -n \x1e -- $res)
            set -l f (string split \x1f -- $rec)
            contains -- "$f[1]" $batch_toks; or continue
            set -l tok $f[1]
            set -l text ""
            set -l variant ""
            switch "$f[2]"
                case ok
                    set -g __gpy_g_t_{$tok}_runfails 0
                    set text "$f[3]"
                    string match -qr '\A[a-z0-9_]{1,32}\z' -- "$f[4]"; and set variant $f[4]
                case exit req c1
                case timeout cap
                    __gpy_g_trip $tok run
                    continue
                case '*'
                    # intr, or a malformed record: hidden, not cached.
                    continue
            end
            set -g __gpy_g_t_{$tok}_text "$text"
            set -g __gpy_g_t_{$tok}_variant "$variant"
            set -l ttl_v __gpy_g_r_{$tok}_ttl
            if test "$$ttl_v" -gt 0
                set -l key_v __gpy_g_r_{$tok}_key
                set -g __gpy_g_t_{$tok}_ckey "$$key_v"
                set -g __gpy_g_t_{$tok}_ctext "$text"
                set -g __gpy_g_t_{$tok}_cvariant "$variant"
                set -g __gpy_g_t_{$tok}_at $now
            end
        end
    end

    functions -e __gpy_g_sigint
    return 0
end
