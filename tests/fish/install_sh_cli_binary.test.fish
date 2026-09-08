#!/usr/bin/env fish
# tests/fish/install_sh_cli_binary.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression test for #327: the user-facing `gpy` CLI binary must ship
# through install.sh alongside the `gpy-agent` daemon. Before this change
# install.sh only installed gpy-agent, so shell completions (which call the
# `gpy` CLI) had no binary to invoke after a local-package install.
#
# Static/text-level check in the same spirit as
# install_oneline_file_lists.test.fish (#308): it parses install.sh rather
# than running a full install, asserting that install.sh (1) computes a
# per-platform `gpy-<platform>-<arch>` CLI binary name for every platform it
# already handles for the agent, and (2) installs that binary to
# ~/.local/bin/gpy.

set -l script_dir (dirname (status --filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

set -l installer install.sh
set -g failed 0

# 1. A CLI_BINARY variable must be assigned the parallel gpy-<platform>-<arch>
#    names for every platform/arch the agent BINARY is assigned. We assert the
#    concrete names rather than the assignment mechanism so a reformat of the
#    case statement doesn't break the test.
set -l expected_cli_names \
    gpy-linux-x86_64 \
    gpy-linux-aarch64 \
    gpy-macos-x86_64 \
    gpy-macos-aarch64 \
    gpy-windows-x86_64.exe

for name in $expected_cli_names
    if grep -qF -- "CLI_BINARY=\"$name\"" $installer
        echo "✅ $installer: assigns CLI_BINARY=\"$name\""
    else
        echo "❌ $installer: missing CLI_BINARY=\"$name\""
        set -g failed 1
    end
end

# 2. The CLI binary must be sourced from bin/$CLI_BINARY (parallel to the
#    agent's bin/$BINARY) and installed to ~/.local/bin/gpy.
if grep -qF -- 'bin/$CLI_BINARY' $installer
    echo "✅ $installer: references bin/\$CLI_BINARY source path"
else
    echo "❌ $installer: does not reference bin/\$CLI_BINARY"
    set -g failed 1
end

if grep -qE -- '~/\.local/bin/gpy([^-a-zA-Z]|$)' $installer
    echo "✅ $installer: installs to ~/.local/bin/gpy"
else
    echo "❌ $installer: does not install to ~/.local/bin/gpy"
    set -g failed 1
end

if test $failed -eq 1
    exit 1
end

echo "✅ install.sh installs the gpy CLI binary alongside gpy-agent"
