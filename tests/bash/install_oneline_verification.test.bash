#!/usr/bin/env bash
# tests/bash/install_oneline_verification.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# install-oneline.sh checksum verification fails closed. Sources the function
# prelude (above `main() {`) and exercises verify_download with a stubbed downloader.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
INSTALLER="$ROOT/install-oneline.sh"

if ! command -v sha256sum >/dev/null 2>&1 && ! command -v shasum >/dev/null 2>&1; then
    test_skip "no SHA-256 tool available"
fi

WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

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

# Echoes "<exit status>|<output>".
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
# A proxy may answer with HTML; field 1 is not 64 hex chars, so it must not match.
result="$(SIDECAR_BODY="<!DOCTYPE html><title>404</title>" run_verify)"
[[ "${result%%|*}" -ne 0 ]] || fail "verify_download accepted an HTML error page as a checksum"

echo "--- curl's own error line reaches stderr ---"
FAKEBIN="$WORKDIR/fakebin"
mkdir -p "$FAKEBIN"
cat >"$FAKEBIN/curl" <<'EOF'
#!/bin/sh
echo "curl: (22) The requested URL returned error: 404" >&2
exit 22
EOF
chmod +x "$FAKEBIN/curl"
fetch_err="$(
    PATH="$FAKEBIN:$PATH" sh -c '
        . "$1"
        fetch_to "https://example.invalid/asset" "$2"
    ' _ "$PRELUDE" "$WORKDIR/fetched.bin" 2>&1 >/dev/null
)" && fail "fetch_to succeeded although curl failed"
case "$fetch_err" in
    *"curl: (22) The requested URL returned error: 404"*) ;;
    *) fail "fetch_to hid curl's error line: $fetch_err" ;;
esac

test_finish "install-oneline.sh verify_download fails closed"
