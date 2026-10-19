#!/usr/bin/env fish
# tests/fish/install_completions_wiring.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Static/text-level guard: the installers (install.sh and install-oneline.sh, via
# `gpy setup`) must wire up Fish shell completions
# for the `gpy` CLI when installing for Fish:
#   1. Regenerate the STRUCTURAL completions (`gpy completions fish`) into
#      the fish completions autoload directory on every run, guarded so a
#      missing/non-executable gpy binary warns and skips instead of failing
#      the install (the same optional-CLI handling the installers use).
#   2. Append the checked-in `fish/completions/gpy-dynamic.fish` dynamic-
#      value glue to that same `gpy.fish`: fish autoloads
#      `completions/<command>.fish` only for the matching command, so a
#      separate gpy-dynamic.fish is never loaded.
#
# In the same spirit as install_sh_cli_binary.test.fish / install_oneline_cli_binary.test.fish,
# this parses the installer scripts as text rather than running a full
# install against a real $HOME (which would modify the user's live Fish
# config and restart the agent -- not something a test suite should do).

set -l script_dir (dirname (status --filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

set -g failed 0

function __icw_check --argument-names label file pattern
    if grep -qE -- "$pattern" $file
        echo "✅ $label"
    else
        echo "❌ $label"
        echo "   file:    $file"
        echo "   pattern: $pattern"
        set -g failed 1
    end
end

# --- fish/completions/gpy-dynamic.fish must exist and be the checked-in glue ---
if test -f fish/completions/gpy-dynamic.fish
    echo "✅ fish/completions/gpy-dynamic.fish exists"
else
    echo "❌ fish/completions/gpy-dynamic.fish is missing"
    set -g failed 1
end

# --- install.sh ---
# Like the one-line installer, it runs `gpy setup` (which generates the
# structural completions and appends the glue; see
# gpy-agent/tests/setup_command_tests.rs).
__icw_check "install.sh: hands the package's shell trees to gpy setup" install.sh \
    '~/\.local/bin/gpy setup --from'

# --- install-oneline.sh ---
# The one-line installer no longer writes completions itself: it stages the
# glue and runs `gpy setup`, which generates the structural completions and
# appends the glue (covered by gpy-agent/tests/setup_command_tests.rs).
__icw_check "install-oneline.sh: downloads the gpy-shell-files.tar.gz archive" install-oneline.sh \
    'gpy-shell-files\.tar\.gz'
__icw_check "install-oneline.sh: hands the staged files to gpy setup" install-oneline.sh \
    '"\$INSTALL_DIR/gpy" setup --shell "\$CURRENT_SHELL" --from "\$STAGE_DIR"'

if test $failed -eq 1
    exit 1
end

echo "✅ All installers wire up Fish completions for the gpy CLI"
