#!/usr/bin/env zsh
# tests/zsh/e2e_lang_deferred_version_probe.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A language version probe slower than the request budget is deferred
# (ADR-0009): the `lang` request gives up within `agent.timeout_seconds`, and
# the probe's result reaches the idle Zsh prompt through the repaint doorbell.
# The agent runs with a PATH holding only a fake `rustc` that takes 3 s, so
# no real toolchain or mise answers first, and a 1 s request budget. With no
# keystroke sent:
#   (a) the agent logs the version probe outlasting the request,
#   (b) the idle prompt shows the probed version.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'

shell_e2e_init "$ROOT"

printf '[ui]\nshow_icons = false\ntheme = "text"\nenabled_segments = ["directory", "language"]\n\n[agent]\ntimeout_seconds = 1\n' \
    >"$XDG_CONFIG_HOME/gpy/config.toml"

mkdir -p "$SHELL_E2E_REPO/src" "$SHELL_E2E_ROOT/fakebin"
printf '[package]\nname = "gpy-e2e-test"\nversion = "0.1.0"\n' >"$SHELL_E2E_REPO/Cargo.toml"
printf 'fn main() {}\n' >"$SHELL_E2E_REPO/src/main.rs"
printf '#!/bin/sh\nsleep 3\necho "rustc 1.97.3 (fake slow e2e binary)"\n' >"$SHELL_E2E_ROOT/fakebin/rustc"
chmod +x "$SHELL_E2E_ROOT/fakebin/rustc"

# Only the agent gets the restricted PATH; the shell keeps the caller's.
PATH="$SHELL_E2E_ROOT/fakebin:/usr/bin:/bin" shell_e2e_start_agent || exit 1
export GPY_AGENT_ENABLED=1
export GPY_AGENT_SUPERVISOR_ENABLED=1
cd "$SHELL_E2E_REPO" || exit 1
shell_e2e_spawn_client zsh "$ROOT" || { shell_e2e_dump_transcript; exit 1; }

timed_out() { grep -q 'Language version probe timed out' "$GPY_DEBUG_LOG" 2>/dev/null; }
if shell_e2e_poll 10 timed_out; then
    pass "the lang request gave up on the slow probe within its budget"
else
    fail "no 'Language version probe timed out' in the agent log"
fi

if shell_e2e_wait_for '1\.97\.3' 15 >/dev/null; then
    pass "the idle prompt shows the deferred probe's version, no keystroke sent"
else
    fail "the probed version never reached the idle prompt"
fi

shell_e2e_send 'exit\r'
if [ "$failures" -gt 0 ]; then
    shell_e2e_dump_transcript
fi
test_finish "a slow version probe is deferred and repaints the idle zsh prompt"
