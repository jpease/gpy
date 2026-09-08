#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# E2E Test: real interactive Fish sessions repaint by themselves (#645)
# ============================================================================
#
# The product's defining behaviour is that the prompt updates with no
# keystroke when the repository changes: watcher -> SIGUSR1 ->
# __gpy_sigusr1_handler -> __gpy_repaint_trigger -> `commandline -f
# force-repaint` -> fish_prompt. `commandline -f force-repaint` is a no-op
# outside interactive mode, so every `fish -c` based test only ever counted
# a signal. These scenarios drive a real `fish -i` on a pseudo-terminal
# (tests/lib/pty_session.py) against a real agent and real repository and
# read what the terminal would show:
#
#   1. idle repaint: a tracked-file edit made from outside the shell yields a
#      new prompt render carrying the dirty marker, then `touch` yields the
#      untracked marker, then `exit` unregisters the client;
#   2. two shells, one agent: a commit typed in shell A repaints shell B, the
#      agent reports two registered clients, and both renders agree;
#   3. SIGUSR2 on a Fish client: a config.toml edit (theme switch, a new
#      segment, icons off) repaints with the new theme and segments with no
#      keypress.
#
# All waits are bounded polls on the transcript; the transcript tail is
# printed on failure. Skips (via test_skip) when python3 is missing: exit 0
# locally, exit 1 under CI (#650).
#
# Expected runtime: ~20 seconds

source (dirname (status -f))/../lib/test_helpers.fish

# The git branch glyph (U+E0A0), built from bytes so no editor or transport
# can silently drop the private-use character from this file.
set -g BRANCH_GLYPH (printf '\xee\x82\xa0')

set -g __sess_pass 0
set -g __sess_fail 0
set -g __pty $__gpy_root/tests/lib/pty_session.py
set -g __sessions

function check --argument-names label ok detail
    if test "$ok" = 1
        set -g __sess_pass (math $__sess_pass + 1)
        print_test_result "$label" PASS
    else
        set -g __sess_fail (math $__sess_fail + 1)
        print_test_result "$label" FAIL "$detail"
    end
end

# Every terminal escape (SGR, cursor movement, mode toggles, OSC titles) and
# the default theme's powerline chevrons (U+E0B0..U+E0BF), so comparisons
# see the text a person reads.
function strip_escapes
    cat | string replace -ra "[\x{e0b0}-\x{e0bf}]" '' | string replace -ra '\e\][^\a\e]*(\a|\e\\\\)' '' | string replace -ra '\eP[^\e]*\e\\\\' '' | string replace -ra '\e\[[0-9;?>=<]*[A-Za-z]' '' | string replace -ra '\e[()=>][A-Za-z0-9]?' '' | string replace -a \r ''
end

# Transcript bytes written after $offset, escapes stripped.
function transcript_after --argument-names dir offset
    # The driver's offsets are bytes (`string sub` counts characters), so
    # take the byte tail with tail(1).
    tail -c +(math $offset + 1) $dir/transcript | strip_escapes | string collect
end

function session_start --argument-names name cwd
    set -l dir $__gpy_test_tmp_dir/session-$name
    python3 $__pty start $dir -- fish --no-config -i -C "
        source $__gpy_root/fish/core/init.fish
        source $__gpy_root/fish/functions/fish_prompt.fish
        cd '$cwd'
    "
    set -ga __sessions $dir
    echo $dir
end

function session_send --argument-names dir text
    python3 $__pty send $dir "$text"
end

function session_size --argument-names dir
    python3 $__pty size $dir
end

# Wait until the transcript after $offset contains $needle (plain text,
# escapes stripped). Returns 0 and echoes the new transcript size on success.
function session_wait --argument-names dir needle timeout offset
    set -l attempts (math "ceil($timeout / 0.1)")
    for i in (seq 1 $attempts)
        if string match -q "*$needle*" -- (transcript_after $dir $offset)
            session_size $dir
            return 0
        end
        sleep 0.1
    end
    echo "--- transcript tail ($dir) ---" >&2
    tail -c 1500 $dir/transcript | strip_escapes >&2
    echo >&2
    return 1
