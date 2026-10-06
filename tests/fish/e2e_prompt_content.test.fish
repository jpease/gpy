#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# ============================================================================
# E2E Test: the Fish prompt says the right thing (#644)
# ============================================================================
#
# Every other Fish prompt assertion in this suite is "non-empty" or "changed".
# This one renders `fish_prompt` through a real `gpy-agent` from a real
# repository, strips the SGR colour escapes, and asserts the exact tokens the
# git, language and directory segments must carry -- branch, ahead count,
# dirty/untracked/stash/detached/rebase markers, language names and versions,
# every directory display mode -- plus the prompt frame (icons, root) and the
# "no JSON leaks" guarantee that used to live in tests/manual/.
#
# Glyphs are the crate defaults from gpy-agent/src/config/defaults.rs and
# gpy-agent/src/formatter/git_resolver.rs (`↑` ahead, `✱` unstaged,
# `?` untracked, `≡` stash, `➦` detached, `↻` in-progress, `✖` conflicts,
# U+E0A0 branch), rendered by the shipped `default` theme so what is asserted
# is what a default install shows.
#
# Timing model: a render is a fresh non-interactive `fish` whose git/language
# segments read the agent's instant-prompt cache; a cold miss is a bounded
# synchronous query, a hit is served as-is (stale entries refresh in the
# background and repaint later). The agent itself learns about a git change
# through its watcher, within a debounce window. So every assertion polls
# renders with the instant cache cleared (each render is a fresh query) until
# the expected tokens appear, bounded by poll_until's timeout -- never a fixed
# sleep. A wrong render therefore fails at the timeout with the last render
# printed.
#
# Expected runtime: ~15 seconds

source (dirname (status -f))/../lib/test_helpers.fish

# The git branch glyph (U+E0A0), built from bytes so no editor or transport
# can silently drop the private-use character from this file.
set -g BRANCH_GLYPH (printf '\xee\x82\xa0')

set -g __content_pass 0
set -g __content_fail 0
set -g __render_last ""
set -g __show_icons true

function check --argument-names label ok detail
    if test "$ok" = 1
        set -g __content_pass (math $__content_pass + 1)
        print_test_result "$label" PASS
    else
        set -g __content_fail (math $__content_fail + 1)
        print_test_result "$label" FAIL "$detail"
    end
end

# SGR colour escapes and the default theme's powerline chevrons
# (U+E0B0..U+E0BF) both go, leaving the text a person reads. `string` inside
# a function does not read the function's piped stdin on fish 4.9, so the
# bytes are routed through `cat` explicitly.
function strip_sgr
    cat | string replace -ra '\e\[[0-9;]*m' '' | string replace -ra "[\x{e0b0}-\x{e0bf}]" ''
end

# Render one prompt from a fresh non-interactive fish in $cwd, SGR-stripped,
# as one string. A fresh process per render: no memo cache carries a stale
# segment between cases.
#
# The language segment answers a cache miss by sending the agent a
# disowned background request and printing nothing; an interactive shell
# lives on and the reply lands in the cache for its next prompt. This
# one-shot fish would exit before that request is even sent, so it lingers
# briefly after the render, standing in for the shell staying open. This is
# not a synchronisation sleep: assertions still poll renders until the cache
# has what they expect.
function render_prompt --argument-names cwd
    fish --no-config -c '
        source $argv[1]/fish/core/init.fish
        source $argv[1]/fish/functions/fish_prompt.fish
        __gpy_register_with_agent >/dev/null 2>&1
        cd $argv[2]
        fish_prompt
        sleep 0.2
    ' -- $__gpy_root $cwd 2>/dev/null | strip_sgr | string collect
end

# Render just the directory segment via its entry point, trimmed.
function render_directory --argument-names cwd
    fish --no-config -c '
        source $argv[1]/fish/core/init.fish
        source $argv[1]/fish/functions/fish_prompt.fish
        __gpy_register_with_agent >/dev/null 2>&1
        cd $argv[2]
        segment_directory_render true true
    ' -- $__gpy_root $cwd 2>/dev/null | strip_sgr | string collect | string trim
