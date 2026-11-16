#!/usr/bin/env bash
# tests/docker/scenarios/toolchain-no-install.sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A repository that pins toolchains (rust-toolchain.toml, .mise.toml,
# .tool-versions, a go.mod `toolchain` line) must not make GPY's language
# version probes install anything. Stub rustup, rustc, mise and go come first
# on PATH and log their argv and the five suppression variables; every probe
# must carry the values SECURITY.md ("Repository-selected code") names, and no
# stub may be asked to install or download.

set -euo pipefail
# shellcheck source=tests/docker/scenario-lib.sh
source /opt/scenario-lib.sh

REPO="$WORK/repo"
STUBS="$WORK/stubs"
CALLS="$WORK/stub-calls.log"
SUPPRESS=(RUSTUP_AUTO_INSTALL=0 GOTOOLCHAIN=local MISE_AUTO_INSTALL=0 MISE_EXEC_AUTO_INSTALL=0 MISE_NOT_FOUND_AUTO_INSTALL=0)

step "install $GPY_RELEASE_VERSION from the archive"
install_new bash
"$LOCAL_BIN/gpy-agent" stop >/dev/null 2>&1 || true

step "stub toolchains and a repository that pins them"
mkdir -p "$STUBS"
# One logger for every stub: `<name> | argv | VAR=value ...`, one line a call.
cat >"$STUBS/stub" <<EOF
#!/bin/bash
name="\${0##*/}"
line="\$name | \$* |"
for var in RUSTUP_AUTO_INSTALL GOTOOLCHAIN MISE_AUTO_INSTALL MISE_EXEC_AUTO_INSTALL MISE_NOT_FOUND_AUTO_INSTALL; do
    line+=" \$var=\${!var-<unset>}"
done
printf '%s\n' "\$line" >>"$CALLS"
case "\$name \$*" in
    "rustc --version") echo "rustc 1.80.0 (stub 2024-07-21)" ;;
    "go version") echo "go version go1.22.0 linux/amd64" ;;
    # A failing mise also sends the probe on to the direct command.
    mise*) exit 1 ;;
    *) exit 1 ;;
esac
EOF
chmod +x "$STUBS/stub"
for tool in rustup rustc cargo mise go; do ln -s stub "$STUBS/$tool"; done
mkdir -p "$REPO/src"
printf '[package]\nname = "pinned"\nversion = "0.1.0"\nedition = "2021"\n' >"$REPO/Cargo.toml"
echo 'fn main() {}' >"$REPO/src/main.rs"
printf '[toolchain]\nchannel = "1.99.0"\n' >"$REPO/rust-toolchain.toml"
printf 'module example.com/pinned\n\ngo 1.22\n\ntoolchain go1.99.0\n' >"$REPO/go.mod"
echo 'package main' >"$REPO/main.go"
printf '[tools]\nrust = "1.99.0"\ngo = "1.99.0"\n' >"$REPO/.mise.toml"
printf 'rust 1.99.0\ngolang 1.99.0\n' >"$REPO/.tool-versions"
make_repo "$REPO"
export PATH="$STUBS:$PATH"
[[ "$(command -v rustc)" == "$STUBS/rustc" ]] || abort "the stub rustc is not first on PATH"
pass "stubs for rustup, rustc, cargo, mise and go come first on PATH"
# The user's own environment allows auto-installs; GPY's probes must override it.
export RUSTUP_AUTO_INSTALL=1 GOTOOLCHAIN=auto MISE_AUTO_INSTALL=1 MISE_EXEC_AUTO_INSTALL=1 MISE_NOT_FOUND_AUTO_INSTALL=1

# `direct_probes` -- rustc and go calls logged so far.
direct_probes() { if [[ -f "$CALLS" ]]; then grep -c '^rustc \|^go ' "$CALLS" || true; else echo 0; fi; }

step "gpy-agent oneshot lang"
oneshot="$("$LOCAL_BIN/gpy-agent" oneshot lang --cwd "$REPO" --format json 2>&1 || true)"
oneshot_probes="$(direct_probes)"
if (( oneshot_probes > 0 )); then
    pass "oneshot lang probed the pinned toolchains: $oneshot"
else
    fail "oneshot lang ran no version probe: $oneshot"
fi

step "the agent serves lang requests"
"$LOCAL_BIN/gpy-agent" start >/dev/null 2>&1 || true
# shellcheck disable=SC2329 # called through poll
agent_up() { [[ "$(agent_field state)" == running ]]; }
poll 10 agent_up || abort "the agent did not start: $("$LOCAL_BIN/gpy-agent" status 2>&1 | head -n 5)"
# The agent may answer the first request from cold caches and probe in the
# background; ask until its own probes have run.
# shellcheck disable=SC2329 # called through poll
agent_probed() {
    ipc "{\"op\":\"lang\",\"cwd\":\"$REPO\",\"format\":\"json\"}" >/dev/null 2>&1 || true
    (( $(direct_probes) > oneshot_probes ))
}
if poll 15 agent_probed; then
    pass "the agent probed the pinned toolchains for an IPC lang request"
else
    fail "the agent ran no version probe for an IPC lang request"
fi
cd "$REPO"
bash -i -c '__gpy_precmd' </dev/null >/dev/null 2>&1 || true
cd "$WORK"
sleep 1
"$LOCAL_BIN/gpy-agent" stop >/dev/null 2>&1 || true

step "what the stubs saw"
[[ -s "$CALLS" ]] || abort "no stub was called; the language probes did not run, so nothing was tested"
sed 's/^/    | /' "$CALLS"
for tool in mise rustc go; do
    if grep -q "^$tool " "$CALLS"; then
        pass "the probes ran $tool ($(grep -c "^$tool " "$CALLS") call(s))"
    else
        fail "no probe ran $tool, so its suppression variables went unchecked"
    fi
done
bad_env=0
while IFS= read -r call; do
    for want in "${SUPPRESS[@]}"; do
        if [[ " ${call##*|} " != *" $want "* ]]; then
            fail "a probe ran without $want: $call"
            bad_env=$((bad_env + 1))
        fi
    done
done <"$CALLS"
if (( bad_env == 0 )); then
    pass "every probe ran with ${SUPPRESS[*]}, overriding the user's own values"
fi
installs="$(awk -F' [|] ' '{ print $1 " " $2 }' "$CALLS" |
    grep -E '(^| )(install|download|toolchain|use|i|sync|upgrade|update|add|get)( |$)' || true)"
if [[ -z "$installs" ]]; then
    pass "no stub was asked to install, download or switch a toolchain"
else
    fail "a stub was asked to install or download: $(echo "$installs" | tr '\n' ';')"
fi

finish