end

# Whether the raw transcript after $offset carries any powerline chevron
# (U+E0B0..U+E0BF): the default theme's signature, absent from `text`.
function transcript_has_chevrons --argument-names dir offset
    tail -c +(math $offset + 1) $dir/transcript | cat | string match -qr "[\x{e0b0}-\x{e0bf}]"
end

# The most recent prompt segment line (the line carrying the branch name)
# in a session's transcript, trimmed.
function latest_segment_line --argument-names dir
    transcript_after $dir 0 | string split \n | string match -e ' main' | tail -1 | string trim
end

function session_stop_all
    for dir in $__sessions
        python3 $__pty stop $dir >/dev/null 2>&1
    end
    set -g __sessions
end

function registered_clients
    gpy-agent status 2>/dev/null | string match -r 'Registered Clients: (\d+)' | tail -1
end

function __clients_are --argument-names n
    test (registered_clients) = $n
end

# Write the config and, with the agent up, have it reload synchronously and
# wait until the theme export it publishes names the configured theme. The
# reload is what a running agent does on its own after the watcher's
# debounce; asking for it directly keeps the test deterministic without
# changing what the shells then see.
function write_config
    mkdir -p $XDG_CONFIG_HOME/gpy
    printf '%s\n' '[ui]' $argv >$XDG_CONFIG_HOME/gpy/config.toml
    if test -S "$GPY_AGENT_SOCKET_PATH"
        gpy-agent config reload >/dev/null 2>&1
        set -l theme (string match -r 'theme = "([^"]+)"' -- $argv)[2]
        function __export_names_theme --inherit-variable theme
            grep -q "__gpy_theme_name \"$theme\"" $XDG_CACHE_HOME/gpy/theme-export.fish 2>/dev/null
        end
        poll_until 10 __export_names_theme
    end
end

function setup_repo
    set -l repo $__gpy_test_tmp_dir/repo
    mkdir -p $repo
    git -C $repo init -q -b main
    git -C $repo config user.email test@example.com
    git -C $repo config user.name Test
    git -C $repo config core.fsmonitor false
    # Commits typed into a pty must never block on the developer's own
    # signing or hook setup.
    git -C $repo config commit.gpgsign false
    git -C $repo config core.hooksPath /dev/null
    echo hello >$repo/tracked.txt
    git -C $repo add tracked.txt
    git -C $repo commit -qm init
    echo $repo
end

# ---------------------------------------------------------------------------

function scenario_idle_repaint --argument-names repo
    print_test_header "Scenario 1: idle repaint on a working-tree edit"

    set -l a (session_start a $repo)
    set -l off (session_wait $a "❯" 10 0)
    if test -z "$off"
        check "first prompt rendered" 0 "no prompt character within 10 s"
        return 1
    end
    check "first prompt rendered" 1
    set -l first (transcript_after $a 0)
    if string match -q "*$BRANCH_GLYPH main*" -- "$first"; and string match -q "* repo *" -- "$first"
        check "first prompt shows the branch (with its glyph) and the directory" 1
    else
        check "first prompt shows the branch (with its glyph) and the directory" 0 "$first"
    end

    if not poll_until 5 __clients_are 1
        check "session registered with the agent" 0 (registered_clients)
        return 1
    end
    check "session registered with the agent" 1

    # No keystroke from here on: the edit comes from this test process.
    echo dirty >>$repo/tracked.txt
    set -l off2 (session_wait $a "✱1" 5 $off)
    if test -n "$off2"
        check "prompt repainted with the dirty marker, no keystroke sent" 1
    else
        check "prompt repainted with the dirty marker, no keystroke sent" 0 "no ✱1 render within 5 s"
        return 1
    end

    session_send $a 'touch untracked.txt\r'
    set -l off3 (session_wait $a "?1" 5 $off2)
    if test -n "$off3"
        check "next prompt after touch shows the untracked marker" 1
    else
        check "next prompt after touch shows the untracked marker" 0 "no ?1 render within 5 s"
    end

    session_send $a 'exit\r'
    if poll_until 5 __clients_are 0
        check "exit unregisters the client" 1
    else
        check "exit unregisters the client" 0 "still (registered_clients) clients"
    end
    session_stop_all
    return 0
