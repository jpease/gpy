#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# tests/fish/script_segment_adapter.test.fish
#
# Script ("G") segments in Fish (fish/core/script.fish): the issue #997
# acceptance list, driven through the real init path. Each case writes a
# plugin directory with `sh` scripts and a theme export cache holding the
# `__gpy_script_*` block (the contract in docs/dev/plugins.md "Theme
# export"), then runs a fresh `fish --no-config -c` that sources
# fish/core/init.fish and emits the prompt events itself:
#
#   p   a new prompt with no command before it (first prompt, empty Enter)
#   c   a command ran (fish_postexec), then the next prompt
#
# Scripts count their runs by appending to a file outside the shell's HOME,
# TMPDIR and cache, so the no-files check can compare those trees.

source (dirname (status --filename))/../lib/test_helpers.fish

set -g scratch (realpath (mktemp -d))
test_track_dir $scratch

# The shells under test must not find an agent: the export comes from the
# cache file, and fish_prompt's agent requests then fork nothing.
set -g test_path
for dir in $PATH
    test -x "$dir/gpy-agent"; and continue
    set -a test_path $dir
end
set -g fish_bin (command -s fish)

set -g case_dir ""
set -g export_lines
set -g export_toks

function reset_fields
    set -g F_name ""
    set -g F_req ""
    set -g F_inputs
    set -g F_env
    set -g F_opts
    set -g F_ttl 0
    set -g F_timeout 500
    set -g F_reqfiles
    set -g F_reqfolders
    set -g F_reqexts
    set -g F_reqenv
end

function new_case --argument-names name
    set -g case_dir $scratch/$name
    mkdir -p $case_dir/home/.cache/gpy $case_dir/home/.config $case_dir/tmp $case_dir/plugin $case_dir/out $case_dir/work
    set -g export_lines
    set -g export_toks
    set -g run_lc_all ""
    set -g run_extra_env
    set -g run_segments ""
    reset_fields
end

function write_script --argument-names path body
    printf '#!/bin/sh\n%s\n' $body >$path
    chmod 755 $path
end

# One `set -g NAME VALUE...` line, every value quoted for Fish.
function xs
    set -l line "set -g $argv[1]"
    for value in $argv[2..]
        set line "$line "(string escape -- $value)
    end
    printf '%s\n' $line
end

# Add a script segment with the current F_* fields; resets them.
function add_seg --argument-names tok body
    set -l name $F_name
    test -n "$name"; or set name (string replace -a -- _ - $tok)
    set -l cmd $case_dir/plugin/$tok.sh
    write_script $cmd $body
    set -l req ""
    if test -n "$F_req"
        set req $case_dir/plugin/$tok.req.sh
        write_script $req $F_req
    end
    set -l scope session
    contains -- pwd $F_inputs; and set scope folder
    set -l p __gpy_script_$tok
    set -a export_lines (xs {$p}_name $name) (xs {$p}_root $case_dir/plugin) (xs {$p}_command $cmd) \
        (xs {$p}_reqcommand $req) (xs {$p}_inputs $F_inputs) (xs {$p}_env $F_env) (xs {$p}_opts $F_opts) \
        (xs {$p}_ttl $F_ttl) (xs {$p}_timeout $F_timeout) (xs {$p}_scope $scope) \
        (xs {$p}_reqfiles $F_reqfiles) (xs {$p}_reqfolders $F_reqfolders) (xs {$p}_reqexts $F_reqexts) \
        (xs {$p}_reqenv $F_reqenv) (xs {$p}_key "4:test1:13:$name")
    set -a export_toks $tok
    set -g run_segments "$run_segments $name"
    reset_fields
end

function write_export
    begin
        printf '%s\n' 'set -g __gpy_export_version "1"'
        xs __gpy_script_sh /bin/sh
        printf '%s\n' $export_lines
        xs __gpy_script_segments $export_toks
        xs __gpy_add_newline 0
    end >$case_dir/home/.cache/gpy/theme-export.fish
end

