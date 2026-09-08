#!/usr/bin/env fish
# tests/fish/directory_truncation_export.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Test: Directory truncation export
# Description: Ensures directory truncation follows GPY_UI_DIRECTORY_MAX_LENGTH exported by gpy-agent theme export.
# Prerequisites: gpy-agent binary built (cargo build --release or --debug)
# Run with: fish tests/fish/directory_truncation_export.test.fish

set -l agent_bin gpy-agent/target/debug/gpy-agent
if not test -x $agent_bin
    echo "❌ gpy-agent binary not found at $agent_bin"
    echo "   Build it first: (cd gpy-agent && RUSTC_WRAPPER= cargo build)"
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

set -l export_output ($agent_bin theme export --format fish | string collect)
if test $status -ne 0
    echo "❌ Failed to run gpy-agent theme export"
    echo $export_output
    exit 1
end

source fish/core/init.fish
eval $export_output

if test "$GPY_UI_DIRECTORY_MAX_LENGTH" != 6
    echo "❌ Expected GPY_UI_DIRECTORY_MAX_LENGTH to be 6 after theme export, got '$GPY_UI_DIRECTORY_MAX_LENGTH'"
    exit 1
end

if test "$GPY_UI_DIRECTORY_DISPLAY" != full
    echo "❌ Expected GPY_UI_DIRECTORY_DISPLAY to be full after theme export, got '$GPY_UI_DIRECTORY_DISPLAY'"
    exit 1
end

echo "✅ Theme export carries the directory truncation settings"
