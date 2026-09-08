#!/usr/bin/env bash
# tests/bash/install_oneline_verification.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression test for #494. install-oneline.sh used to verify checksums
# "opportunistically": if the release published no <asset>.sha256 -- which no
# release did -- it skipped verification and installed the binary anyway. It
# also fell back to raw.githubusercontent.com/jpease/gpy/main/bin/<asset> and to
# VERSION="main" whenever the release download or the GitHub API failed, so the
# documented recovery path from "I could not verify this" was "install from a
# mutable branch instead".
#
# install-oneline.sh is a curl|sh entry point and cannot be run end to end here,
# so this does two things:
#   1. sources the script's function prelude (everything above `main() {`) and
#      exercises verify_download directly against a stubbed downloader
#   2. asserts structurally that the removed fallbacks have not come back and
#      that quarantine clearing happens only after verification

# shellcheck disable=SC2016
# The single quotes are load-bearing throughout this file: the assertions grep
# for literal `$VAR` text in a script under test, so expanding here would test
# the wrong thing.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
INSTALLER="$ROOT/install-oneline.sh"

if ! command -v sha256sum >/dev/null 2>&1 && ! command -v shasum >/dev/null 2>&1; then
    echo "SKIP: no SHA-256 tool available"
    exit 0
fi

WORKDIR="$(mktemp -d)"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT

failures=0
fail() {
    echo "FAIL: $*"
    failures=$((failures + 1))
}

# --- behavioral: verify_download ----------------------------------------------
#
# The prelude is everything above `main() {`: the logging helpers, the
# downloader wrappers and the verification functions. Sourcing it gets the real
# implementation under test without executing the installer body.
PRELUDE="$WORKDIR/prelude.sh"
sed -n '1,/^main() {/p' "$INSTALLER" | sed '$d' >"$PRELUDE"

grep -q '^verify_download()' "$PRELUDE" ||
    fail "verify_download is not defined above main(); the prelude extraction is out of date"

PAYLOAD="$WORKDIR/payload.bin"
printf 'the binary that was downloaded\n' >"$PAYLOAD"
if command -v sha256sum >/dev/null 2>&1; then
    GOOD_SUM="$(sha256sum "$PAYLOAD" | awk '{print $1}')"
else
    GOOD_SUM="$(shasum -a 256 "$PAYLOAD" | awk '{print $1}')"
fi

# Run verify_download with fetch_stdout stubbed to emit $SIDECAR_BODY as the
# sidecar the release would have served. Echoes "<exit status>|<output>".
run_verify() {
    local out status=0
    out="$(
        sh -c '
            . "$1"
            fetch_stdout() { printf "%s" "$SIDECAR_BODY"; }
            verify_download "$2" "https://example.invalid/asset" "test-asset"
        ' _ "$PRELUDE" "$PAYLOAD"
    )" || status=$?
    printf '%s|%s' "$status" "$out"
}

echo "--- a matching sidecar verifies ---"
result="$(SIDECAR_BODY="$GOOD_SUM  test-asset" run_verify)"
[[ "${result%%|*}" -eq 0 ]] || fail "verify_download rejected a correct checksum (exit ${result%%|*})"
[[ "$result" == *"Verified test-asset checksum"* ]] ||
    fail "verify_download did not report a successful verification"

echo "--- a mismatched sidecar aborts ---"
result="$(SIDECAR_BODY="0000000000000000000000000000000000000000000000000000000000000000  test-asset" run_verify)"
[[ "${result%%|*}" -ne 0 ]] || fail "verify_download accepted a mismatched checksum"

echo "--- an absent sidecar aborts ---"
result="$(SIDECAR_BODY="" run_verify)"
[[ "${result%%|*}" -ne 0 ]] ||
    fail "verify_download accepted a missing checksum; verification must fail closed"

