#!/usr/bin/env bash
# tests/bash/bash_limitations_claims.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Tripwire for the documented shell-support claims (#647).
#
# The Bash/Zsh E2E tier now measures what each shell actually does (idle
# repaint, re-registration, oneshot recovery). The docs that describe those
# results must keep saying what the tests prove, so this pins every row of
# the feature matrix in docs/user/bash-limitations.md, the shell comparison
# rows in docs/user/troubleshooting.md that the E2E tests back, and the
# minimum shell versions in docs/INSTALL.md, verbatim. A silent support
# change (a matrix cell flipped, a minimum bumped, a limitation reworded
# away) fails here and forces the doc and the test that measures it to move
# together.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
pass() { echo "PASS: $*"; }

pin() {
    file="$1" text="$2"
    if grep -qF -- "$text" "$file"; then
        pass "$file: $text"
    else
        fail "$file no longer contains: $text"
    fi
}

echo "--- docs/user/bash-limitations.md feature matrix ---"
MATRIX="docs/user/bash-limitations.md"
pin "$MATRIX" "| Feature | Fish | Zsh | Bash 5 | Bash 4 | Bash 3 |"
pin "$MATRIX" "| Git status | ✅ | ✅ | ✅ | ✅ | ✅ |"
pin "$MATRIX" "| Language detection | ✅ | ✅ | ✅ | ✅ | ✅ |"
pin "$MATRIX" "| Status indicator | ✅ | ✅ | ✅ | ✅ | ✅ |"
pin "$MATRIX" "| Clock | ✅ | ✅ | ✅ | ✅ | ✅ |"
pin "$MATRIX" "| Directory | ✅ | ✅ | ✅ | ✅ | ✅ |"
pin "$MATRIX" "| **Duration** | ✅ | ✅ | ✅ | ⚠️ | ❌ |"
pin "$MATRIX" "| **Live updates (SIGUSR1)** | ✅ idle prompt repaints | ✅ idle prompt repaints | ⚠️ shown at next prompt | ⚠️ shown at next prompt | ⚠️ shown at next prompt |"
pin "$MATRIX" "| Config hot-reload (SIGUSR2) | ✅ | ✅ | ✅ | ✅ | ✅ |"
pin "$MATRIX" "| Agent IPC | ✅ | ✅ | ✅ | ✅ | ✅ |"
pin "$MATRIX" "| Transient prompt | ✅ | ✅ | ❌ | ❌ | ❌ |"
# The measured Bash limitation, in the words the E2E test cites.
pin "$MATRIX" "An idle Bash prompt does not repaint by itself."
pin "$MATRIX" "tests/bash/e2e_git_live_content.test.bash"

echo "--- docs/user/troubleshooting.md shell comparison ---"
COMPARE="docs/user/troubleshooting.md"
pin "$COMPARE" "| Live SIGUSR1 updates | Full: idle prompt repaints in place | Full: idle prompt repaints in place (re-rendered in \`TRAPUSR1\`, #637) | Shown at the next prompt: readline cannot repaint an idle prompt (measured, see [Bash Limitations](bash-limitations.md#2-live-updates-sigusr1)) | Same as Bash 5.x |"
pin "$COMPARE" "| Re-registers after an agent restart | Yes (SIGALRM) | Yes (SIGALRM, #638) | Yes (SIGALRM, #638) | Yes |"
pin "$COMPARE" "tests/zsh/e2e_git_live_content.test.zsh"
pin "$COMPARE" "tests/zsh/e2e_reregister_after_restart.test.zsh"

echo "--- docs/INSTALL.md minimum shell versions ---"
INSTALL="docs/INSTALL.md"
pin "$INSTALL" "| Fish | 3.6+ |"
pin "$INSTALL" "| Zsh | 5.8+ |"
pin "$INSTALL" "| Bash | 4.0+ (5.0+ recommended"

# The tests the claims cite must exist.
for t in tests/bash/e2e_git_live_content.test.bash tests/zsh/e2e_git_live_content.test.zsh \
    tests/zsh/e2e_reregister_after_restart.test.zsh tests/bash/e2e_reregister_after_restart.test.bash; do
    if [ -f "$t" ]; then
        pass "$t exists"
    else
        fail "cited test $t is missing"
    fi
done

if [ "$failures" -gt 0 ]; then
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: documented shell-support claims match the measured tests"