# Run CODE in a fresh Fish that loaded GPY from this checkout. stdout and
# stderr are the caller's.
function run_fish --argument-names code
    set -l envs HOME=$case_dir/home XDG_CACHE_HOME=$case_dir/home/.cache XDG_CONFIG_HOME=$case_dir/home/.config \
        XDG_RUNTIME_DIR=$case_dir/runtime TMPDIR=$case_dir/tmp PATH=(string join : -- $test_path) \
        GPY_AGENT_ENABLED=1 GPY_AGENT_SUPERVISOR_ENABLED=0 GPY_T_SEGMENTS=(string trim -- $run_segments) \
        GPY_T_ROOT=$__gpy_root TERM=dumb
    set -q LANG; and set -a envs LANG=$LANG
    test -n "$run_lc_all"; and set -a envs LC_ALL=$run_lc_all
    set -l pre '
        set -g __gpy_test_segments (string split " " -- "$GPY_T_SEGMENTS")
        source $GPY_T_ROOT/fish/core/init.fish
        source $GPY_T_ROOT/fish/functions/fish_prompt.fish
        function p; emit fish_prompt; end
        function c; emit fish_postexec; emit fish_prompt; end
        function show
            for t in $argv
                set -l v __gpy_g_t_{$t}_text
                printf "%s=[%s]\n" $t "$$v"
            end
        end
        function hex --argument-names t
            set -l v __gpy_g_t_{$t}_text
            printf "%s" "$$v" | od -An -tx1 | string join "" | string replace -a " " ""
        end
    '
    command env -i $envs $run_extra_env $fish_bin --no-config -c "$pre
$code"
end

function runs --argument-names file
    test -f $file; or begin
        echo 0
        return
    end
    count <$file
end

function no_process --argument-names pattern
    not pgrep -f -- $pattern >/dev/null
end

