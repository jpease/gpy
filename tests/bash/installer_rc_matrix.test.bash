#!/usr/bin/env bash
# tests/bash/installer_rc_matrix.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# INSTALLER RC TEST -- the class test for every installer rc-file bug in epic
# #675. It is also the named regression test for #746 (a space in HOME or
# XDG_CONFIG_HOME broke the unquoted `source` line), so there is no separate
# install_oneline_space_in_path file.
#
# Invariant, asserted per row:
#   after install, every shell the installer targets, started the way a user
#   starts it (interactive login AND non-login), sources gpy exactly once and
#   prints nothing on stderr; after uninstall (scripts/uninstall.sh or
#   scripts/uninstall.fish) every rc file is byte-identical to before install.
#   A row that expects `refuse` instead asserts the installer exits non-zero
#   and no rc file (and no gpy file under XDG_CONFIG_HOME) was touched.
#
# Rows = installer x shell x hostile HOME layout:
#
#   row INSTALLER SHELL LAYOUT EXPECT [UNINSTALL]
#     INSTALLER  oneline (install-oneline.sh, fake curl) | install.sh (staged
#                release payload, fish only)
#     SHELL      fish | zsh | bash
#     LAYOUT     a `layout_<name>` function below; it sets HOME and
#                XDG_CONFIG_HOME
#     EXPECT     ok | refuse
#     UNINSTALL  optional: `fish` runs scripts/uninstall.fish instead of the
#                default (scripts/uninstall.sh for oneline, uninstall.fish for
#                install.sh)
#
# Adding a row is one `row ...` line at the bottom; adding a hostile layout is
# one `layout_<name>` function. Later issues add: #747 (no ~/.bashrc but
# ~/.profile -> bash login/non-login), #748 (ZDOTDIR set).
#
# Paths are never interpolated into `-c` code: they travel as arguments or
# environment. Every row's agent is stopped before the row ends.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

test_require_command fish
test_require_command zsh
test_require_command python3

GPY_CLI="$ROOT/gpy-agent/target/debug/gpy"
GPY_AGENT="$ROOT/gpy-agent/target/debug/gpy-agent"
if [ ! -x "$GPY_CLI" ] || [ ! -x "$GPY_AGENT" ]; then
    (cd "$ROOT/gpy-agent" && cargo build --quiet --bin gpy --bin gpy-agent) || {
        echo "FAIL: could not build the gpy binaries"
        exit 1
    }
fi

case "$(uname -s | tr '[:upper:]' '[:lower:]')" in
    linux)
        case "$(uname -m)" in
            x86_64) AGENT_ASSET=gpy-agent-linux-x86_64; CLI_ASSET=gpy-linux-x86_64 ;;
            aarch64 | arm64) AGENT_ASSET=gpy-agent-linux-aarch64; CLI_ASSET=gpy-linux-aarch64 ;;
            *) test_skip "unsupported architecture $(uname -m)" ;;
        esac
        ;;
    darwin)
        case "$(uname -m)" in
            x86_64) AGENT_ASSET=gpy-agent-macos-x86_64; CLI_ASSET=gpy-macos-x86_64 ;;
            arm64) AGENT_ASSET=gpy-agent-macos-aarch64; CLI_ASSET=gpy-macos-aarch64 ;;
            *) test_skip "unsupported architecture $(uname -m)" ;;
        esac
        ;;
    *) test_skip "the installers do not support $(uname -s)" ;;
esac

failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
pass() { echo "PASS: $*"; }

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'; else shasum -a 256 "$1" | awk '{print $1}'; fi
}

# Copied from install_oneline_e2e_real_binary.test.bash (that file is a
# script, not a library, and existing tests stay unmodified): a `curl` that
# serves release assets from the debug build and shell files from this
# checkout.
write_fake_curl() {
    cat >"$1/curl" <<EOF
#!/bin/sh
ROOT="$ROOT"
AGENT_BIN="$GPY_AGENT"
CLI_BIN="$GPY_CLI"
AGENT_ASSET="$AGENT_ASSET"
CLI_ASSET="$CLI_ASSET"
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
        */releases/download/*/"$AGENT_ASSET") echo "$AGENT_BIN" ;;
        */releases/download/*/"$CLI_ASSET") echo "$CLI_BIN" ;;
        */raw.githubusercontent.com/*) echo "$ROOT/${1#*/raw.githubusercontent.com/*/*/*/}" ;;
        *) echo "" ;;
    esac
}
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

