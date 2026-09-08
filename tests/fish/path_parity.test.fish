#!/usr/bin/env fish
# Differential cross-shell path parity harness (#476).
#
# Path resolution is implemented four times -- in the Rust agent/CLI, in Fish,
# in Bash and in Zsh -- for two roots (runtime and cache). They agree today only
# by convention, and nothing enforced it. This harness runs
# `gpy debug paths --format kv` and each shell's `__gpy_debug_paths` under a
# matrix of synthetic environments and diffs the FULL maps, so #477 can
# centralize resolution and prove it changed nothing on Unix.
#
# Three contracts are enforced here, all of them ratchets -- each fails when the
# world gets BETTER as well as when it gets worse, so nothing rots into a
# permanent green:
#
#   1. Key set and order. Every implementation emits every key in
#      GPY_PP_KEYS order. A key added on one side and not the others fails.
#
#   2. Ownership (GPY_PP_OWNERS). Each key names the implementations that
#      resolve it. An owner emitting <unimplemented> fails, and so does a
#      non-owner emitting a path. When #477 teaches Bash to read a theme
#      export, this table must be updated in the same commit.
#
#   3. Divergences (GPY_PP_DIVERGENCES). Every disagreement between the agent
#      and a shell must be declared, with the value it currently produces and
#      the issue that will fix it. An UNDECLARED disagreement fails. So does a
#      declared one whose value changed shape. So does a declared one that has
#      started AGREEING -- that is what makes this a ratchet rather than a wish
#      list: fixing a divergence in #477 forces the entry (and its issue
#      reference) to be deleted here, in the same commit, or the gate goes red.
#      And an entry that never fires anywhere fails as obsolete.
#
# #476 recorded the contract rather than fixing it; #626 then decided every
# recorded divergence and implemented one answer across the agent, Fish, Bash
# and Zsh, which is why GPY_PP_DIVERGENCES below is now empty.
#
# The one path #476 left uncovered -- `gpy_config_runtime_dir` /
# `gpy_config_path` in fish/core/util.fish, a THIRD runtime-root
# implementation (fallback `/tmp/gpy-$USER`, not `/tmp/gpy`) pointing at a
# `config.json` snapshot nothing in the agent ever writes -- was deleted by
# #626 rather than diffed: it had no second side to compare against, its
# readers (`gpy_setup` printed it, `gpy_config_validate` `test -f`'d it, never
# true) now use `__gpy_runtime_root` and `__gpy_locate_config_path`, and a
# fourth resolver was exactly the drift this harness exists to prevent.

set -l repo_root (path resolve (status dirname)/../..)

echo "Testing cross-shell path parity..."

# ============================================================================
# CONTRACT
# ============================================================================

# The key set, in emission order. Mirrors `commands::debug_paths::KEYS`.
set -g GPY_PP_KEYS \
    runtime_root \
    socket \
    shell_registry_dir \
    cache_root \
    instant_prompts_dir \
    theme_export_file \
    config_path \
    config_candidates \
    theme_dir

# Which implementations resolve each key. A key is compared only between the
# agent and the shells that own it; a non-owner must say <unimplemented> so the
# gap is a declared property rather than an accident.
#
# Every shell talks to the agent, reads its pre-rendered cache, and (since
# #638) records its PID under the runtime root's shells/ directory for the
# agent's restart nudge. Fish additionally sources the theme export by path
# and locates config.toml for its own prompt resolution. theme_dir is the
# agent's alone -- no shell ever locates a theme file.
set -g GPY_PP_OWNERS \
    "runtime_root:rust,fish,bash,zsh" \
    "socket:rust,fish,bash,zsh" \
    "shell_registry_dir:rust,fish,bash,zsh" \
    "cache_root:rust,fish,bash,zsh" \
    "instant_prompts_dir:rust,fish,bash,zsh" \
    "theme_export_file:rust,fish" \
    "config_path:rust,fish" \
    "config_candidates:rust,fish" \
    "theme_dir:rust"

