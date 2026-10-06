#!/usr/bin/env bash
# tests/bash/quality_check_fix_cwd.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# #811: quality-check.sh --fix must cd to the repo root before running
# fish_indent and other commands. If invoked from a different directory,
# it should not reformat .fish files under the caller's cwd.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

# Create a temporary directory for the test.
TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/gpy-fixcwd.XXXXXX")"
trap 'rm -rf "$TEST_ROOT"' EXIT

# Create a fake repo structure inside the test root.
FAKE_REPO="$TEST_ROOT/fake-repo"
mkdir -p "$FAKE_REPO/gpy-agent/src"
mkdir -p "$FAKE_REPO/bash"
mkdir -p "$FAKE_REPO/fish"
mkdir -p "$FAKE_REPO/scripts"
mkdir -p "$FAKE_REPO/tests"

# Copy the real quality-check.sh and its helpers.
cp "$ROOT/scripts/quality-check.sh" "$FAKE_REPO/scripts/quality-check.sh"
cp "$ROOT/scripts/check-active-toolchain.sh" "$FAKE_REPO/scripts/check-active-toolchain.sh"
cp "$ROOT/scripts/check-license-metadata.sh" "$FAKE_REPO/scripts/check-license-metadata.sh"
cp "$ROOT/scripts/check-privacy-patterns.sh" "$FAKE_REPO/scripts/check-privacy-patterns.sh"
cp "$ROOT/scripts/check-doc-links.sh" "$FAKE_REPO/scripts/check-doc-links.sh"

# Create a minimal Cargo.toml and rust-toolchain.toml for the fake repo
# so require_pinned_toolchain succeeds.
cat > "$FAKE_REPO/gpy-agent/Cargo.toml" << 'EOF'
[package]
name = "gpy-agent"
version = "0.1.0"
edition = "2021"

[lints]
EOF

# Stub check-active-toolchain.sh to succeed without checking rustc.
# The real script would fail since we don't have the right toolchain.
cat > "$FAKE_REPO/scripts/check-active-toolchain.sh" << 'EOF'
#!/bin/bash
# Stub that always succeeds
exit 0
EOF
chmod +x "$FAKE_REPO/scripts/check-active-toolchain.sh"

# Create a minimal .fish file to test formatting outside the repo.
EXTERNAL_FISH="$TEST_ROOT/external.fish"
cat > "$EXTERNAL_FISH" << 'EOF'
function   badly_formatted
    echo   "This has extra spaces"
end
EOF
chmod 644 "$EXTERNAL_FISH"

# Record the original content.
EXTERNAL_ORIGINAL="$(cat "$EXTERNAL_FISH")"

# Create a minimal .fish file inside the fake repo that should be formatted.
REPO_FISH="$FAKE_REPO/fish/test.fish"
cat > "$REPO_FISH" << 'EOF'
function   also_badly_formatted
    echo   "This also has extra spaces"
end
EOF
chmod 644 "$REPO_FISH"

# Create a minimal gpy.bash to satisfy shell integration checks.
mkdir -p "$FAKE_REPO/bash"
cat > "$FAKE_REPO/bash/gpy.bash" << 'EOF'
#!/bin/bash
# Stub bash integration
__prompt_color="always"
__gpy_duration_method="BASH_REMATCH"
__gpy_render_prompt() { PS1="test> "; }
__enabled_segments=""
EOF

# Create a minimal gpy.fish to satisfy shell integration checks.
mkdir -p "$FAKE_REPO/fish"
cat > "$FAKE_REPO/fish/gpy.fish" << 'EOF'
#!/usr/bin/env fish
# Stub fish integration
set __prompt_color always
function __gpy_render_prompt; end
EOF

# Create a minimal test directory.
mkdir -p "$FAKE_REPO/tests/bash"
mkdir -p "$FAKE_REPO/tests/fish"
mkdir -p "$FAKE_REPO/tests/zsh"

# Create a git repo in the fake repo so git commands work.
(cd "$FAKE_REPO" && git init -q && git config user.email "test@test.com" && git config user.name "Test")

# Run quality-check.sh --fix from the test root (NOT from the repo).
# This is the bug scenario: calling from a different directory.
cd "$TEST_ROOT"

if bash "$FAKE_REPO/scripts/quality-check.sh" --fix > /dev/null 2>&1; then
    # Script should succeed.
    echo "PASS: quality-check.sh --fix completed successfully"
else
    # If --fix failed due to missing commands, that's OK for this test.
    # The critical part is whether external.fish was modified.
    echo "INFO: quality-check.sh --fix exited with status $?, continuing test"
fi

# The critical check: the external .fish file should NOT have been modified.
EXTERNAL_AFTER="$(cat "$EXTERNAL_FISH")"
if [[ "$EXTERNAL_ORIGINAL" == "$EXTERNAL_AFTER" ]]; then
    echo "PASS: External .fish file was not modified"
else
    echo "FAIL: External .fish file was modified unexpectedly"
    echo "Before: $EXTERNAL_ORIGINAL"
    echo "After:  $EXTERNAL_AFTER"
    exit 1
fi

echo "=== All tests passed ==="
