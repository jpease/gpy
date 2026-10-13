#!/usr/bin/env bash
# tests/bash/install_docs_env_placement.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Installer env overrides must sit on the `sh` side of the pipe.
# `VAR=x curl ... | sh` sets VAR for curl only, so the installer never sees
# GPY_SHELL / GPY_VERSION / GPY_NERD_FONT. The correct shape is
# `curl ... | VAR=x sh`. No GPY_*= assignment may directly precede curl in
# README, docs, scripts, install-oneline.sh or install.sh.

set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

hits=""
while IFS= read -r f; do
    case "$f" in
        *.sh) out="$(grep -vE '^[[:space:]]*#' "$f" | grep -nE 'GPY_[A-Z_]+=[^ ]+ +curl' | sed "s#^#$f:#")" ;;
        *) out="$(grep -nE 'GPY_[A-Z_]+=[^ ]+ +curl' "$f" | sed "s#^#$f:#")" ;;
    esac
    [[ -n "$out" ]] && hits+="$out"$'\n'
done < <(find README.md docs scripts install-oneline.sh install.sh \
    -type f \( -name '*.md' -o -name '*.sh' \) 2>/dev/null)
[[ -z "$hits" ]] || fail "env override placed before curl (applies to curl, not sh):"$'\n'"$hits"

test_finish "installer env overrides are on the sh side of the pipe"