# Declared divergences, `^`-separated (paths and the `|`-joined candidate list
# rule out the obvious separators):
#
#   cases ^ key ^ impl ^ shell-value-glob ^ agent-value-glob ^ issue ^ why
#
# `cases` is either a comma-separated list of case names or `*`. A named entry
# wins over a `*` entry for the same key and impl, so `*` is the fallback for
# "diverges everywhere else"; two named entries claiming the same comparison is
# an error. Either kind is held to its word: the moment a covered comparison
# AGREES, the entry fails.
#
# BOTH sides are pinned. The shell glob alone would let #477 change the agent's
# value while the shell still disagreed, and the entry would absorb it silently
# — which is precisely the change this harness exists to catch. Globs match the
# normalized values (%T% = scratch root, %PWHOME% = passwd home).
#
# Every entry below was produced by running this matrix, not by reading code.
#
# The table is EMPTY, and that is the contract now: #626 decided each of the
# 32 recorded disagreements (empty/relative `$XDG_*` values, an absent `HOME`,
# Fish's extra `$HOME/.gpy.toml` config candidate, and empty
# `GPY_CONFIG_PATH`/`GPY_AGENT_SOCKET_PATH` overrides) and implemented one
# answer across the agent and all three shells, so every comparison in the
# matrix agrees. Keep it empty: a new entry here is a new way for the prompt
# and the agent to read and write different paths, and needs the same
# treatment -- decide the correct behaviour and implement it everywhere --
# rather than a note.
set -g GPY_PP_DIVERGENCES

# Environment variables the matrix controls. Any variable named in a case is
# exported with that value; every other one here is explicitly unset, so a case
# never inherits the developer's own environment.
set -g GPY_PP_VARS XDG_RUNTIME_DIR XDG_CACHE_HOME XDG_CONFIG_HOME HOME GPY_AGENT_SOCKET_PATH GPY_CONFIG_PATH

# ============================================================================
# FIXTURE
# ============================================================================

set -g GPY_PP_BIN $repo_root/gpy-agent/target/debug/gpy
if not test -x "$GPY_PP_BIN"
    echo "❌ $GPY_PP_BIN not found -- build it first (cd gpy-agent && cargo build)"
    exit 1
end

for shell_bin in fish bash zsh
    if not command -q $shell_bin
        echo "❌ $shell_bin not found; the parity harness compares all three shells"
        exit 1
    end
end

set -g GPY_PP_ROOT $repo_root
set -g GPY_PP_TMP (path resolve (mktemp -d))
set -g GPY_PP_ERR $GPY_PP_TMP/stderr.log

# Fish and Zsh re-derive $HOME from the passwd database when the environment
# does not carry it; Bash and the agent do not. That is itself a path
# divergence worth recording, but the derived path is machine-specific, so
# normalize it to %PWHOME% and let the divergence table quote it portably.
set -g GPY_PP_PWHOME (env -u HOME fish --no-config -c 'echo $HOME' 2>/dev/null)

# `home` has both a `.config/gpy/config.toml` and a `~/.gpy.toml`; `home2` has
# only the latter, which is the shape that exposes Fish's extra candidate.
# `emptyconf` is an XDG_CONFIG_HOME directory holding no config at all -- the
# shape reported live on #476. `home3` holds no config of any kind, which is
# what isolates the empty-XDG_CONFIG_HOME divergence onto `config_path`: with
# no candidate existing anywhere, each side falls back to its own first
# candidate and the difference stops being hidden by an existence check.
mkdir -p \
    $GPY_PP_TMP/run \
    $GPY_PP_TMP/cache \
    $GPY_PP_TMP/conf/gpy \
    $GPY_PP_TMP/home/.config/gpy \
    $GPY_PP_TMP/home2 \
    $GPY_PP_TMP/home3 \
    $GPY_PP_TMP/emptyconf \
    $GPY_PP_TMP/cfg \
    $GPY_PP_TMP/sock \
    $GPY_PP_TMP/cwd
touch \
    $GPY_PP_TMP/conf/gpy/config.toml \
    $GPY_PP_TMP/home/.config/gpy/config.toml \
    $GPY_PP_TMP/home/.gpy.toml \
    $GPY_PP_TMP/home2/.gpy.toml \
    $GPY_PP_TMP/cfg/custom.toml