end

# Does $text contain every `|`-separated token of $wanted and none of $unwanted?
function text_matches --argument-names text wanted unwanted
    for token in (string split '|' -- $wanted)
        test -n "$token"; or continue
        string match -q "*$token*" -- "$text"; or return 1
    end
    for token in (string split '|' -- $unwanted)
        test -n "$token"; or continue
        string match -q "*$token*" -- "$text"; and return 1
    end
    return 0
end

# Poll renders of $cwd until the text carries every $wanted token and no
# $unwanted one. The git instant-cache entries are cleared before each render
# so every git render is a real query (a cached hit would be served as-is);
# language entries are kept, because the language segment only ever fills in
# from the cache the agent writes after a first, background request.
# Leaves the last render in $__render_last.
function wait_render --argument-names cwd wanted unwanted
    function __wait_render_probe --inherit-variable cwd --inherit-variable wanted --inherit-variable unwanted
        rm -f $XDG_CACHE_HOME/gpy/instant-prompts/*.git.*
        set -g __render_last (render_prompt $cwd)
        text_matches "$__render_last" "$wanted" "$unwanted"
    end
    poll_until 10 __wait_render_probe
end

function wait_directory --argument-names cwd expected
    function __wait_dir_probe --inherit-variable cwd --inherit-variable expected
        set -g __render_last (render_directory $cwd)
        test "$__render_last" = "$expected"
    end
    poll_until 10 __wait_dir_probe
end

# Assert with polling: every token in $wanted present, none of $unwanted.
function assert_render --argument-names label cwd wanted unwanted
    if wait_render $cwd "$wanted" "$unwanted"
        check "$label" 1
    else
        check "$label" 0 "wanted [$wanted] without [$unwanted]; last render: "(string escape -- "$__render_last")
    end
end

# Write the config and, when the agent is up, make it reload synchronously:
# the file watcher would pick the change up after its debounce, but a render
# issued before that would be served under the previous settings (and the
# language detection it triggers cached that way for hours).
function write_config
    mkdir -p $XDG_CONFIG_HOME/gpy
    printf '%s\n' \
        '[ui]' \
        "show_icons = $__show_icons" \
        'theme = "default"' \
        'enabled_segments = ["directory", "git", "language"]' \
        $argv >$XDG_CONFIG_HOME/gpy/config.toml
    if test -S "$GPY_AGENT_SOCKET_PATH"
        gpy-agent config reload >/dev/null 2>&1
    end
end

function make_repo --argument-names path
    mkdir -p $path
    git -C $path init -q -b main
    git -C $path config user.email test@example.com
    git -C $path config user.name Test
    # See e2e_git_live_content.test.fish: a global core.fsmonitor daemon
    # competes with the agent's watcher for the same events.
    git -C $path config core.fsmonitor false
    # Commits typed into a pty must never block on the developer's own
    # signing or hook setup.
    git -C $path config commit.gpgsign false
    git -C $path config core.hooksPath /dev/null
    echo hello >$path/tracked.txt
    git -C $path add tracked.txt
    git -C $path commit -qm init
end

# ---- 1. git ------------------------------------------------------------------
function test_git_content --argument-names repo
    print_test_header "Git segment content from a real repository"

    git -C $repo checkout -qb feature/x
    git init -q --bare $__gpy_test_tmp_dir/remote.git
    git -C $repo remote add origin $__gpy_test_tmp_dir/remote.git
    git -C $repo push -qu origin feature/x >/dev/null 2>&1
    echo a >>$repo/tracked.txt
    git -C $repo commit -qam a
    echo b >>$repo/tracked.txt
    git -C $repo commit -qam b
    # One stash first (git stash takes every tracked change with it), then
    # the dirty/untracked working tree the render must report.
    echo stashed >$repo/stashed.txt
    git -C $repo add stashed.txt
    git -C $repo stash -q
    echo dirty >>$repo/tracked.txt
    touch $repo/untracked.txt

    assert_render "git: branch, glyph, ahead 2, unstaged 1, untracked 1, stash 1, repo basename" \
        $repo "feature/x|$BRANCH_GLYPH feature/x|↑2|✱1|?1|≡1| repo " "↓"
    set -l render $__render_last
    if string match -q '*{"*' -- "$render"; or string match -q '*"op"*' -- "$render"
        check "git: no JSON leaks into the prompt" 0 "$render"
    else
        check "git: no JSON leaks into the prompt" 1
    end

    # Detached HEAD: the branch glyph becomes the detached marker and the
    # branch name gives way to a short hash.
    git -C $repo checkout -q --detach
    assert_render "git: detached HEAD marker replaces the branch" $repo "➦" feature/x
    git -C $repo checkout -q feature/x

    # A conflicting rebase: the conflict marker, and HEAD is detached while
    # the rebase runs. Conflicts take precedence over the in-progress label
    # (git_resolver: conflicts render via $status), so the label is asserted
    # separately below on a rebase paused without conflicts.
    git -C $repo stash -q --include-untracked
    git -C $repo checkout -qb other main
    echo other >$repo/tracked.txt
    git -C $repo commit -qam other
    git -C $repo checkout -q feature/x
    git -C $repo rebase -q other >/dev/null 2>&1
    assert_render "git: conflicting rebase shows the conflict marker on a detached HEAD" $repo "✖1|➦" feature/x
    git -C $repo rebase --abort >/dev/null 2>&1

    # An interactive rebase stopped at an `edit` step with no conflicts: the
    # in-progress label with its step counter.
    env GIT_SEQUENCE_EDITOR="sed -i.bak -e 1s/^pick/edit/" git -C $repo rebase -qi main >/dev/null 2>&1
    assert_render "git: interactive rebase paused at edit shows REBASING 1/2" $repo "↻ REBASING 1/2" "✖"
    git -C $repo rebase --abort >/dev/null 2>&1

    # Back to clean: every dirty token gone.
    git -C $repo stash pop -q >/dev/null 2>&1
    git -C $repo add -A
    git -C $repo commit -qm clean
    git -C $repo stash clear
    assert_render "git: clean tree shows the branch and no dirty, stash or state tokens" \
        $repo feature/x "✱|?|≡|↻|✖"
end

# ---- 2. language ---------------------------------------------------------------
# Each case gets its own directory and its config is written BEFORE the first
# render there: the agent caches a directory's detection (including whether
# versions were probed) for hours, so a later config change would not be
# reflected by a re-render of the same directory.
function test_language_content
    print_test_header "Language segment content"

    set -l both $__gpy_test_tmp_dir/lang-both
    set -l primary $__gpy_test_tmp_dir/lang-primary
    for dir in $both $primary
        mkdir -p $dir
        printf '[package]\nname = "x"\nversion = "0.1.0"\n' >$dir/Cargo.toml
        printf 'fn main() {}\n' >$dir/main.rs
        printf '{"name": "x", "version": "1.0.0"}\n' >$dir/package.json
        printf 'console.log(1)\n' >$dir/index.js
    end

    write_config '[language]' 'display = "text"' 'show_versions = false'
    assert_render "language: rust and node both named in text mode" $both "rust|node" ""

    write_config '[language]' 'display = "text"' 'show_versions = false' 'filter = "primary"'
    function __one_language --inherit-variable primary
        set -g __render_last (render_prompt $primary)
        test (string match -ra 'rust|node' -- "$__render_last" | count) -eq 1
    end
    if poll_until 10 __one_language
        check "language: filter = primary names exactly one language" 1
    else
        check "language: filter = primary names exactly one language" 0 "$__render_last"
    end

    # Python with a project venv: the version comes from the venv's
    # pyvenv.cfg rather than the daemon's global interpreter, so a fixture
    # version proves the venv path was taken without needing python at all.
    write_config '[language]' 'display = "text"' 'show_versions = true'
    # pyproject.toml is one of the project markers segment_language_detect
    # gates on; a bare .py file alone never reaches the agent.
    set -l py $__gpy_test_tmp_dir/lang-py
    mkdir -p $py/.venv/bin
    printf 'def main(): pass\n' >$py/main.py
    printf '[build-system]\nrequires = ["setuptools"]\n' >$py/pyproject.toml
    printf 'home = /nonexistent\nversion = 3.99.7\n' >$py/.venv/pyvenv.cfg
    assert_render "language: python with the version read from the project venv" $py "python 3.99.7" ""

    # `[language] enabled = false` reaches the shell as GPY_LANGUAGE_ENABLED=0
    # through the theme export, and segment_language_detect then hides the
    # segment. (Setting the variable in config.fish instead is overwritten by
    # that same export; see the follow-up filed from #644.)
    write_config '[language]' 'enabled = false' 'display = "text"' 'show_versions = true'
    assert_render "language: [language] enabled = false hides the segment" $py "❯" "python|3.99.7"
end

# ---- 3. directory ---------------------------------------------------------------
# Rendered by the agent, so `~` is the agent's HOME: this test sets HOME to
# its own root before starting the agent, and the repo lives at $HOME/repo.
function test_directory_content --argument-names repo
    print_test_header "Directory segment display modes"

    set -l deep $repo/a/b/c
    mkdir -p $deep

    # `full` is the whole path with the home directory shown as `~`.
    set -l cases \
        "basename|c" \
        "abbreviated|~/r/a/b/c" \
        "truncated|a/b/c" \
        "full|~/repo/a/b/c"
    for case in $cases
        set -l mode (string split -m1 '|' -- $case)[1]
        set -l expected (string split -m1 '|' -- $case)[2]
        write_config '[ui.directory]' "display = \"$mode\""
        if wait_directory $deep "$expected"
            check "directory: display = $mode renders '$expected'" 1
        else
            check "directory: display = $mode renders '$expected'" 0 "got '$__render_last'"
        end
    end

    # truncated keeps truncation_length (3) components; anchored at the repo
    # root the repo folder is the leading one of those three.
    write_config '[ui.directory]' 'display = "truncated"' 'truncate_to_repo = true'
    if wait_directory $deep repo/b/c
        check "directory: truncate_to_repo anchors at the repo root" 1
    else
        check "directory: truncate_to_repo anchors at the repo root" 0 "got '$__render_last'"
    end

    write_config '[ui.directory]' 'display = "full"' 'max_length = 6'
    if wait_directory $deep "...b/c"
        check "directory: max_length = 6 keeps an ellipsis plus the tail" 1
    else
        check "directory: max_length = 6 keeps an ellipsis plus the tail" 0 "got '$__render_last'"
    end
end

# ---- 3b. show_icons = false: no powerline caps (#695) --------------------------------
# A raw render: SGR escapes go, but unlike `strip_sgr` the powerline caps
# (U+E0B0..U+E0BF) stay, because their absence is the assertion.
function render_prompt_raw --argument-names cwd
    fish --no-config -c '
        source $argv[1]/fish/core/init.fish
        source $argv[1]/fish/functions/fish_prompt.fish
        __gpy_register_with_agent >/dev/null 2>&1
        cd $argv[2]
        fish_prompt
        sleep 0.2
    ' -- $__gpy_root $cwd 2>/dev/null | string replace -ra '\e\[[0-9;]*m' '' | string collect
end

# Poll raw renders of $cwd until the directory segment is drawn and the
# presence of a powerline cap equals $expect_cap (1 or 0).
function wait_powerline_cap --argument-names cwd expect_cap
    function __powerline_cap_probe --inherit-variable cwd --inherit-variable expect_cap
        # `find`, not an `rm` glob: fish refuses a wildcard with no match.
        find $XDG_CACHE_HOME/gpy/instant-prompts -name '*.git.*' -delete 2>/dev/null
        set -g __render_last (render_prompt_raw $cwd)
        string match -q '*repo *' -- "$__render_last"; or return 1
        set -l has_cap 0
        string match -qr '[\x{e0b0}-\x{e0bf}]' -- "$__render_last"; and set has_cap 1
        test $has_cap = $expect_cap
    end
    poll_until 10 __powerline_cap_probe
end

function test_icons_off_draws_no_powerline_caps --argument-names repo
    print_test_header "show_icons = false draws no powerline caps"

    # Control: with icons on the same render does carry powerline caps, so
    # the icons-off assertion below cannot pass on an empty render.
    if wait_powerline_cap $repo 1
        check "icons on: the prompt carries powerline caps (control)" 1
    else
        check "icons on: the prompt carries powerline caps (control)" 0 (string escape -- "$__render_last")
    end

    set -g __show_icons false
    write_config
    if wait_powerline_cap $repo 0
        check "icons off: the prompt carries no powerline cap" 1
    else
        check "icons off: the prompt carries no powerline cap" 0 (string escape -- "$__render_last")
    end
end

# ---- 4. prompt frame (no agent: socket pointed at nothing) ---------------------------
function frame_render
    # $argv[1] and $argv[2] are code snippets run before/after sourcing the
    # integration; they travel as arguments to `eval`, like the repo root.
    fish --no-config -c '
        set -gx GPY_AGENT_SOCKET_PATH /nonexistent/gpy.sock
        set -gx GPY_AGENT_ENABLED 0
        set -gx GPY_AGENT_SUPERVISOR_ENABLED 0
        eval $argv[2]
        source $argv[1]/fish/core/init.fish
        source $argv[1]/fish/functions/fish_prompt.fish
        eval $argv[3]
        fish_prompt
    ' -- $__gpy_root "$argv[1]" "$argv[2]" 2>/dev/null | strip_sgr | string collect
end

function test_prompt_frame
    print_test_header "Prompt frame: icon mode and root prompt"

    set -l nerd (frame_render 'set -g __prompt_icons nerd' '')
    if text_matches "$nerd" "❯|✔" ""
        check "frame: nerd icons render ✔ and ❯" 1
    else
        check "frame: nerd icons render ✔ and ❯" 0 "$nerd"
    end

    set -l ascii (frame_render 'set -g __prompt_icons ascii' '')
    if text_matches "$ascii" "ok |> " "❯|✔"
        check "frame: ascii icons render ok and >" 1
    else
        check "frame: ascii icons render ok and >" 0 "$ascii"
    end

    set -l root (frame_render '' 'set -g __gpy_is_root 1; function __gpy_request_character; echo CHARACTER-REQUESTED; end')
    if text_matches "$root" '!❯!' CHARACTER-REQUESTED
        check "frame: root prompt renders !❯! without asking the agent for a character" 1
    else
        check "frame: root prompt renders !❯! without asking the agent for a character" 0 "$root"
    end
end

# ---------------------------------------------------------------------------------

init_test_env
# The agent canonicalises paths (/private/tmp on macOS), so HOME must be the
# real path for the `~` abbreviation to apply.
set -gx HOME (realpath $__gpy_test_tmp_dir)
write_config
set -l repo $HOME/repo
make_repo $repo

if not start_test_agent
    print_test_result "Agent Start" FAIL "gpy-agent failed to start"
    cleanup_test_files
    exit 1
end
print_test_result "Agent Start" PASS

test_git_content $repo
test_language_content
test_directory_content $repo
test_prompt_frame
test_icons_off_draws_no_powerline_caps $repo

cleanup_test_files

echo ""
echo "Passed: $__content_pass  Failed: $__content_fail"
test $__content_fail -eq 0
