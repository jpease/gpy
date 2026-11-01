#!/usr/bin/env fish
# tests/fish/directory_truncation_export.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Test: Directory truncation export
# Description: Ensures the theme export does not export the directory settings (GPY_UI_DIRECTORY_*) into the environment; the agent renders the directory from config.toml.
# Prerequisites: gpy-agent binary built (cargo build --release or --debug)
# Run with: fish tests/fish/directory_truncation_export.test.fish

source (dirname (status --filename))/../lib/test_helpers.fish

set -l agent_bin gpy-agent/target/debug/gpy-agent
if not test -x $agent_bin
    fail "gpy-agent binary not found at $agent_bin"
    echo "   Build it first: (cd gpy-agent && cargo build)"
    exit 1
end

set -l temp_root (mktemp -d)
set -l config_dir "$temp_root/gpy"
mkdir -p $config_dir

set -l config_file "$config_dir/config.toml"
printf '[ui]\nshow_icons = false\ntheme = "default"\ndirectory.display = "full"\ndirectory.max_length = 6\nenabled_segments = ["clock","directory"]\n' >$config_file

set -lx MISE_DISABLE 1
set -lx XDG_CONFIG_HOME $temp_root
set -lx HOME $temp_root

set -l export_output ($agent_bin theme export --shell fish | string collect)
if test $status -ne 0
    fail "Failed to run gpy-agent theme export"
    echo $export_output
    exit 1
end

# A stale GPY_UI_DIRECTORY_* in the caller's environment (an older export's
# leftovers) is not the export's doing; start without them.
set -e GPY_UI_DIRECTORY_MAX_LENGTH GPY_UI_DIRECTORY_DISPLAY

source fish/core/init.fish
eval $export_output

for name in GPY_UI_DIRECTORY_MAX_LENGTH GPY_UI_DIRECTORY_DISPLAY
    if set -q $name
        fail "The theme export must not set $name, got '$$name'"
        exit 1
    end
end

if string match -q '*GPY_UI_DIRECTORY*' -- "$export_output"
    fail "The theme export must not mention GPY_UI_DIRECTORY_*"
    exit 1
end

test_finish "Theme export does not export the directory settings"