# ============================================================================
# MATRIX
# ============================================================================
#
# `name;VAR=value;VAR=value...`. A variable absent from a case is unset; a
# variable with an empty value is set-but-empty, which is the state the three
# shells and the agent were each reported to handle differently.
set -g GPY_PP_CASES \
    "xdg_all_set;XDG_RUNTIME_DIR=%T%/run;XDG_CACHE_HOME=%T%/cache;XDG_CONFIG_HOME=%T%/conf;HOME=%T%/home" \
    "runtime_unset;XDG_CACHE_HOME=%T%/cache;XDG_CONFIG_HOME=%T%/conf;HOME=%T%/home" \
    "runtime_empty;XDG_RUNTIME_DIR=;XDG_CACHE_HOME=%T%/cache;XDG_CONFIG_HOME=%T%/conf;HOME=%T%/home" \
    "cache_unset;XDG_RUNTIME_DIR=%T%/run;XDG_CONFIG_HOME=%T%/conf;HOME=%T%/home" \
    "cache_empty;XDG_RUNTIME_DIR=%T%/run;XDG_CACHE_HOME=;XDG_CONFIG_HOME=%T%/conf;HOME=%T%/home" \
    "runtime_and_cache_unset;XDG_CONFIG_HOME=%T%/conf;HOME=%T%/home" \
    "runtime_unset_cache_empty;XDG_CACHE_HOME=;XDG_CONFIG_HOME=%T%/conf;HOME=%T%/home" \
    "runtime_empty_cache_empty;XDG_RUNTIME_DIR=;XDG_CACHE_HOME=;XDG_CONFIG_HOME=%T%/conf;HOME=%T%/home" \
    "config_unset;XDG_RUNTIME_DIR=%T%/run;XDG_CACHE_HOME=%T%/cache;HOME=%T%/home" \
    "config_empty;XDG_RUNTIME_DIR=%T%/run;XDG_CACHE_HOME=%T%/cache;XDG_CONFIG_HOME=;HOME=%T%/home" \
    "config_empty_dir;XDG_RUNTIME_DIR=%T%/run;XDG_CACHE_HOME=%T%/cache;XDG_CONFIG_HOME=%T%/emptyconf;HOME=%T%/home" \
    "config_empty_no_config;XDG_RUNTIME_DIR=%T%/run;XDG_CACHE_HOME=%T%/cache;XDG_CONFIG_HOME=;HOME=%T%/home3" \
    "home_unset;XDG_RUNTIME_DIR=%T%/run;XDG_CACHE_HOME=%T%/cache;XDG_CONFIG_HOME=%T%/conf" \
    "home_unset_cache_unset;XDG_RUNTIME_DIR=%T%/run;XDG_CONFIG_HOME=%T%/conf" \
    "all_unset;" \
    "home_only;HOME=%T%/home" \
    "home_dotgpy_only;HOME=%T%/home2" \
    "cache_only;XDG_CACHE_HOME=%T%/cache" \
    "socket_override;XDG_RUNTIME_DIR=%T%/run;XDG_CACHE_HOME=%T%/cache;XDG_CONFIG_HOME=%T%/conf;HOME=%T%/home;GPY_AGENT_SOCKET_PATH=%T%/sock/custom.sock" \
    "socket_override_no_xdg;HOME=%T%/home;GPY_AGENT_SOCKET_PATH=%T%/sock/custom.sock" \
    "socket_override_empty;XDG_RUNTIME_DIR=%T%/run;XDG_CACHE_HOME=%T%/cache;HOME=%T%/home;GPY_AGENT_SOCKET_PATH=" \
    "config_override;XDG_RUNTIME_DIR=%T%/run;XDG_CACHE_HOME=%T%/cache;XDG_CONFIG_HOME=%T%/conf;HOME=%T%/home;GPY_CONFIG_PATH=%T%/cfg/custom.toml" \
    "config_override_missing;XDG_RUNTIME_DIR=%T%/run;XDG_CACHE_HOME=%T%/cache;XDG_CONFIG_HOME=%T%/conf;HOME=%T%/home;GPY_CONFIG_PATH=%T%/cfg/absent.toml" \
    "config_override_empty;XDG_RUNTIME_DIR=%T%/run;XDG_CACHE_HOME=%T%/cache;XDG_CONFIG_HOME=%T%/conf;HOME=%T%/home;GPY_CONFIG_PATH="

# ============================================================================
# HARNESS
# ============================================================================

set -g GPY_PP_FAILURES 0
set -g GPY_PP_XFAILS 0
set -g GPY_PP_COMPARISONS 0
set -g GPY_PP_FIRED
set -g GPY_PP_MESSAGES

# Failures are accumulated, not printed. Most of them are raised from inside a
# command substitution (`set -l values (__pp_values ...)`), and anything printed
# there is captured as data instead of reaching the terminal. Fish command
# substitutions share the parent's variables, so the list survives; it is
# printed once at the end.
function __pp_fail --description 'Record a parity failure'
    set -g GPY_PP_MESSAGES $GPY_PP_MESSAGES "❌ $argv"
    set -g GPY_PP_FAILURES (math $GPY_PP_FAILURES + 1)
end