end

function scenario_two_shells --argument-names repo
    print_test_header "Scenario 2: two shells, one agent"

    set -l a (session_start a $repo)
    set -l b (session_start b $repo)
    set -l off_a (session_wait $a "❯" 10 0)
    set -l off_b (session_wait $b "❯" 10 0)
    if test -z "$off_a" -o -z "$off_b"
        check "both sessions rendered a prompt" 0
        return 1
    end
    check "both sessions rendered a prompt" 1

    if poll_until 5 __clients_are 2
        check "agent reports two registered clients" 1
    else
        check "agent reports two registered clients" 0 (registered_clients)
    end

    # The tree is dirty from scenario 1; commit it from shell A only. A's
    # own next prompt is the clean render; B typed nothing and must repaint
    # to the same clean state within 5 s of A's prompt.
    session_send $a 'git add -A; git commit -qm x\r'
    function __a_clean --inherit-variable a
        string match -qr 'main\s*$' -- (latest_segment_line $a)
    end
    if not poll_until 10 __a_clean
        check "shell A renders a clean prompt after its commit" 0 (latest_segment_line $a)
        return 1
    end
    check "shell A renders a clean prompt after its commit" 1
    function __b_clean --inherit-variable b
        string match -qr 'main\s*$' -- (latest_segment_line $b)
    end
    if poll_until 5 __b_clean
        check "shell B repainted clean after A's commit, no keystroke sent" 1
    else
        check "shell B repainted clean after A's commit, no keystroke sent" 0 (latest_segment_line $b)
    end

    # Both shells' latest segment line (the one naming the branch) is the
    # same text.
    set -l line_a (latest_segment_line $a)
    set -l line_b (latest_segment_line $b)
    if test -n "$line_a"; and test "$line_a" = "$line_b"
        check "both shells render the same prompt" 1
    else
        check "both shells render the same prompt" 0 "A: '$line_a' B: '$line_b'"
        echo "--- A tail:"
        transcript_after $a 0 | tail -8
        echo "--- B tail:"
        transcript_after $b 0 | tail -8
    end

    session_send $a 'exit\r'
    session_send $b 'exit\r'
    poll_until 5 __clients_are 0
    session_stop_all
    return 0
end