echo "--- a 404 page in place of a sidecar aborts ---"
# A proxy or a mistyped host can answer with HTML instead of a digest. Field 1
# of that is not 64 hex characters, so it must not match.
result="$(SIDECAR_BODY="<!DOCTYPE html><title>404</title>" run_verify)"
[[ "${result%%|*}" -ne 0 ]] || fail "verify_download accepted an HTML error page as a checksum"

# --- structural: the removed fallbacks stay removed ---------------------------
#
# Grep the code, not the prose. The comments in install-oneline.sh name the
# patterns that were removed and why, which is exactly what these assertions
# search for -- so full-line comments are stripped first. Line numbers below
# come from this same stripped view so their relative order still holds.
CODE="$WORKDIR/code.sh"
grep -vE '^[[:space:]]*#' "$INSTALLER" >"$CODE"
INSTALLER="$CODE"

echo "--- no mutable-branch fallbacks ---"

if grep -q 'raw.githubusercontent.com/\$REPO/main/bin' "$INSTALLER"; then
    fail "install-oneline.sh downloads binaries from the mutable main branch again"
fi

if grep -qE '^\s*VERSION="main"' "$INSTALLER"; then
    fail "install-oneline.sh falls back to VERSION=\"main\" again (unpinned, unverifiable install)"
fi

# Shell files are pinned to the resolved tag; the binary URL must be a release
# download, not a branch path.
grep -q 'BINARY_URL="https://github.com/\$REPO/releases/download/\$VERSION/\$BINARY_NAME"' "$INSTALLER" ||
    fail "the agent download URL is no longer a pinned release asset"

echo "--- both binaries are verified ---"
for call in 'verify_download "$TEMP_BINARY" "$BINARY_URL"' 'verify_download "$TEMP_CLI_BINARY" "$CLI_BINARY_URL"'; do
    grep -qF -- "$call" "$INSTALLER" || fail "install-oneline.sh does not call: $call"
done

echo "--- quarantine handling ---"

if grep -q 'xattr -c' "$INSTALLER"; then
    fail "install-oneline.sh still runs 'xattr -c', which strips every extended attribute"
fi

grep -q 'xattr -d com.apple.quarantine' "$INSTALLER" ||
    fail "install-oneline.sh no longer clears the macOS quarantine flag at all"

# Verification must come first for each binary: a quarantine flag is a Gatekeeper
# barrier, and removing it from an unverified download is the one ordering that
# makes the whole exercise pointless.
agent_verify_line="$(grep -n 'verify_download "\$TEMP_BINARY"' "$INSTALLER" | head -1 | cut -d: -f1)"
agent_clear_line="$(grep -n 'clear_quarantine "\$INSTALL_DIR/gpy-agent"' "$INSTALLER" | head -1 | cut -d: -f1)"
cli_verify_line="$(grep -n 'verify_download "\$TEMP_CLI_BINARY"' "$INSTALLER" | head -1 | cut -d: -f1)"
cli_clear_line="$(grep -n 'clear_quarantine "\$INSTALL_DIR/gpy"' "$INSTALLER" | head -1 | cut -d: -f1)"

if [[ -z "$agent_verify_line" || -z "$agent_clear_line" ]]; then
    fail "could not locate the agent verify/quarantine pair"
elif [[ "$agent_verify_line" -ge "$agent_clear_line" ]]; then
    fail "the agent's quarantine flag is cleared before its checksum is verified"
fi

if [[ -z "$cli_verify_line" || -z "$cli_clear_line" ]]; then
    fail "could not locate the CLI verify/quarantine pair"
elif [[ "$cli_verify_line" -ge "$cli_clear_line" ]]; then
    fail "the CLI's quarantine flag is cleared before its checksum is verified"
fi

echo "--- prerequisites are fatal up front ---"
grep -q 'No SHA-256 tool found' "$INSTALLER" ||
    fail "install-oneline.sh does not refuse to run without a SHA-256 tool"

if [[ $failures -ne 0 ]]; then
    echo "$failures assertion(s) failed"
    exit 1
fi

echo "PASS"