# `-u` flags come first: env(1) stops parsing options at the first operand, so
# `env FOO=1 -u BAR cmd` would try to execute `-u`.
function __pp_env_args --description 'Turn a case spec into env(1) arguments'
    set -l assigns $argv
    set -l named
    set -l assign_args
    for assign in $assigns
        test -n "$assign"; or continue
        set -l var (string replace -r '=.*$' '' -- $assign)
        set -l value (string replace -r '^[^=]*=' '' -- $assign)
        set -a named $var
        set -a assign_args "$var="(string replace -a '%T%' $GPY_PP_TMP -- "$value")
    end
    set -l env_args
    for var in $GPY_PP_VARS
        if not contains -- $var $named
            set -a env_args -u $var
        end
    end
    set -a env_args $assign_args
    printf '%s\n' $env_args
end

function __pp_dump --description 'Run one implementation and echo its key=value lines'
    set -l impl $argv[1]
    set -l env_args $argv[2..-1]
    switch $impl
        case rust
            env $env_args $GPY_PP_BIN debug paths --format kv 2>$GPY_PP_ERR
        case fish
            env $env_args fish --no-config -c "source $GPY_PP_ROOT/fish/core/ipc.fish
source $GPY_PP_ROOT/fish/core/util.fish
source $GPY_PP_ROOT/fish/core/debug.fish
__gpy_debug_paths" 2>$GPY_PP_ERR
        case bash
            env $env_args bash --noprofile --norc -c "source $GPY_PP_ROOT/bash/core/ipc.bash; __gpy_debug_paths" 2>$GPY_PP_ERR
        case zsh
            env $env_args zsh -f -c "source $GPY_PP_ROOT/zsh/core/ipc.zsh; __gpy_debug_paths" 2>$GPY_PP_ERR
    end
end

function __pp_values --description 'Validate a dump against the key contract and echo its values'
    set -l case_name $argv[1]
    set -l impl $argv[2]
    set -l lines $argv[3..-1]

    if test (count $lines) -ne (count $GPY_PP_KEYS)
        set -l detail ""
        if test -s $GPY_PP_ERR
            set detail "
     stderr: "(head -3 $GPY_PP_ERR | string join '; ')
        end
        __pp_fail "$case_name/$impl: emitted "(count $lines)" keys, expected "(count $GPY_PP_KEYS)"$detail"
        return 1
    end

    set -l values
    for index in (seq (count $GPY_PP_KEYS))
        set -l expected_key $GPY_PP_KEYS[$index]
        set -l actual_key (string replace -r '=.*$' '' -- $lines[$index])
        if test "$actual_key" != "$expected_key"
            __pp_fail "$case_name/$impl: key $index is '$actual_key', expected '$expected_key' (key set or order drifted)"
            return 1
        end
        set -l value (string replace -r '^[^=]*=' '' -- $lines[$index])
        set value (string replace -a $GPY_PP_TMP '%T%' -- "$value")
        if test -n "$GPY_PP_PWHOME"
            set value (string replace -a $GPY_PP_PWHOME '%PWHOME%' -- "$value")
        end
        set -a values "$value"
    end

    printf '%s\n' $values
end

function __pp_owners --description 'Echo the implementations that own a key'
    set -l key $argv[1]
    for entry in $GPY_PP_OWNERS
        if test (string replace -r ':.*$' '' -- $entry) = "$key"
            string split ',' -- (string replace -r '^[^:]*:' '' -- $entry)
            return 0
        end
    end
    return 1
end

# Named entries win over `*` entries, so `*` reads as "and everywhere else".
# Two named entries claiming the same comparison is ambiguous and reported.
function __pp_divergence --description 'Echo the declared divergences matching case/key/impl'
    set -l case_name $argv[1]
    set -l key $argv[2]
    set -l impl $argv[3]
    set -l named
    set -l fallback
    for entry in $GPY_PP_DIVERGENCES
        set -l fields (string split '^' -- $entry)
        test "$fields[2]" = "$key"; or continue
        test "$fields[3]" = "$impl"; or continue
        if test "$fields[1]" = '*'
            set -a fallback $entry
        else if contains -- $case_name (string split ',' -- $fields[1])
            set -a named $entry
        end
    end
    if test (count $named) -gt 0
        printf '%s\n' $named
    else
        printf '%s\n' $fallback
    end
end

# ============================================================================
# RUN
# ============================================================================

# The relative `.gpy.toml` config candidate and the relative runtime root an
# empty XDG_RUNTIME_DIR produces are both CWD-sensitive, and the agent's
# runtime-root resolver materializes the directory it returns. Run everything
# from a scratch CWD so neither can touch this repo. The one write that escapes
# the scratch tree is `/tmp/gpy` in the `all_unset` case, which is the agent's
# own last-resort runtime root -- exactly what `gpy status` would create there.
cd $GPY_PP_TMP/cwd

