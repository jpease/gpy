#!/usr/bin/env fish
# tests/fish/install_completions_wiring.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Static/text-level guard for #329 (Epic #326, task 3/6): all three
# installers (install.sh, install-oneline.sh, install-dev.fish) must wire up
# Fish shell completions for the `gpy` CLI when installing for Fish:
#   1. Regenerate the STRUCTURAL completions (`gpy completions fish`) into
#      the fish completions autoload directory on every run, guarded so a
#      missing/non-executable gpy binary warns and skips instead of failing
#      the install (mirrors the optional-CLI handling from #327).
#   2. Append the checked-in `fish/completions/gpy-dynamic.fish` dynamic-
#      value glue to that same `gpy.fish`: fish autoloads
#      `completions/<command>.fish` only for the matching command, so a
#      separate gpy-dynamic.fish is never loaded (#702).
#
# In the same spirit as install_oneline_file_lists.test.fish (#308) and
# install_sh_cli_binary.test.fish / install_oneline_cli_binary.test.fish
# (#327), this parses the installer scripts as text rather than running a
# full install against a real $HOME (which would modify the user's live
# Fish config and restart the agent -- not something a test suite should
# do).

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

# ---------------------------------------------------------------------------
# fish/completions/gpy-dynamic.fish must exist and be the checked-in glue
# ---------------------------------------------------------------------------
if test -f fish/completions/gpy-dynamic.fish
    echo "✅ fish/completions/gpy-dynamic.fish exists"
else
    echo "❌ fish/completions/gpy-dynamic.fish is missing"
    set -g failed 1
end

# ---------------------------------------------------------------------------
# install.sh
# ---------------------------------------------------------------------------
__icw_check "install.sh: regenerates structural completions" install.sh \
    'gpy completions fish.*>.*FISH_CONFIG_DIR/completions/gpy\.fish'
__icw_check "install.sh: guards on the CLI binary being executable" install.sh \
    '\[ -x ~/\.local/bin/gpy \]'
__icw_check "install.sh: appends gpy-dynamic.fish to completions/gpy.fish" install.sh \
    'completions/gpy-dynamic\.fish" *>>.*FISH_CONFIG_DIR/completions/gpy\.fish'

# ---------------------------------------------------------------------------
# install-oneline.sh
# ---------------------------------------------------------------------------
__icw_check "install-oneline.sh: regenerates structural completions" install-oneline.sh \
    'completions fish.*>.*FISH_COMPLETIONS_DIR/gpy\.fish'
__icw_check "install-oneline.sh: guards on the CLI binary being executable" install-oneline.sh \
    '\[ -x "\$INSTALL_DIR/gpy" \]'
__icw_check "install-oneline.sh: downloads gpy-dynamic.fish from SHELL_FILES_BASE_URL" install-oneline.sh \
    'SHELL_FILES_BASE_URL/completions/gpy-dynamic\.fish'
__icw_check "install-oneline.sh: appends the glue to completions/gpy.fish" install-oneline.sh \
    '>>.*FISH_COMPLETIONS_DIR/gpy\.fish'

# ---------------------------------------------------------------------------
# install-dev.fish
# ---------------------------------------------------------------------------
__icw_check "install-dev.fish: regenerates structural completions" install-dev.fish \
    'gpy_cli completions fish.*>.*fish_completions_dir/gpy\.fish'
__icw_check "install-dev.fish: guards on the CLI binary being executable" install-dev.fish \
    'test -x \$dev_gpy_cli'
__icw_check "install-dev.fish: appends gpy-dynamic.fish to completions/gpy.fish" install-dev.fish \
    'cat fish/completions/gpy-dynamic\.fish >>.*fish_completions_dir/gpy\.fish'

if test $failed -eq 1
    exit 1
end

echo "✅ All installers wire up Fish completions for the gpy CLI"
