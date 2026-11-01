#!/usr/bin/env zsh
# tests/zsh/segment_enabled_mirror.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The git and language segments read the theme export's config mirrors,
# __gpy_cfg_git_enabled / __gpy_cfg_language_enabled, and nothing else: a
# stale GPY_GIT_ENABLED=1 / GPY_LANGUAGE_ENABLED=1 in the environment is
# ignored. Twins in tests/bash and tests/fish.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

TMP=$(mktemp -d "${TMPDIR:-/tmp}/gpy-seg-mirror.XXXXXX")
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/repo"
git -C "$TMP/repo" init -q -b main

source zsh/core/constants.zsh
source zsh/core/ipc.zsh
source zsh/segments/git.zsh
source zsh/segments/language.zsh

__enabled_segments=(git language)
cd "$TMP/repo" || exit 1

# A stale value of the old exported name must change nothing.
export GPY_GIT_ENABLED=1 GPY_LANGUAGE_ENABLED=1

# status_of DETECT: 0 shown, 1 hidden.
status_of() {
    "$1"
    print -r -- $?
}

unset __gpy_cfg_git_enabled __gpy_cfg_language_enabled
check "git: no mirror and a git repo as PWD: shown" 0 "$(status_of __gpy_segment_git_detect)"
check "language: no mirror: shown" 0 "$(status_of __gpy_segment_language_detect)"

__gpy_cfg_git_enabled=0
__gpy_cfg_language_enabled=0
check "git: __gpy_cfg_git_enabled=0 hides it despite GPY_GIT_ENABLED=1" 1 "$(status_of __gpy_segment_git_detect)"
check "language: __gpy_cfg_language_enabled=0 hides it despite GPY_LANGUAGE_ENABLED=1" 1 "$(status_of __gpy_segment_language_detect)"

__gpy_cfg_git_enabled=1
__gpy_cfg_language_enabled=1
check "git: __gpy_cfg_git_enabled=1 shows it" 0 "$(status_of __gpy_segment_git_detect)"
check "language: __gpy_cfg_language_enabled=1 shows it" 0 "$(status_of __gpy_segment_language_detect)"

# An old exported 0 is not a switch either.
unset __gpy_cfg_git_enabled __gpy_cfg_language_enabled
export GPY_GIT_ENABLED=0 GPY_LANGUAGE_ENABLED=0
check "git: GPY_GIT_ENABLED=0 alone does not hide it" 0 "$(status_of __gpy_segment_git_detect)"
check "language: GPY_LANGUAGE_ENABLED=0 alone does not hide it" 0 "$(status_of __gpy_segment_language_detect)"

test_finish "git and language segments read only the config mirrors"