for spec in $GPY_PP_CASES
    set -l fields (string split ';' -- $spec)
    set -l case_name $fields[1]
    set -l env_args (__pp_env_args $fields[2..-1])

    # `set x (cmd)` reports set's own status, not the substitution's, so the
    # value count is the reliable signal that a dump was well-formed.
    set -l rust_values (__pp_values $case_name rust (__pp_dump rust $env_args))
    test (count $rust_values) -eq (count $GPY_PP_KEYS); or continue

    for impl in fish bash zsh
        set -l impl_values (__pp_values $case_name $impl (__pp_dump $impl $env_args))
        test (count $impl_values) -eq (count $GPY_PP_KEYS); or continue

        for index in (seq (count $GPY_PP_KEYS))
            set -l key $GPY_PP_KEYS[$index]
            set -l owners (__pp_owners $key)
            set -l rust_value $rust_values[$index]
            set -l impl_value $impl_values[$index]

            # Contract 2: ownership.
            if contains -- $impl $owners
                if test "$impl_value" = '<unimplemented>'
                    __pp_fail "$case_name/$impl/$key: declared an owner but emitted <unimplemented> -- update GPY_PP_OWNERS or restore the resolver"
                    continue
                end
            else
                if test "$impl_value" != '<unimplemented>'
                    __pp_fail "$case_name/$impl/$key: not declared an owner but resolved '$impl_value' -- add $impl to GPY_PP_OWNERS for $key"
                end
                continue
            end
            contains -- rust $owners; or continue

            # Contract 3: divergences.
            set -g GPY_PP_COMPARISONS (math $GPY_PP_COMPARISONS + 1)
            set -l entries (__pp_divergence $case_name $key $impl)
            if test (count $entries) -gt 1
                __pp_fail "$case_name/$impl/$key: "(count $entries)" divergence entries claim this comparison; entries must not overlap"
                continue
            end
            set -l entry $entries[1]

            if test "$rust_value" = "$impl_value"
                if test -n "$entry"
                    __pp_fail "$case_name/$impl/$key: declared divergence now AGREES with the agent ('$rust_value'). Delete the entry, or narrow its case list: $entry"
                end
                continue
            end

            if test -z "$entry"
                __pp_fail "$case_name/$impl/$key: undeclared divergence
     agent: $rust_value
     $impl: $impl_value"
                continue
            end

            set -l fields (string split '^' -- $entry)
            if not string match -q -- $fields[4] $impl_value
                __pp_fail "$case_name/$impl/$key: declared divergence changed shape on the $impl side
     expected glob: $fields[4]
     $impl:         $impl_value"
                continue
            end
            if not string match -q -- $fields[5] $rust_value
                __pp_fail "$case_name/$impl/$key: declared divergence changed shape on the agent side
     expected glob: $fields[5]
     agent:         $rust_value"
                continue
            end

            set -g GPY_PP_XFAILS (math $GPY_PP_XFAILS + 1)
            if not contains -- $entry $GPY_PP_FIRED
                set -a GPY_PP_FIRED $entry
            end
        end
    end
end

cd $GPY_PP_ROOT

# Contract 3, obsolescence half: an entry that never fired is stale.
for entry in $GPY_PP_DIVERGENCES
    if not contains -- $entry $GPY_PP_FIRED
        __pp_fail "obsolete divergence entry never fired anywhere in the matrix -- delete it: $entry"
    end
end

# Contract 1, JSON half: `--format json` is the documented interface (#476), so
# it must carry the same keys in the same order as the kv form the diff uses.
set -l json_keys (env -u GPY_CONFIG_PATH $GPY_PP_BIN debug paths --format json | string match -r '^  "([a-z_]+)":' -g)
if test "$json_keys" != "$GPY_PP_KEYS"
    __pp_fail "--format json key set/order differs from the contract
     json: $json_keys
     kv:   $GPY_PP_KEYS"
else
    echo "✅ --format json emits the contract key set in order"
end

rm -rf $GPY_PP_TMP

for message in $GPY_PP_MESSAGES
    echo $message
end

echo
echo "Cases: "(count $GPY_PP_CASES)"  comparisons: $GPY_PP_COMPARISONS  expected divergences: $GPY_PP_XFAILS"

if test $GPY_PP_FAILURES -gt 0
    echo "❌ $GPY_PP_FAILURES path parity failure(s)"
    exit 1
end

echo "✅ Cross-shell path parity holds (all disagreements declared)"
exit 0