# --- hostile layouts ----------------------------------------------------------
# Each sets HOME and XDG_CONFIG_HOME under $SB (the sandbox root).
layout_plain() { HOME="$SB/home"; XDG_CONFIG_HOME="$SB/config"; }
layout_space() { HOME="$SB/sp ace/home"; XDG_CONFIG_HOME="$SB/sp ace/config"; }
layout_quote() { HOME="$SB/home"; XDG_CONFIG_HOME="$SB/q\"uote/config"; }
layout_dollar() { HOME="$SB/home"; XDG_CONFIG_HOME="$SB/d\$ollar/config"; }
layout_backtick() { HOME="$SB/home"; XDG_CONFIG_HOME="$SB/b\`tick/config"; }
layout_backslash() { HOME="$SB/home"; XDG_CONFIG_HOME="$SB/b\\slash/config"; }

# #747: bash startup-file shapes. SEED_BASH picks what seed_user_files makes
# (default `chain`: ~/.bashrc plus a ~/.bash_profile that sources it).
#   profile_only  no ~/.bashrc, ~/.profile exports FROM_PROFILE (Debian default
#                 minus the skeleton .bashrc)
#   none          no bash startup file at all
#   nochain       ~/.bashrc and a ~/.bash_profile that does not source it
layout_bash_profile_only() { layout_plain; SEED_BASH=profile_only; }
layout_bash_none() { layout_plain; SEED_BASH=none; }
layout_bash_nochain() { layout_plain; SEED_BASH=nochain; }

# #748: zsh reads ${ZDOTDIR:-$HOME}/.zshrc. ZDOTDIR is exported to the
# installer and uninstaller; ~/.zshrc must stay absent throughout.
#   zdotdir         $ZDOTDIR/.zshrc exists with user content
#   zdotdir_absent  ZDOTDIR is set and its directory exists, but no .zshrc
layout_zdotdir() { layout_plain; ZDOTDIR="$SB/zdot"; }
layout_zdotdir_absent() { layout_plain; ZDOTDIR="$SB/zdot"; SEED_ZSH=absent; }

# #744: ~/.config/fish/functions/fish_prompt.fish is a symlink into a dotfiles
# directory (stow/yadm/chezmoi). Install must move it aside as a symlink and
# uninstall must put it back; the link text and the target's bytes are part of
# the byte-identical snapshot.
layout_fish_prompt_symlink() { layout_plain; SEED_PROMPT=symlink; }

# The rc files that exist before install: the shell's own rc, and for bash a
# ~/.bash_profile that chains to ~/.bashrc (as macOS and most distros ship),
# so a login bash reaches the same file a non-login one does.
seed_user_files() {
    case "$1" in
        fish)
            mkdir -p "$XDG_CONFIG_HOME/fish"
            printf '# user rc before gpy\n' >"$XDG_CONFIG_HOME/fish/config.fish"
            if [ "$SEED_PROMPT" = symlink ]; then
                mkdir -p "$HOME/dotfiles" "$XDG_CONFIG_HOME/fish/functions"
                printf 'function fish_prompt; echo mine; end\n' >"$HOME/dotfiles/fish_prompt.fish"
                ln -s "$HOME/dotfiles/fish_prompt.fish" "$XDG_CONFIG_HOME/fish/functions/fish_prompt.fish"
            fi
            ;;
        zsh)
            mkdir -p "${ZDOTDIR:-$HOME}"
            [ "$SEED_ZSH" = absent ] || printf '# user rc before gpy\n' >"${ZDOTDIR:-$HOME}/.zshrc"
            ;;
        bash)
            case "$SEED_BASH" in
                profile_only) printf 'export FROM_PROFILE=yes\n' >"$HOME/.profile" ;;
                none) ;;
                nochain)
                    printf '# user rc before gpy\n' >"$HOME/.bashrc"
                    printf 'export FROM_PROFILE=yes\n' >"$HOME/.bash_profile"
                    ;;
                *)
                    printf '# user rc before gpy\n' >"$HOME/.bashrc"
                    printf '[ -f ~/.bashrc ] && . ~/.bashrc\n' >"$HOME/.bash_profile"
                    ;;
            esac
            ;;
    esac
}

# #747: the login startup file bash reads must keep running after install.
check_profile_survives() {
    [ "$SEED_BASH" = profile_only ] || [ "$SEED_BASH" = nochain ] || return 0
    got="$(bash -l -c 'echo "${FROM_PROFILE:-unset}"' </dev/null 2>/dev/null)"
    if [ "$got" = yes ]; then pass "$1: login startup file still runs"; else fail "$1: login startup file shadowed (FROM_PROFILE=$got)"; fi
}

# One line per candidate rc file: its checksum, or `absent`. The fish prompt
# line records a symlink's text and its target's checksum (#744).
prompt_snapshot() {
    p="$XDG_CONFIG_HOME/fish/functions/fish_prompt.fish"
    if [ -L "$p" ]; then
        printf 'link %s %s\n' "$(readlink "$p")" "$(cksum <"$p" 2>/dev/null || echo dangling)"
    elif [ -e "$p" ]; then
        printf 'file %s\n' "$(cksum <"$p")"
    else
        printf 'absent\n'
    fi
}

