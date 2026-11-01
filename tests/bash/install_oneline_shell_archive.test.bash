#!/usr/bin/env bash
# tests/bash/install_oneline_shell_archive.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# install-oneline.sh installs shell scripts from gpy-shell-files.tar.gz only
# after its published SHA-256 matches; a missing entry, bad/absent checksum or
# failed download aborts before anything is written.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

case "$(uname -s | tr '[:upper:]' '[:lower:]')" in
    linux | darwin) ;;
    *) test_skip "install-oneline.sh does not support $(uname -s)" ;;
esac

write_fake_curl() {
    cat >"$1/curl" <<EOF
#!/bin/sh
AGENT_SRC="$2"
CLI_SRC="$3"
SHELL_ARCHIVE="$4"
EOF
    cat >>"$1/curl" <<'EOF'
out=""; url=""; prev=""
for arg in "$@"; do
    if [ "$prev" = "-o" ]; then out="$arg"; else case "$arg" in -*) ;; *) url="$arg" ;; esac; fi
    prev="$arg"
done
printf '%s\n' "$url" >>"$CURL_LOG"
digest_of() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'; else shasum -a 256 "$1" | awk '{print $1}'; fi
}
source_for() {
    case "$1" in
        */releases/download/*/gpy-shell-files.tar.gz) echo "$SHELL_ARCHIVE" ;;
        */releases/download/*/gpy-agent-*) echo "$AGENT_SRC" ;;
        */releases/download/*/gpy-*) echo "$CLI_SRC" ;;
        *) echo "" ;;
    esac
}
if [ -n "$FAIL_PATTERN" ]; then
    case "$url" in *"$FAIL_PATTERN") exit 22 ;; esac
fi
case "$url" in
    *gpy-shell-files.tar.gz.sha256)
        if [ -n "$BAD_SHELL_DIGEST" ]; then
            printf '%s  gpy-shell-files.tar.gz\n' "0000000000000000000000000000000000000000000000000000000000000000"
            exit 0
        fi
        ;;
esac
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

REAL_GPY="$ROOT/gpy-agent/target/debug/gpy"
if [ ! -x "$REAL_GPY" ]; then
    (cd "$ROOT/gpy-agent" && cargo build --quiet --bin gpy) || {
        echo "FAIL: could not build the gpy binary"
        exit 1
    }
fi

write_cli_stub() {
    # shellcheck disable=SC2016  # the stub's own $1 and $@ must stay literal
    printf '#!/bin/sh\nif [ "$1" = setup ]; then exec "%s" "$@"; fi\necho "gpy 9.9.9"\nexit 0\n' "$REAL_GPY" >"$1"
    chmod +x "$1"
}

shell_e2e_init "$ROOT"

# DROP: file left out of the archive; FAIL_PATTERN: URL suffix curl refuses;
# non-empty BAD_DIGEST: sidecar disagrees with the archive.
run_scenario() {
    name="$1" shell="$2" drop="$3" pattern="$4" bad_digest="$5"
    echo "=== $name ==="
    home="$SHELL_E2E_ROOT/$name/home"
    stubs="$SHELL_E2E_ROOT/$name/stubs"
    fakebin="$SHELL_E2E_ROOT/$name/fakebin"
    cfg="$SHELL_E2E_ROOT/$name/xdg"
    data="$SHELL_E2E_ROOT/$name/data"
    log="$SHELL_E2E_ROOT/$name/install.log"
    curl_log="$SHELL_E2E_ROOT/$name/curl.log"
    archive="$SHELL_E2E_ROOT/$name/gpy-shell-files.tar.gz"
    mkdir -p "$home" "$stubs" "$fakebin" "$cfg"
    : >"$curl_log"
    shell_e2e_build_shell_archive "$ROOT" "$archive" "$drop" || {
        fail "$name: could not build the shell archive"
        return 1
    }
    write_stub "$stubs/agent" "gpy-agent 9.9.9"
    write_cli_stub "$stubs/cli"
    write_fake_curl "$fakebin" "$stubs/agent" "$stubs/cli" "$archive"

    env HOME="$home" XDG_CONFIG_HOME="$cfg" XDG_DATA_HOME="$data" CURL_LOG="$curl_log" FAIL_PATTERN="$pattern" \
        BAD_SHELL_DIGEST="$bad_digest" \
        PATH="$fakebin:$home/.local/bin:$PATH" GPY_SHELL="$shell" GPY_VERSION=v9.9.9-test \
        sh "$ROOT/install-oneline.sh" </dev/null >"$log" 2>&1
    rc=$?
}

rc_wired() {
    case "$1" in
        fish) rc_file="$cfg/fish/config.fish" ;;
        zsh) rc_file="$home/.zshrc" ;;
        bash) rc_file="$home/.bashrc" ;;
    esac
    [ -f "$rc_file" ] && grep -qF '# >>> gpy-init >>>' "$rc_file"
}

assert_not_installed() {
    name="$1" shell="$2"
    if [ "$rc" -ne 0 ]; then
        pass "$name: installer exits non-zero"
    else
        fail "$name: installer exited 0"
    fi
    if rc_wired "$shell"; then
        fail "$name: the rc file has a gpy-init block"
    else
        pass "$name: the rc file has no gpy-init block"
    fi
    if [ -e "$data/gpy" ] || [ -L "$cfg/fish/functions/fish_prompt.fish" ]; then
        fail "$name: shell files or the prompt symlink were installed"
    else
        pass "$name: no shell files or prompt symlink were installed"
    fi
    if [ -e "$home/.local/bin/gpy-agent" ] || [ -e "$home/.local/bin/gpy" ]; then
        fail "$name: binaries were installed before the archive was verified"
    else
        pass "$name: no binary was installed"
    fi
}

for shell in fish zsh bash; do
    run_scenario "complete_$shell" "$shell" "" "" ""
    if [ "$rc" -eq 0 ] && rc_wired "$shell"; then
        pass "$shell: a verified archive installs and wires the rc file"
    else
        fail "$shell: a verified archive did not install (rc=$rc)"
        sed 's/^/    /' "$log"
    fi
    if grep -qF 'Verified gpy-shell-files.tar.gz checksum' "$log"; then
        pass "$shell: the archive checksum was verified"
    else
        fail "$shell: no archive checksum verification in the log"
    fi
    if grep -q 'raw.githubusercontent.com' "$curl_log"; then
        fail "$shell: the installer fetched from raw.githubusercontent.com"
    else
        pass "$shell: nothing was fetched from raw.githubusercontent.com"
    fi
    if grep -v '^https://github.com/jpease/gpy/releases/download/v9.9.9-test/' "$curl_log" | grep -q .; then
        fail "$shell: a URL outside the requested release was fetched: $(grep -v '^https://github.com/jpease/gpy/releases/download/v9.9.9-test/' "$curl_log")"
    else
        pass "$shell: every download is an asset of the requested release"
    fi
done

for entry in fish/conf.d/gpy_init.fish fish/functions/fish_prompt.fish fish/core/init.fish \
    fish/core/flags.fish fish/completions/gpy-dynamic.fish bash/core/g_runner.bash; do
    run_scenario "missing_$(printf '%s' "$entry" | tr '/.' '__')" fish "$entry" "" ""
    assert_not_installed "archive without $entry" fish
    if grep -qF "has no ${entry}" "$log"; then
        pass "archive without $entry: the error names the file"
    else
        fail "archive without $entry: the log does not name the missing file"
    fi
done
run_scenario missing_gpy_zsh zsh zsh/gpy.zsh "" ""
assert_not_installed "archive without zsh/gpy.zsh" zsh
run_scenario missing_gpy_bash bash bash/gpy.bash "" ""
assert_not_installed "archive without bash/gpy.bash" bash

run_scenario bad_digest bash "" "" 1
assert_not_installed "archive whose digest does not match" bash
if grep -qF 'Checksum mismatch' "$log"; then
    pass "mismatched digest: the error reports a checksum mismatch"
else
    fail "mismatched digest: no checksum mismatch message"
fi

run_scenario no_sidecar bash "" "gpy-shell-files.tar.gz.sha256" ""
assert_not_installed "archive without a published checksum" bash

run_scenario no_archive bash "" "/gpy-shell-files.tar.gz" ""
assert_not_installed "archive that cannot be downloaded" bash

test_finish "install-oneline.sh installs shell files only from a verified release archive"
