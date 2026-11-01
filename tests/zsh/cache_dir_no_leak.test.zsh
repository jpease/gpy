#!/usr/bin/env zsh
# tests/zsh/cache_dir_no_leak.test.zsh
#
# Zsh renders the prompt in-process and keeps its memos in shell variables:
# loading GPY and rendering several prompts creates nothing under $TMPDIR,
# and `__gpy_cache_dir` no longer exists. Children never start an agent: none
# is on their PATH. Paths reach children only as environment/argv.

ROOT=${0:a:h:h:h}
SANDBOX=$(mktemp -d "${TMPDIR:-/tmp}/gpy-cleak.XXXXXX") || exit 1
trap 'rm -rf "$SANDBOX"' EXIT
mkdir -p "$SANDBOX/tmp" "$SANDBOX/zdot" "$SANDBOX/d1" "$SANDBOX/d2"
print -r -- 'source "$GPY_TEST_ROOT/zsh/gpy.zsh"' >"$SANDBOX/zdot/.zshrc"

emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'

# child_env <cmd...>: run a command in a scrubbed environment.
child_env() {
    env -i PATH=/usr/bin:/bin TERM=dumb HOME="$SANDBOX" ZDOTDIR="$SANDBOX/zdot" \
        TMPDIR="$SANDBOX/tmp" XDG_CONFIG_HOME="$SANDBOX/cfg" \
        XDG_CACHE_HOME="$SANDBOX/cache" XDG_RUNTIME_DIR="$SANDBOX/run" \
        GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock" \
        GPY_TEST_ROOT="$ROOT" "$@"
}

entries() { local -a e; e=("$SANDBOX/tmp"/*(N) "$SANDBOX/tmp"/.*(N)); print -r -- ${#e}; }

# Several prompts: each command line returns to precmd and a render.
out=$(print -r -- $'true\ncd "$SANDBOX_D1"\ntrue\ncd "$SANDBOX_D2"\nfalse\ntrue\nprint -r -- "cache_dir_set=${+__gpy_cache_dir}"\nexit' \
    | SANDBOX_D1="$SANDBOX/d1" SANDBOX_D2="$SANDBOX/d2" child_env zsh -i 2>/dev/null)
[[ $out == *"cache_dir_set=0"* ]] && pass "__gpy_cache_dir is unset" \
    || fail "__gpy_cache_dir is set after loading GPY"

n=$(entries)
[[ $n == 0 ]] && pass "no file or directory under TMPDIR after load and several prompts" \
    || fail "$n entr(ies) left under TMPDIR: $(print -r -- "$SANDBOX/tmp"/*(N))"

print -r -- 'exit' | child_env zsh -i >/dev/null 2>&1
[[ $(entries) == 0 ]] && pass "an immediate exit leaves nothing under TMPDIR" \
    || fail "an immediate exit left entries under TMPDIR"

print -r -- $'source "$GPY_TEST_ROOT/zsh/gpy.zsh"\nexec zsh -i\nexit' | child_env zsh -i >/dev/null 2>&1
[[ $(entries) == 0 ]] && pass "re-source and exec leave nothing under TMPDIR" \
    || fail "re-source and exec left entries under TMPDIR"

test_finish