rc_snapshot() {
    prompt_snapshot
    for f in "$HOME/.zshrc" "${ZDOTDIR:+$ZDOTDIR/.zshrc}" "$HOME/.zprofile" "$HOME/.zshenv" "$HOME/.zlogin" \
        "$HOME/.bashrc" "$HOME/.bash_profile" "$HOME/.bash_login" "$HOME/.profile" \
        "$XDG_CONFIG_HOME/fish/config.fish"; do
        [ -n "$f" ] || continue
        if [ -e "$f" ]; then printf '%s %s\n' "$(cksum <"$f")" "$f"; else printf 'absent %s\n' "$f"; fi
    done
}

agent_stop() { "$HOME/.local/bin/gpy-agent" stop >/dev/null 2>&1 || true; }

# `check_start LABEL SHELL MODE`: start SHELL interactively (MODE nonlogin or
# login), assert no stderr, the integration loaded, and its entry point
# sourced exactly once.
check_start() {
    label="$1" shell="$2" mode="$3"
    set -- "$shell"
    [ "$mode" = login ] && set -- "$@" -l
    err="$SB/start.err"
    case "$shell" in
        fish) loaded="$("$@" -i -c 'functions -q __gpy_load_theme; and echo loaded' </dev/null 2>"$err")" ;;
        zsh) loaded="$("$@" -d -i -c 'whence -w __gpy_load_theme >/dev/null && echo loaded' </dev/null 2>"$err")" ;;
        bash) loaded="$("$@" -i -c 'declare -F __gpy_load_theme >/dev/null && echo loaded' </dev/null 2>"$err")" ;;
    esac
    # A tty-less interactive bash always prints these two job-control lines;
    # a user's terminal does not, so they are not errors.
    grep -Fv -e 'cannot set terminal process group' -e 'no job control in this shell' "$err" >"$err.f"
    if [ -s "$err.f" ]; then fail "$label: stderr not empty: $(cat "$err.f")"; else pass "$label: no stderr"; fi
    if [ "$loaded" = loaded ]; then pass "$label: gpy loaded"; else fail "$label: gpy not loaded (got '$loaded')"; fi
    trace="$SB/start.trace"
    : >"$trace"
    case "$shell" in
        fish) "$@" -i --profile-startup="$trace" -c true </dev/null >/dev/null 2>&1; pat='> source .*/gpy_init\.fish' ;;
        zsh) "$@" -d -i -x -c true </dev/null >/dev/null 2>"$trace"; pat='source .*/gpy/zsh/gpy\.zsh'"'"'?$' ;;
        bash) "$@" -i -x -c true </dev/null >/dev/null 2>"$trace"; pat='source .*/gpy/bash/gpy\.bash'"'"'?$' ;;
    esac
    count="$(grep -Ec "$pat" "$trace")"
    if [ "$count" = 1 ]; then
        pass "$label: sourced exactly once"
    else
        fail "$label: sourced $count times (expected 1)"
        sed -n '1,20p' "$trace"
    fi
}

