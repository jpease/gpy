#!/usr/bin/env fish
# SPDX-License-Identifier: GPL-3.0-or-later
# tests/fish/script_runner_symlinked_tree.test.fish
#
# When the Fish tree is a symlink to a directory that has no sibling bash/,
# __gpy_g_resolve_runner still finds bash/core/g_runner.bash beside the link:
# the lookup joins the paths lexically (`path normalize`), not through the
# kernel's physical `..`.

source (dirname (status --filename))/../lib/test_helpers.fish

set -g scratch (realpath (mktemp -d))
test_track_dir $scratch

mkdir -p $scratch/data/bash/core $scratch/dot/fish-real/core
cp $__gpy_root/fish/core/script.fish $scratch/dot/fish-real/core/script.fish
cp $__gpy_root/bash/core/g_runner.bash $scratch/data/bash/core/g_runner.bash
ln -s $scratch/dot/fish-real $scratch/data/fish

set -g __gpy_core_dir $scratch/data/fish/core
source $scratch/data/fish/core/script.fish
__gpy_g_resolve_runner 2>/dev/null

check "a symlinked fish tree finds the sibling bash/core runner" \
    $scratch/data/bash/core/g_runner.bash "$__gpy_g_runner_file"
check "the runner resolved from the symlinked tree is ready" ready "$__gpy_g_runner_state"

test_finish "script runner lookup through a symlinked tree"
