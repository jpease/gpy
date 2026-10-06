#!/usr/bin/env bash
# tests/bash/install_docs_env_placement.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Installer env overrides must sit on the `sh` side of the pipe (#745).
# `VAR=x curl ... | sh` sets VAR for curl only, so the installer never sees
# GPY_SHELL / GPY_VERSION / GPY_NERD_FONT. The correct shape is
# `curl ... | VAR=x sh`.
#
# Asserts:
#   (a) no GPY_*= assignment directly precedes curl in README, docs (minus
#       docs/archive), scripts, install-oneline.sh or install.sh
#   (b) install-oneline.sh's pin-a-version hint uses the `| GPY_VERSION=` form

set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

failures=0
fail() {
    echo "FAIL: $*"
    failures=$((failures + 1))
}

hits=""
while IFS= read -r f; do
    case "$f" in
        docs/archive/*) continue ;;
        *.sh) out="$(grep -vE '^[[:space:]]*#' "$f" | grep -nE 'GPY_[A-Z_]+=[^ ]+ +curl' | sed "s#^#$f:#")" ;;
        *) out="$(grep -nE 'GPY_[A-Z_]+=[^ ]+ +curl' "$f" | sed "s#^#$f:#")" ;;
    esac
    [[ -n "$out" ]] && hits+="$out"$'\n'
done < <(find README.md docs scripts install-oneline.sh install.sh \
    -type f \( -name '*.md' -o -name '*.sh' \) 2>/dev/null)
[[ -z "$hits" ]] || fail "env override placed before curl (applies to curl, not sh):"$'\n'"$hits"

grep -qF '| GPY_VERSION=' install-oneline.sh \
    || fail "install-oneline.sh pin hint lacks '| GPY_VERSION=... sh'"

if ((failures > 0)); then
    echo "$failures failure(s)"
    exit 1
fi
echo "PASS: installer env overrides are on the sh side of the pipe"
