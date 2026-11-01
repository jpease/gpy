#!/usr/bin/env bash
# tests/bash/export_no_env_leak.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The theme export never puts a GPY_* name (or a __gpy_* mirror) into the
# environment: for each shell, source the real export in a sandbox and run
# `env` in a child process. Only the variable this test set may show.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

shell_e2e_init "$ROOT"

# Every config mirror gets a non-default value so it would show up if exported.
printf '[agent]\nenabled = false\n\n[agent.supervisor]\nenabled = false\ncheck_interval_seconds = 45\nmax_restart_attempts = 9\n\n[git]\nenabled = false\n\n[language]\nenabled = false\n\n[ui]\ntransient_prompt = false\n' \
    >"$XDG_CONFIG_HOME/gpy/config.toml"

EXPORT_DIR="$SHELL_E2E_ROOT/exports"
mkdir -p "$EXPORT_DIR"

# env_of SHELL FILE: the environment a child process of SHELL sees after it
# sourced FILE, with a clean parent environment plus one marker.
env_of() {
    # shellcheck disable=SC2016 # expands in the child
    case "$1" in
        bash) env -i PATH="$PATH" HOME="$HOME" GPY_TEST_MARKER=1 bash --noprofile --norc -c 'source "$1"; env' _ "$2" ;;
        zsh) env -i PATH="$PATH" HOME="$HOME" GPY_TEST_MARKER=1 zsh -f -c 'source "$1"; env' _ "$2" ;;
        fish) env -i PATH="$PATH" HOME="$HOME" GPY_TEST_MARKER=1 fish --no-config -c 'source $argv[1]; env' -- "$2" ;;
    esac
}

for sh in bash zsh fish; do
    if ! command -v "$sh" >/dev/null 2>&1; then
        test_skip_part "$sh is not installed"
        continue
    fi
    file="$EXPORT_DIR/export.$sh"
    if ! "$SHELL_E2E_AGENT_BIN" theme export --shell "$sh" >"$file" 2>"$EXPORT_DIR/export.$sh.err"; then
        fail "$sh: theme export failed: $(cat "$EXPORT_DIR/export.$sh.err")"
        continue
    fi
    # The mirrors really are in the file, so the empty result below means something.
    for mirror in agent_enabled supervisor_enabled supervisor_check_interval supervisor_max_restarts git_enabled language_enabled transient_prompt; do
        grep -q "__gpy_cfg_$mirror" "$file" || fail "$sh: the export lacks __gpy_cfg_$mirror"
    done
    seen="$(env_of "$sh" "$file" | sed -n 's/^\(GPY_[A-Za-z0-9_]*\)=.*/\1/p; s/^\(__gpy_[A-Za-z0-9_]*\)=.*/\1/p' | sort | paste -sd' ' -)"
    check "$sh: the child's environment has only the test's own GPY_ variable" "GPY_TEST_MARKER" "$seen"
done

test_finish "the theme export leaks no GPY_* name into child processes"
