#!/usr/bin/env bash
# tests/bash/install_oneline_verification.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# install-oneline.sh's checksum verification fails closed. The installer is a
# curl|sh entry point and cannot run end to end here, so this sources its
# function prelude (everything above `main() {`) and exercises
# verify_download against a stubbed downloader: a matching sidecar verifies;
# a mismatched, absent or non-digest sidecar aborts.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
INSTALLER="$ROOT/install-oneline.sh"

if ! command -v sha256sum >/dev/null 2>&1 && ! command -v shasum >/dev/null 2>&1; then
    test_skip "no SHA-256 tool available"
fi

WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

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

test_finish "install-oneline.sh verify_download fails closed"
