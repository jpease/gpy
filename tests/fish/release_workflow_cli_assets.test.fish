#!/usr/bin/env fish
# tests/fish/release_workflow_cli_assets.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The release pipeline must build and publish the user-facing `gpy` CLI
# binary as a release asset for every target it ships the `gpy-agent` daemon
# for; otherwise no install path can fetch `gpy` from a GitHub release.
#
# Static/text-level check: it parses release.yml with the
# same pure-fish/grep tooling the rest of the suite uses, deliberately NOT a
# YAML CLI. The sibling install tests avoid external tools so the suite stays
# portable across CI runners; `yq` in particular is ambiguous (mikefarah vs
# python/kislyuk take incompatible syntax) and is not installed by any of the
# fish-test CI jobs (pr-gate.yml installs only fish/zsh/bash), so depending on
# it would fail CI spuriously.
#
# It asserts (1) each build-matrix row declares cli_artifact_name and
# cli_asset_name, (2) the CLI asset names are the expected parallel set, and
# (3) the packaging step requires each CLI asset and the release uploads them.
#
# Publishing goes through packaging: scripts/package-release.sh renames every
# artifact to its asset name under `gpy-release/bin/`, fails closed on any
# that is absent, and create-release uploads that directory rather than
# naming each `release-artifacts/<asset>/<file>` path (a list that would
# duplicate the build matrix and drift from it). So the per-asset assertion
# targets package-release.sh's EXPECTED_ASSETS list, which is what actually
# enforces "every target ships a gpy binary".

source (dirname (status --filename))/../lib/test_helpers.fish

set -l script_dir (dirname (status --filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

set -l workflow .github/workflows/release.yml
# 1. Every build-matrix row must declare cli_artifact_name and cli_asset_name.
#    The matrix has one row per target; count the agent asset_name lines to get
#    the row count, then require an equal number of cli_ counterparts. Anchor
#    on `cli_asset_name` explicitly so it is not conflated with the agent's
#    `asset_name`.
set -l row_count (grep -cE '^ +asset_name: gpy-agent-' $workflow)
set -l cli_asset_count (grep -cE '^ +cli_asset_name:' $workflow)
set -l cli_artifact_count (grep -cE '^ +cli_artifact_name:' $workflow)

if test "$cli_asset_count" = "$row_count"
    pass "all $row_count matrix rows declare cli_asset_name"
else
    fail "only $cli_asset_count/$row_count matrix rows declare cli_asset_name"
end

if test "$cli_artifact_count" = "$row_count"
    pass "all $row_count matrix rows declare cli_artifact_name"
else
    fail "only $cli_artifact_count/$row_count matrix rows declare cli_artifact_name"
end

# 2. Each expected CLI asset name must appear as a cli_asset_name in the matrix.
set -l expected_cli_assets \
    gpy-linux-x86_64 \
    gpy-linux-aarch64 \
    gpy-macos-x86_64 \
    gpy-macos-aarch64 \
    gpy-windows-x86_64.exe

for asset in $expected_cli_assets
    if grep -qF -- "cli_asset_name: $asset" $workflow
        pass "matrix declares cli_asset_name: $asset"
    else
        fail "matrix missing cli_asset_name: $asset"
    end
end

# 3a. Packaging must require each CLI asset. EXPECTED_ASSETS entries are one
#     per line at a fixed four-space indent, so an exact whole-line match keeps
#     `gpy-linux-x86_64` from being satisfied by `gpy-agent-linux-x86_64`.
set -l packager scripts/package-release.sh

for asset in $expected_cli_assets
    if grep -qxF -- "    $asset" $packager
        pass "$packager requires $asset"
    else
        fail "$packager does not require $asset"
    end
end

# 3b. The create-release job must upload the directory packaging wrote those
#     asset-named binaries into.
if grep -qF -- 'dist/gpy-release/bin/*' $workflow
    pass "create-release uploads dist/gpy-release/bin/*"
else
    fail "create-release does not upload dist/gpy-release/bin/*"
end

test_finish "release.yml builds and publishes the gpy CLI binary for every target"
