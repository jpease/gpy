#!/usr/bin/env fish
# tests/fish/install_oneline_cli_binary.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression test for #327: install-oneline.sh must download and install the
# user-facing `gpy` CLI binary alongside the `gpy-agent` daemon, so a
# `curl | sh` install leaves a working `gpy` on PATH for shell completions.
#
# Static/text-level check in the same spirit as
# install_oneline_file_lists.test.fish (#308): it parses install-oneline.sh
# rather than running a network install, asserting that the script (1)
# computes a per-platform `gpy-<platform>-<arch>` CLI binary name for every
# platform it already handles for the agent, and (2) installs that binary to
# $INSTALL_DIR/gpy.

set -l script_dir (dirname (status --filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

set -l installer install-oneline.sh
set -g failed 0

# 1. A CLI_BINARY_NAME variable must be assigned the parallel
#    gpy-<platform>-<arch> names for every platform/arch the agent
#    BINARY_NAME is assigned.
set -l expected_cli_names \
    gpy-linux-x86_64 \
    gpy-linux-aarch64 \
    gpy-macos-x86_64 \
    gpy-macos-aarch64

for name in $expected_cli_names
    if grep -qF -- "CLI_BINARY_NAME=\"$name\"" $installer
        echo "✅ $installer: assigns CLI_BINARY_NAME=\"$name\""
    else
        echo "❌ $installer: missing CLI_BINARY_NAME=\"$name\""
        set -g failed 1
    end
end

# 2. The CLI binary must be downloaded (its URL references $CLI_BINARY_NAME)
#    and installed to $INSTALL_DIR/gpy.
if grep -qF -- '$CLI_BINARY_NAME' $installer
    echo "✅ $installer: builds a download URL from \$CLI_BINARY_NAME"
else
    echo "❌ $installer: never references \$CLI_BINARY_NAME (no CLI download)"
    set -g failed 1
end

if grep -qE -- '\$INSTALL_DIR/gpy([^-a-zA-Z]|$)' $installer
    echo "✅ $installer: installs to \$INSTALL_DIR/gpy"
else
    echo "❌ $installer: does not install to \$INSTALL_DIR/gpy"
    set -g failed 1
end

if test $failed -eq 1
    exit 1
end

echo "✅ install-oneline.sh downloads and installs the gpy CLI binary"
