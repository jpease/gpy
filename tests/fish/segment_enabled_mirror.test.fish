#!/usr/bin/env fish
# tests/fish/segment_enabled_mirror.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The git and language segments read the theme export's config mirrors,
# __gpy_cfg_git_enabled / __gpy_cfg_language_enabled, and nothing else: a
# stale GPY_GIT_ENABLED=1 / GPY_LANGUAGE_ENABLED=1 in the environment is
# ignored. Twins in tests/bash and tests/zsh.

source (dirname (status --filename))/../lib/test_helpers.fish

set -l temp_home (mktemp -d)
set -gx HOME $temp_home
set -gx XDG_CONFIG_HOME $temp_home/.config
set -gx XDG_CACHE_HOME $temp_home/.cache
set -gx GPY_AGENT_SUPERVISOR_ENABLED 0
mkdir -p $XDG_CONFIG_HOME/gpy $temp_home/repo
git -C $temp_home/repo init -q -b main

source fish/core/init.fish
source fish/segments/git.fish
source fish/segments/language.fish

set -g __enabled_segments git language
cd $temp_home/repo

# A stale value of the old exported name must change nothing.
set -gx GPY_GIT_ENABLED 1
set -gx GPY_LANGUAGE_ENABLED 1

# status_of DETECT: 0 shown, 1 hidden.
function status_of --argument-names detect
    $detect
    echo $status
end

set -e __gpy_cfg_git_enabled __gpy_cfg_language_enabled
check "git: no mirror and a git repo as PWD: shown" 0 (status_of segment_git_detect)
check "language: no mirror: shown" 0 (status_of segment_language_detect)

set -g __gpy_cfg_git_enabled 0
set -g __gpy_cfg_language_enabled 0
check "git: __gpy_cfg_git_enabled=0 hides it despite GPY_GIT_ENABLED=1" 1 (status_of segment_git_detect)
check "language: __gpy_cfg_language_enabled=0 hides it despite GPY_LANGUAGE_ENABLED=1" 1 (status_of segment_language_detect)

set -g __gpy_cfg_git_enabled 1
set -g __gpy_cfg_language_enabled 1
check "git: __gpy_cfg_git_enabled=1 shows it" 0 (status_of segment_git_detect)
check "language: __gpy_cfg_language_enabled=1 shows it" 0 (status_of segment_language_detect)

# An old exported 0 is not a switch either.
set -e __gpy_cfg_git_enabled __gpy_cfg_language_enabled
set -gx GPY_GIT_ENABLED 0
set -gx GPY_LANGUAGE_ENABLED 0
check "git: GPY_GIT_ENABLED=0 alone does not hide it" 0 (status_of segment_git_detect)
check "language: GPY_LANGUAGE_ENABLED=0 alone does not hide it" 0 (status_of segment_language_detect)

test_finish "git and language segments read only the config mirrors"