# `row INSTALLER SHELL LAYOUT EXPECT`
row() {
    installer="$1" shell="$2" layout="$3" expect="$4" uninstaller="${5:-}"
    name="$installer/$shell/$layout/$expect${uninstaller:+/uninstall.$uninstaller}"
    echo "=== $name ==="
    shell_e2e_init "$ROOT"
    SB="$SHELL_E2E_ROOT"
    SEED_BASH=chain
    SEED_ZSH=present
    SEED_PROMPT=none
    unset ZDOTDIR
    "layout_$layout"
    export HOME XDG_CONFIG_HOME
    [ -z "${ZDOTDIR:-}" ] || export ZDOTDIR
    export XDG_CACHE_HOME="$SB/cache"
    mkdir -p "$HOME"
    export PATH="$HOME/.local/bin:$SB/fakebin:$PATH"
    mkdir -p "$SB/fakebin"
    write_fake_curl "$SB/fakebin"
    seed_user_files "$shell"
    # fish writes its stock config.fish on first run; seed it so running
    # uninstall.fish for a non-fish shell does not look like a leftover.
    if [ "$uninstaller" = fish ] && [ "$shell" != fish ]; then
        seed_user_files fish
    fi
    before="$(rc_snapshot)"

    case "$installer" in
        oneline)
            (cd "$SB" && GPY_SHELL="$shell" GPY_VERSION=v9.9.9-test sh "$ROOT/install-oneline.sh" </dev/null) >"$SB/install.log" 2>&1
            ;;
        install.sh)
            PKG="$SB/pkg"
            mkdir -p "$PKG/bin" "$PKG/scripts"
            cp -R "$ROOT/fish" "$PKG/fish"
            cp "$ROOT/install.sh" "$PKG/install.sh"
            cp "$ROOT/scripts/uninstall.fish" "$ROOT/scripts/uninstall.sh" "$PKG/scripts/"
            cp "$GPY_AGENT" "$PKG/bin/$AGENT_ASSET"
            cp "$GPY_CLI" "$PKG/bin/$CLI_ASSET"
            for asset in "$AGENT_ASSET" "$CLI_ASSET"; do
                printf '%s  %s\n' "$(sha256_of "$PKG/bin/$asset")" "$asset" >"$PKG/bin/$asset.sha256"
            done
            (cd "$PKG" && bash ./install.sh </dev/null) >"$SB/install.log" 2>&1
            ;;
    esac
    rc=$?

    if [ "$expect" = refuse ]; then
        if [ "$rc" -ne 0 ]; then pass "$name: installer refused (exit $rc)"; else fail "$name: installer exited 0 on an unsupported path"; fi
        if [ "$(rc_snapshot)" = "$before" ]; then pass "$name: no rc file touched"; else fail "$name: an rc file changed"; fi
        leftovers="$(find "$XDG_CONFIG_HOME" "$HOME/.local" -iname '*gpy*' 2>/dev/null)"
        if [ -z "$leftovers" ]; then pass "$name: nothing installed"; else fail "$name: refused install left files: $leftovers"; fi
    else
        if [ "$rc" -eq 0 ]; then pass "$name: installer exited 0"; else fail "$name: installer exited $rc"; sed -n '1,40p' "$SB/install.log"; fi
        for mode in nonlogin login; do
            check_start "$name [$mode]" "$shell" "$mode"
        done
        check_profile_survives "$name"
        if [ "$SEED_PROMPT" = symlink ]; then
            moved=""
            for b in "$XDG_CONFIG_HOME"/fish/functions/fish_prompt.fish.*backup.*; do
                [ -L "$b" ] && [ "$(readlink "$b")" = "$HOME/dotfiles/fish_prompt.fish" ] && moved="$b"
            done
            if [ -n "$moved" ]; then pass "$name: foreign symlink moved aside as a symlink"; else fail "$name: no symlink backup of the user's prompt"; fi
        fi
        agent_stop
        if [ "$installer" = install.sh ] || [ "$uninstaller" = fish ]; then
            printf '\n' | fish "$ROOT/scripts/uninstall.fish" >"$SB/uninstall.log" 2>&1
        else
            printf '\n' | GPY_SHELL="$shell" sh "$ROOT/scripts/uninstall.sh" >"$SB/uninstall.log" 2>&1
        fi
        urc=$?
        if [ "$urc" -eq 0 ]; then pass "$name: uninstaller exited 0"; else fail "$name: uninstaller exited $urc"; cat "$SB/uninstall.log"; fi
        if [ "$(rc_snapshot)" = "$before" ]; then
            pass "$name: every rc file byte-identical after uninstall"
        else
            fail "$name: rc files differ after uninstall"
            diff <(printf '%s\n' "$before") <(rc_snapshot)
        fi
    fi
    agent_stop
    shell_e2e_cleanup
    trap - EXIT
}

# --- rows ---------------------------------------------------------------------
row oneline fish plain ok
row oneline zsh plain ok
row oneline bash plain ok

# #746: a space in HOME / XDG_CONFIG_HOME
row oneline fish space ok
row oneline zsh space ok
row oneline bash space ok
row install.sh fish space ok

# #747: bash without a ~/.bashrc must not shadow ~/.profile or skip non-login shells
row oneline bash bash_profile_only ok
row oneline bash bash_none ok
row oneline bash bash_nochain ok

# #748: zsh reads $ZDOTDIR/.zshrc; install and both uninstallers must use it
row oneline zsh zdotdir ok
row oneline zsh zdotdir ok fish
row oneline zsh zdotdir_absent ok
row oneline zsh zdotdir_absent ok fish

# #744: a symlinked fish_prompt.fish survives install + uninstall
row oneline fish fish_prompt_symlink ok
row oneline fish fish_prompt_symlink ok fish
row install.sh fish fish_prompt_symlink ok

# #746: characters double quotes cannot protect are refused up front
row oneline fish quote refuse
row oneline zsh quote refuse
row oneline bash quote refuse
row install.sh fish quote refuse
row oneline zsh dollar refuse
row oneline bash backtick refuse
row oneline fish backslash refuse
row install.sh fish dollar refuse

if [ "$failures" -gt 0 ]; then
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: installer rc matrix"