function scenario_sigusr2 --argument-names repo
    print_test_header "Scenario 3: SIGUSR2 config reload on a Fish client"

    write_config 'show_icons = true' 'theme = "text"' 'enabled_segments = ["directory", "git"]'
    # Instant-cache entries are keyed by path, not theme; drop the renders
    # the earlier scenarios left so this session's first prompt is a fresh
    # text-theme render rather than a stale default-theme one.
    # (Only the entries: the agent creates the directory once at startup and
    # every later write fails if it is gone.)
    rm -f $XDG_CACHE_HOME/gpy/instant-prompts/*
    set -l a (session_start a $repo)
    set -l off (session_wait $a "❯" 10 0)
    if test -z "$off"
        check "first prompt rendered (text theme)" 0
        return 1
    end
    if transcript_has_chevrons $a 0
        check "text theme renders without powerline chevrons" 0 (latest_segment_line $a)
    else
        check "text theme renders without powerline chevrons" 1
    end
    poll_until 5 __clients_are 1

    # Theme switch + a new segment, no keypress: the agent's SIGUSR2 makes
    # the shell re-source the export and repaint, and the default theme
    # frames segments in powerline chevrons the text theme never emits.
    write_config 'show_icons = true' 'theme = "default"' 'enabled_segments = ["directory", "git", "duration"]'
    function __default_theme_line --inherit-variable a --inherit-variable off
        transcript_has_chevrons $a $off
    end
    if poll_until 10 __default_theme_line
        check "repaint after the theme switch uses the default theme, no keypress" 1
    else
        check "repaint after the theme switch uses the default theme, no keypress" 0 (latest_segment_line $a)
        return 1
    end
    set -l off2 (session_size $a)

    session_send $a 'sleep 0.3\r'
    set -l off3 (session_wait $a "s" 10 $off2)
    function __duration_shown --inherit-variable a
        string match -qr '0\.[0-9]+s' -- (latest_segment_line $a)
    end
    if poll_until 10 __duration_shown
        check "duration segment appears after a slow command" 1
    else
        check "duration segment appears after a slow command" 0 (latest_segment_line $a)
    end
    set off3 (session_size $a)

    # Settle on a fresh idle prompt after a fast command (an empty Enter
    # keeps the previous CMD_DURATION, so the duration segment would stay),
    # then toggle icons off: the git branch glyph disappears from the next
    # repaint, with no keypress after the toggle.
    session_send $a 'true\r'
    function __idle_prompt --inherit-variable a
        set -l line (latest_segment_line $a)
        test -n "$line"; and not string match -qr '[0-9]s$' -- "$line"
    end
    if not poll_until 10 __idle_prompt
        check "an idle prompt without the duration segment follows a fast command" 0 (latest_segment_line $a)
        return 1
    end
    write_config 'show_icons = false' 'theme = "default"' 'enabled_segments = ["directory", "git", "duration"]'
    function __no_branch_glyph --inherit-variable a
        set -l line (latest_segment_line $a)
        string match -q "*main*" -- "$line"; and not string match -q "*$BRANCH_GLYPH*" -- "$line"
    end
    if poll_until 10 __no_branch_glyph
        check "show_icons = false drops the branch glyph on the next repaint" 1
    else
        check "show_icons = false drops the branch glyph on the next repaint" 0 (latest_segment_line $a)
    end

    session_send $a 'exit\r'
    poll_until 5 __clients_are 0
    session_stop_all
    return 0
end

# ---------------------------------------------------------------------------

if not command -q python3
    test_skip "python3 is required to drive a pseudo-terminal"
end

init_test_env
set -gx GPY_AGENT_ENABLED 1
# An interactive shell registers with the agent from its on-prompt
# supervisor hook, which also spawns the detached supervisor loop; that loop
# sources the *installed* layout under $XDG_CONFIG_HOME/fish/gpy, so give
# the sandbox one that points at this checkout. The loop is stopped from
# its pidfile at the end so it cannot restart the agent behind cleanup.
mkdir -p $XDG_CONFIG_HOME/fish
ln -s $__gpy_root/fish $XDG_CONFIG_HOME/fish/gpy
function stop_supervisor_loop
    set -l pidfile $XDG_CACHE_HOME/gpy/supervisor.pid
    if test -f $pidfile
        kill (cat $pidfile) 2>/dev/null
        rm -f $pidfile
    end
end
write_config 'show_icons = true' 'theme = "default"' 'enabled_segments = ["directory", "git"]'
set -l repo (setup_repo)

if not start_test_agent
    print_test_result "Agent Start" FAIL "gpy-agent failed to start"
    cleanup_test_files
    exit 1
end
print_test_result "Agent Start" PASS

scenario_idle_repaint $repo
scenario_two_shells $repo
scenario_sigusr2 $repo

session_stop_all
stop_supervisor_loop
cleanup_test_files

echo ""
echo "Passed: $__sess_pass  Failed: $__sess_fail"
test $__sess_fail -eq 0
