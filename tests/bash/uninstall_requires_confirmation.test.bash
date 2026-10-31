#!/usr/bin/env bash
# tests/bash/uninstall_requires_confirmation.test.bash
# scripts/uninstall.sh deletes the user's config, themes and palettes, so an
# EOF on stdin (closed, </dev/null, piped script) must not count as consent:
# it exits non-zero and removes nothing. `--yes` is the explicit opt-in, and
# an unknown argument is refused before anything is touched.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

WORKDIR="$(mktemp -d)"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

# A sandboxed install holding user-authored files; prints the sandbox root.
make_sandbox() {
    local sb
    sb="$(mktemp -d "$WORKDIR/sb.XXXXXX")"
    mkdir -p "$sb/home" "$sb/config/gpy/themes" "$sb/config/gpy/palettes" "$sb/cache" "$sb/run"
    printf '[ui]\ntheme = "mine"\n' >"$sb/config/gpy/config.toml"
    printf '# hand-written theme\n' >"$sb/config/gpy/themes/mine.toml"
    printf '# hand-written palette\n' >"$sb/config/gpy/palettes/mine.toml"
    mkdir -p "$sb/config/gpy/plugins/demo"
    printf '# hand-written plugin\n' >"$sb/config/gpy/plugins/demo/plugin.toml"
    printf '%s' "$sb"
}

# run_uninstall SANDBOX STDIN_FILE [ARGS...]: exit status of the uninstaller.
run_uninstall() {
    local sb="$1" input="$2"
    shift 2
    env -u ZDOTDIR HOME="$sb/home" XDG_CONFIG_HOME="$sb/config" XDG_CACHE_HOME="$sb/cache" \
        XDG_RUNTIME_DIR="$sb/run" sh "$ROOT/scripts/uninstall.sh" "$@" <"$input" >"$sb/uninstall.log" 2>&1
}

user_files_intact() {
    [[ -f "$1/config/gpy/config.toml" && -f "$1/config/gpy/themes/mine.toml" &&
        -f "$1/config/gpy/palettes/mine.toml" && -f "$1/config/gpy/plugins/demo/plugin.toml" ]]
}

sb="$(make_sandbox)"
run_uninstall "$sb" /dev/null
rc=$?
[[ $rc -ne 0 ]] || fail "EOF on stdin: uninstaller exited 0"
user_files_intact "$sb" || fail "EOF on stdin: user config, themes or palettes were removed"

sb="$(make_sandbox)"
run_uninstall "$sb" /dev/null --bogus
rc=$?
[[ $rc -eq 2 ]] || fail "unknown argument: expected exit 2, got $rc"
user_files_intact "$sb" || fail "unknown argument: user files were removed"

sb="$(make_sandbox)"
run_uninstall "$sb" /dev/null --yes
rc=$?
[[ $rc -eq 0 ]] || fail "--yes: uninstaller exited $rc"
user_files_intact "$sb" || fail "--yes: user config, themes, palettes or plugins were removed"

sb="$(make_sandbox)"
printf '\n' >"$WORKDIR/enter"
run_uninstall "$sb" "$WORKDIR/enter"
rc=$?
[[ $rc -eq 0 ]] || fail "Enter: uninstaller exited $rc"
user_files_intact "$sb" || fail "Enter: user config, themes, palettes or plugins were removed"

sb="$(make_sandbox)"
run_uninstall "$sb" /dev/null --yes --purge-config
rc=$?
[[ $rc -eq 0 ]] || fail "--purge-config: uninstaller exited $rc"
[[ ! -e "$sb/config/gpy" ]] || fail "--purge-config: config directory still present"

sb="$(make_sandbox)"
rm -rf "$sb/config/gpy"
mkdir -p "$sb/config/gpy/bash" "$sb/config/gpy/zsh" "$sb/config/gpy/bundled-plugins"
run_uninstall "$sb" /dev/null --yes
rc=$?
[[ $rc -eq 0 ]] || fail "GPY-only config dir: uninstaller exited $rc"
[[ ! -e "$sb/config/gpy" ]] || fail "GPY-only config dir: empty config directory still present"

sb="$(make_sandbox)"
rm -rf "$sb/config/gpy"
run_uninstall "$sb" /dev/null --yes
rc=$?
[[ $rc -eq 0 ]] || fail "no config dir: uninstaller exited $rc"
grep -q 'Configuration directory not found' "$sb/uninstall.log" || fail "no config dir: missing 'not found' message"
! grep -qE 'Kept (your )?configuration' "$sb/uninstall.log" || fail "no config dir: unexpected Kept line"

if [[ $failures -ne 0 ]]; then
    for log in "$WORKDIR"/sb.*/uninstall.log; do
        echo "--- $log ---"
        cat "$log"
    done
fi
test_finish "uninstall requires confirmation"