# --- Once per command; repaints and SIGURG never run the script ---------
print_test_header "once per command across repaints"
new_case once
set -l cnt $case_dir/out/count
add_seg demo "echo run >>'$cnt'; echo hello"
write_export
set -l out (run_fish '
    p
    show demo
    fish_prompt >/dev/null 2>&1
    kill -s URG $fish_pid
    fish_prompt >/dev/null 2>&1
    fish_prompt --final-rendering >/dev/null 2>&1
    p
    p
    show demo
    fish_prompt 2>/dev/null | string replace -ra "\e\[[0-9;]*m" ""
    echo
    c
    show demo
' 2>&1)
check "first prompt runs the script and stores its line" "demo=[hello]" "$out[1]"
check "repaints, SIGURG and empty Enters keep the text" "demo=[hello]" "$out[2]"
if string match -q '* hello*' -- "$out[3..]"
    pass "fish_prompt renders the script segment"
else
    fail "fish_prompt renders the script segment" "$out"
end
check "script ran twice: first prompt and one command (ttl 0)" 2 (runs $cnt)

# --- Cache: ttl, key from declared values -------------------------------
print_test_header "cache keyed by declared values, ttl"
new_case cache
set -l cnt $case_dir/out/count
set F_ttl 60
set F_env T_VAR
add_seg demo "echo run >>'$cnt'; printf 'v=%s\n' \"\${T_VAR-unset}\""
write_export
set -l out (run_fish '
    set -gx T_VAR a
    p
    show demo
    c
    c
    set -gx T_VAR b
    c
    show demo
    set -e T_VAR
    c
    show demo
')
check "declared env value reaches the script" "demo=[v=a]" "$out[1]"
check "a changed value is a miss" "demo=[v=b]" "$out[2]"
check "unset is a distinct key" "demo=[v=unset]" "$out[3]"
check "within ttl the same key is a hit (3 runs for 5 due prompts)" 3 (runs $cnt)

# --- Newline and ESC in $PWD --------------------------------------------
print_test_header "newline and ESC in PWD"
new_case pwd
set -l cnt $case_dir/out/count
set -l dir "$case_dir/work/"(printf 'n\nl' | string collect)/(printf 'e\e[31mx')
mkdir -p $dir
set F_inputs pwd
set F_ttl 60
add_seg demo "echo run >>'$cnt'; printf '%s' \"\$GPY_PWD\" >'$case_dir/out/gpy_pwd'; pwd -P >'$case_dir/out/cwd'; printf '[%s]\n' \"\${GPY_PWD##*/}\""
write_export
set -lx GPY_T_DIR $dir
set -g run_extra_env GPY_T_DIR=$dir
set -l out (run_fish '
    cd $GPY_T_DIR
    p
    show demo
    c
    hex demo
')
check "ESC stripped from the stored text" "demo=[[e[31mx]]" "$out[1]"
check "stored text holds no control byte" 5b655b33316d785d "$out[2]"
check "GPY_PWD arrives byte for byte" "$dir" (string collect <$case_dir/out/gpy_pwd)
check "script runs in PWD for a pwd input" "$dir" (string collect <$case_dir/out/cwd | string trim -r -c \n | string collect)
check "same folder is a cache hit" 1 (runs $cnt)

# --- Stale state and PID reuse ------------------------------------------
print_test_header "stale state from the environment, PID-named files"
new_case stale
set -l cnt $case_dir/out/count
set F_ttl 600
add_seg demo "echo run >>'$cnt'; echo fresh"
write_export
set -g run_extra_env __gpy_g_t_demo_text=PLANTED __gpy_g_t_demo_ctext=PLANTED "__gpy_g_t_demo_ckey=4:test1:13:demo" \
    __gpy_g_t_demo_at=99999999999 __gpy_g_t_demo_off=1 __gpy_g_due=0 __gpy_g_tokens=demo
set -l out (run_fish '
    for d in $TMPDIR $HOME/.cache/gpy $HOME/.cache/gpy/script
        mkdir -p $d
        echo PLANTED >$d/gpy-g-$fish_pid
        echo PLANTED >$d/$fish_pid
    end
    segment_demo_detect; and echo detect-before
    p
    show demo
')
check "no inherited text before the first run" "demo=[fresh]" "$out[1]"
check "inherited cache and breaker state ignored" 1 (runs $cnt)

new_case stale_noexport
set -l cnt $case_dir/out/count
write_script $case_dir/plugin/ghost.sh "echo run >>'$cnt'; echo ghost"
set -g run_segments ghost
set -g run_extra_env __gpy_script_segments=ghost __gpy_script_sh=/bin/sh __gpy_script_ghost_name=ghost \
    __gpy_script_ghost_command=$case_dir/plugin/ghost.sh __gpy_script_ghost_root=$case_dir/plugin \
    __gpy_script_ghost_ttl=0 __gpy_script_ghost_timeout=500 __gpy_script_ghost_scope=session
set -l out (run_fish '
    p
    c
    printf "tokens=[%s]\n" "$__gpy_g_tokens"
' 2>/dev/null)
check "a list from the environment is not used without an export" "tokens=[]" "$out[1]"
check "inherited segment never runs" 0 (runs $cnt)

# --- C1 controls, locale independent ------------------------------------
for locale in C en_US.UTF-8
    print_test_header "C1 and C0 under LC_ALL=$locale"
    new_case c1_(string replace -a . _ $locale)
    set -g run_lc_all $locale
    add_seg csi "printf 'a\\302\\233b\\n'"
    add_seg joined "printf '\\302\\033\\233x\\n'"
    add_seg nel "printf 'a\\302\\205\\033\\007\\177\\tb\\n'"
    add_seg utf "printf 'caf\\303\\251 \\342\\206\\222\\n'"
    add_seg lone "printf 'a\\233b\\n'"
    write_export
    set -l out (run_fish '
        p
        for t in csi joined nel utf lone
            printf "%s=%s\n" $t (hex $t)
        end
    ')
    check "CSI (c2 9b) hides the segment" "csi=" "$out[1]"
    check "C1 formed by the C0 strip hides the segment" "joined=" "$out[2]"
    check "NEL with C0 and DEL hides the segment" "nel=" "$out[3]"
    check "UTF-8 text passes unchanged" "utf=636166c3a920e28692" "$out[4]"
    check "lone 0x9b byte is kept (documented gap)" "lone=619b62" "$out[5]"
end

# --- Hostile names ------------------------------------------------------
print_test_header "a[\$(cmd)] and glob characters in manifest names"
new_case names
set -l cnt $case_dir/out/count
set -l pwned $case_dir/out/pwned
set F_env "a[\$(touch $pwned.env)]"
add_seg envname "echo run >>'$cnt'; echo envname"
set F_reqenv "a[\$(touch $pwned.reqenv)]"
add_seg reqenvname "echo run >>'$cnt'; echo reqenvname"
set F_opts "GPY_OPT_x[\$(touch $pwned.opt)]=1"
add_seg optname "echo run >>'$cnt'; echo optname"
add_seg variant "printf 'shown\\na[\$(touch $pwned.variant)]\\n'"
set F_reqfiles '*' 'f?le'
set F_reqfolders '[ab]'
add_seg globs "echo globs"
write_export
# A hostile token next to the valid ones.
set -a export_lines (xs "__gpy_script_segments" $export_toks "x[\$(touch $pwned.tok)]" '*')
write_export
mkdir -p $case_dir/work/g/a
touch $case_dir/work/g/file $case_dir/work/g/other
set -g run_extra_env GPY_T_G=$case_dir/work/g
set -l out (run_fish '
    cd $GPY_T_G
    p
    show envname reqenvname optname variant globs
    printf "variant=[%s]\n" "$__gpy_g_t_variant_variant"
    printf "tokens=[%s]\n" "$__gpy_g_tokens"
    touch "*" "f?le"
    mkdir "[ab]"
    c
    show globs
')
check "env name a[\$(cmd)] skips the segment" "envname=[]" "$out[1]"
check "require_env name a[\$(cmd)] skips the segment" "reqenvname=[]" "$out[2]"
check "setting name a[\$(cmd)] skips the segment" "optname=[]" "$out[3]"
check "text shown with a hostile variant line" "variant=[shown]" "$out[4]"
check "glob names do not match other files" "globs=[]" "$out[5]"
check "hostile variant ignored" "variant=[]" "$out[6]"
check "hostile tokens dropped" "tokens=[envname reqenvname optname variant globs]" "$out[7]"
check "literal glob names match their own files" "globs=[globs]" "$out[8]"
check "no hostile segment ran" 0 (runs $cnt)
if test -n "$(path filter $pwned.*)"
    fail "no command substitution ran from a name" (path filter $pwned.*)
else
    pass "no command substitution ran from a name"
end

# --- No files ------------------------------------------------------------
print_test_header "no cache or state files"
new_case nofiles
set F_ttl 60
set F_inputs pwd is_root is_sudo is_ssh cmd_duration prompt_icons
set F_reqfiles marker
add_seg demo "echo files"
write_export
touch $case_dir/work/marker
# Fish creates its own config and data directories on first start.
run_fish true
set -l before (find $case_dir/home $case_dir/tmp $case_dir/plugin $case_dir/work | sort)
set -g run_extra_env GPY_T_W=$case_dir/work
set -l out (run_fish '
    cd $GPY_T_W
    p
    c
    c
    show demo
')
set -l after (find $case_dir/home $case_dir/tmp $case_dir/plugin $case_dir/work | sort)
check "segment ran" "demo=[files]" "$out[1]"
check "no file created in HOME, TMPDIR, plugin or PWD" "$before" "$after"

# --- Output cap ---------------------------------------------------------
print_test_header "4096-byte output cap, counted in bytes"
new_case cap
add_seg big "head -c 5000000 /dev/zero | tr '\\0' a; echo"
add_seg wide "i=0; while [ \$i -lt 2049 ]; do printf '\\303\\251'; i=\$((i + 1)); done; echo"
add_seg fits "i=0; while [ \$i -lt 2000 ]; do printf '\\303\\251'; i=\$((i + 1)); done; echo"
write_export
set -l err $case_dir/out/stderr
set -l out (run_fish '
    p
    show big wide
    set -l v __gpy_g_t_fits_text
    printf "fits=%s\n" (printf "%s" "$$v" | wc -c | string trim)
' 2>$err)
check "5 MB of output hides the segment" "big=[]" "$out[1]"
check "4098 bytes of two-byte characters hide the segment" "wide=[]" "$out[2]"
check "4000 bytes of two-byte characters pass" fits=4000 "$out[3]"
check "no text on stderr (no Fish read-limit error)" "" (string collect <$err)

# --- Timeout with a grandchild holding stdout ---------------------------
print_test_header "timeout with a grandchild holding stdout"
new_case timeout
set F_timeout 200
add_seg hung "sh -c 'sleep 7.31' & sleep 7.32; echo late"
set F_timeout 500
add_seg leftover "sh -c 'trap \"\" TERM; exec sleep 7.33' & echo early"
write_export
set -l t0 (test_now_ns)
set -l out (run_fish '
    p
    show hung leftover
')
set -l elapsed_ms (math -s0 "("(test_now_ns)" - $t0) / 1000000")
check "timed-out segment hidden" "hung=[]" "$out[1]"
check "a normal exit with a leftover shows its line" "leftover=[early]" "$out[2]"
if test $elapsed_ms -lt 3000
    pass "run bounded by the timeout ($elapsed_ms ms for the whole shell)"
else
    fail "run bounded by the timeout" "$elapsed_ms ms"
end
if poll_until 2 no_process 'sleep 7.3[123]'
    pass "script tree and leftover killed"
else
    fail "script tree and leftover killed" (pgrep -fl 'sleep 7.3[123]')
    pkill -f 'sleep 7.3[123]'
end

# --- require_command ----------------------------------------------------
print_test_header "require_command under the timeout"
new_case reqcmd
set -l cnt $case_dir/out/count
set F_timeout 200
set F_req "sleep 7.34"
add_seg slow "echo run >>'$cnt'; echo slow"
set F_req "exit 1"
add_seg no "echo run >>'$cnt'; echo no"
set F_req "test \"\$GPY_OPT_k\" = v"
set F_opts GPY_OPT_k=v
add_seg yes "echo yes"
write_export
set -l t0 (test_now_ns)
set -l out (run_fish '
    p
    show slow no yes
')
set -l elapsed_ms (math -s0 "("(test_now_ns)" - $t0) / 1000000")
check "require_command past the timeout hides the segment" "slow=[]" "$out[1]"
check "failing require_command hides the segment" "no=[]" "$out[2]"
check "passing require_command (same environment) runs command" "yes=[yes]" "$out[3]"
check "command never ran after a failed or timed-out require_command" 0 (runs $cnt)
if test $elapsed_ms -lt 3000
    pass "require_command bounded by the timeout ($elapsed_ms ms)"
else
    fail "require_command bounded by the timeout" "$elapsed_ms ms"
end
if poll_until 2 no_process 'sleep 7.34'
    pass "require_command tree killed"
else
    fail "require_command tree killed"
    pkill -f 'sleep 7.34'
end

# --- Circuit breaker ----------------------------------------------------
print_test_header "circuit breaker"
new_case breaker
set -l cnt $case_dir/out/count
set F_timeout 50
add_seg slow "echo run >>'$cnt'; sleep 7.35"
write_export
set -l err $case_dir/out/stderr
set -l out (run_fish '
    p
    c
    c
    c
    printf "runs=%s\n" (count < '$cnt')
    __gpy_reload_theme
    c
' 2>$err)
check "two timeouts disable the segment" runs=2 "$out[1]"
check "reload resets the breaker" 3 (runs $cnt)
check "one stderr line" 1 (count <$err)
pkill -f 'sleep 7.35'

# Scan and run strikes count apart: one of each leaves the segment on.
new_case strikes
add_seg demo "echo hi"
write_export
set -l out (run_fish '
    p
    __gpy_g_trip demo scan
    __gpy_g_trip demo run
    echo off=$__gpy_g_t_demo_off
    __gpy_g_trip demo run
    echo off=$__gpy_g_t_demo_off
' 2>/dev/null)
check "one scan and one run strike keep the segment" off=0 "$out[1]"
check "a second run strike disables it" off=1 "$out[2]"

# --- Failed trigger drops the cache entry -------------------------------
print_test_header "failed trigger drops the cache"
new_case trigdrop
set -l cnt $case_dir/out/count
set F_ttl 60
set F_reqenv T_ON
add_seg demo "echo run >>'$cnt'; echo hi"
write_export
run_fish '
    set -gx T_ON 1
    p
    set -e T_ON
    c
    set -gx T_ON 1
    c
' >/dev/null 2>&1
check "trigger back after a failure runs again (no stale cache hit)" 2 (runs $cnt)

# --- Clean environment --------------------------------------------------
print_test_header "clean environment, values never in argv"
new_case cleanenv
set -l envout $case_dir/out/env
set -l argvout $case_dir/out/argv
set F_env T_DECL
set F_opts GPY_OPT_label=good
add_seg demo "env >'$envout'; ps -A -ww -o args= >'$argvout' 2>/dev/null; echo ok"
write_export
set -g run_extra_env T_SECRET=s3cr3t-unique GPY_OPT_label=evil "T_DECL=v4lue-\$(id)-unique"
set -l out (run_fish '
    set -gx PATH . "" $PATH
    p
    show demo
')
check "script ran" "demo=[ok]" "$out[1]"
set -l envs (string split \n -- (string collect <$envout))
contains -- "T_DECL=v4lue-\$(id)-unique" $envs; and pass "declared value arrives literally"; or fail "declared value arrives literally" $envs
string match -q '*s3cr3t*' -- $envs; and fail "undeclared variable absent" $envs; or pass "undeclared variable absent"
contains -- GPY_OPT_label=good $envs; and pass "setting overrides an exported GPY_OPT_*"; or fail "setting overrides an exported GPY_OPT_*" $envs
set -l path_entries (string split : -- (string replace -r '^PATH=' '' -- (string match 'PATH=*' -- $envs)))
if contains -- . $path_entries; or contains -- "" $path_entries
    fail "relative PATH entries dropped" "$path_entries"
else
    pass "relative PATH entries dropped"
end
if test -s $argvout
    if string match -q '*v4lue-*' -- (string collect <$argvout)
        fail "declared value in no process argv"
    else
        pass "declared value in no process argv"
    end
end

# --- A declared PATH never restores relative entries ---------------------
print_test_header "declared PATH keeps the cleaned value"
new_case declpath
set -l plant $case_dir/work/plant
mkdir -p $plant/rel
for d in $plant $plant/rel
    write_script $d/plantedtool "echo x >>'$case_dir/out/planted'"
end
set -l cnt $case_dir/out/count
set -l pathout $case_dir/out/path
set -l reqout $case_dir/out/reqpath
set F_ttl 60
add_seg plain "printf '%s' \"\$PATH\" >'$pathout'.plain; plantedtool; echo plain"
set F_env PATH
set F_ttl 60
set F_req "printf '%s' \"\$PATH\" >'$reqout'; plantedtool; exit 0"
add_seg decl "echo run >>'$cnt'; printf '%s' \"\$PATH\" >'$pathout'; plantedtool; echo decl"
write_export
set -g run_extra_env GPY_T_PLANT=$plant
set -l out (run_fish '
    cd $GPY_T_PLANT
    set -gx PATH . rel "" /usr/bin /bin
    p
    show plain decl
    c
    set -gx PATH /bin . rel /usr/bin
    c
')
check "the declared-PATH script ran" "decl=[decl]" "$out[2]"
check "an undeclared PATH is cleaned, absolute entries in order" "/usr/bin:/bin" (string collect <$pathout.plain)
check "require_command sees the cleaned PATH" "/bin:/usr/bin" (string collect <$reqout)
test -e $case_dir/out/planted; and fail "a declared PATH never ran a planted tool (command, require_command)"; or pass "a declared PATH never ran a planted tool (command, require_command)"
check "an unchanged declared PATH is a cache hit, a changed one a miss" 2 (runs $cnt)
check "the changed PATH is cleaned in order" "/bin:/usr/bin" (string collect <$pathout)

# --- Rendering with exported styles -------------------------------------
print_test_header "render with exported styles"
new_case render
add_seg demo "printf 'txt\\nwarn\\n'"
add_seg plain "echo plain"
set -a export_lines (xs __color_demo_bg blue) (xs __color_demo_fg white) (xs __color_demo_warn_bg red) \
    (xs __color_demo_warn_fg black) (xs __icon_demo ICON) (xs __color_plain_bg green) \
    (xs __gpy_segment_plain_open_icon '<') (xs __gpy_segment_plain_close_icon '>')
write_export
set -l out (run_fish '
    p
    set -g __gpy_segment_position first
    segment_demo_render "" true | string replace -ra "\e\[[0-9;]*m" ""
    echo
    printf "bg=%s\n" $__gpy_last_segment_bg
    segment_plain_render true "" | string replace -ra "\e\[[0-9;]*m" ""
    echo
')
if string match -q '* ICON txt*' -- "$out[1]"
    pass "icon and text rendered"
else
    fail "icon and text rendered" "$out[1]"
end
check "variant colour wins" bg=red "$out[2]"
if string match -q '< plain>' -- "$out[3]"
    pass "open and close caps replace the delimiters"
else
    fail "open and close caps replace the delimiters" "$out[3]"
end

# Last pill: the theme close cap sits on the terminal default background.
set -l out (run_fish '
    p
    set -gx TERM xterm-256color
    set -g __prompt_close_bg magenta
    set -g __segment_delim_last E
    set -g __gpy_segment_position middle
    segment_demo_render true "" | string escape; echo
    gpy_section_standalone blue white x true | string escape; echo
    __gpy_g_clear_styles
    set -q __color_demo_bg __color_demo_warn_bg __icon_demo __gpy_segment_plain_open_icon
    printf "left=%s\n" $status
')
if string match -q -- '*E*' "$out[1]"; and not string match -qr -- '45m[^E]*E' "$out[1]"
    pass "a last plugin pill draws its close cap on the default background"
else
    fail "a last plugin pill draws its close cap on the default background" "$out[1]"
end
if string match -qr -- '45m[^E]*E' "$out[2]"
    pass "a built-in last segment keeps the prompt_close background"
else
    fail "a built-in last segment keeps the prompt_close background" "$out[2]"
end
check "a theme reload clears the plugin segments' old style variables" left=4 "$out[3]"

test_finish "script segment adapter"
