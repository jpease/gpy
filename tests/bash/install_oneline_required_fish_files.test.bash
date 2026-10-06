#!/usr/bin/env bash
# tests/bash/install_oneline_required_fish_files.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# install-oneline.sh must treat the two Fish files its wiring depends on as
# required (#808): conf.d/gpy_init.fish (the file the config.fish block
# sources) and functions/fish_prompt.fish (the symlink target). A failed
# download of either aborts the install before any rc file or symlink is
# written, completing the partial-install contract of the core files (#309).
# A failed segment download stays best-effort.
#
# Each scenario runs the installer in a sandboxed HOME behind a fake `curl`
# that serves stub binaries and this checkout's shell files, except URLs
# ending in $FAIL_PATTERN, which exit 22.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

case "$(uname -s | tr '[:upper:]' '[:lower:]')" in
    linux | darwin) ;;
    *) test_skip "install-oneline.sh does not support $(uname -s)" ;;
esac

failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
pass() { echo "PASS: $*"; }

write_fake_curl() {
    cat >"$1/curl" <<EOF
#!/bin/sh
ROOT="$ROOT"
AGENT_SRC="$2"
CLI_SRC="$3"
EOF
    cat >>"$1/curl" <<'EOF'
out=""; url=""; prev=""
for arg in "$@"; do
    if [ "$prev" = "-o" ]; then out="$arg"; else case "$arg" in -*) ;; *) url="$arg" ;; esac; fi
    prev="$arg"
done
digest_of() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'; else shasum -a 256 "$1" | awk '{print $1}'; fi
}
source_for() {
    case "$1" in
        */releases/download/*/gpy-agent-*) echo "$AGENT_SRC" ;;
        */releases/download/*/gpy-*) echo "$CLI_SRC" ;;
        */raw.githubusercontent.com/*) echo "$ROOT/${1#*/raw.githubusercontent.com/*/*/*/}" ;;
        *) echo "" ;;
    esac
}
if [ -n "$FAIL_PATTERN" ]; then
    case "$url" in *"$FAIL_PATTERN") exit 22 ;; esac
fi
case "$url" in
    *.sha256)
        src="$(source_for "${url%.sha256}")"
        [ -n "$src" ] && [ -f "$src" ] || exit 22
        base="${url##*/}"
        printf '%s  %s\n' "$(digest_of "$src")" "${base%.sha256}"
        exit 0
        ;;
esac
src="$(source_for "$url")"
[ -n "$src" ] && [ -f "$src" ] || exit 22
if [ -n "$out" ]; then
    mkdir -p "$(dirname "$out")"
    cp "$src" "$out"
else
    cat "$src"
fi
exit 0
EOF
    chmod +x "$1/curl"
}

write_stub() {
    printf '#!/bin/sh\necho "%s"\nexit 0\n' "$2" >"$1"
    chmod +x "$1"
}

shell_e2e_init "$ROOT"

# `run_scenario NAME FAIL_PATTERN` -- sets rc, cfg, log.
run_scenario() {
    name="$1" pattern="$2"
    echo "=== $name ==="
    home="$SHELL_E2E_ROOT/$name/home"
    stubs="$SHELL_E2E_ROOT/$name/stubs"
    fakebin="$SHELL_E2E_ROOT/$name/fakebin"
    cfg="$SHELL_E2E_ROOT/$name/xdg"
    log="$SHELL_E2E_ROOT/$name/install.log"
    mkdir -p "$home" "$stubs" "$fakebin" "$cfg"
    write_stub "$stubs/agent" "gpy-agent 9.9.9"
    write_stub "$stubs/cli" "gpy 9.9.9"
    write_fake_curl "$fakebin" "$stubs/agent" "$stubs/cli"

    env HOME="$home" XDG_CONFIG_HOME="$cfg" FAIL_PATTERN="$pattern" \
        PATH="$fakebin:$home/.local/bin:$PATH" GPY_SHELL=fish GPY_VERSION=v9.9.9-test \
        sh "$ROOT/install-oneline.sh" </dev/null >"$log" 2>&1
    rc=$?
}

for required in conf.d/gpy_init.fish functions/fish_prompt.fish; do
    run_scenario "required_$(printf '%s' "$required" | tr '/.' '__')" "/fish/$required"
    if [ "$rc" -ne 0 ]; then
        pass "$required download failure: installer exits non-zero"
    else
        fail "$required download failure: installer exited 0"
    fi
    if [ -f "$cfg/fish/config.fish" ] && grep -qF '# >>> gpy-init >>>' "$cfg/fish/config.fish"; then
        fail "$required download failure: config.fish has a gpy-init block"
    else
        pass "$required download failure: config.fish has no gpy-init block"
    fi
    if [ -L "$cfg/fish/functions/fish_prompt.fish" ]; then
        fail "$required download failure: fish_prompt.fish symlink was created"
    else
        pass "$required download failure: no fish_prompt.fish symlink"
    fi
done

run_scenario optional_segment "/fish/segments/clock.fish"
if [ "$rc" -eq 0 ] && grep -qF 'Could not download clock.fish' "$log" \
    && grep -qF '# >>> gpy-init >>>' "$cfg/fish/config.fish"; then
    pass "segment download failure only warns; install completes"
else
    fail "segment download failure must warn and finish (rc=$rc)"
    sed 's/^/    /' "$log"
fi

if [ "$failures" -gt 0 ]; then
    echo "FAIL: $failures check(s) failed"
    exit 1
fi
echo "PASS: required Fish files abort the install before any rc change"
