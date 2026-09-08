#!/usr/bin/env fish
# tests/fish/uninstall_rc_marker_roundtrip.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression test for #310: install.sh and install-oneline.sh appended a
# plain "# GPY Prompt Enhancement" block (fish) or a bare `source` line
# (zsh/bash) with NO delimiter markers at all, while scripts/uninstall.fish
# looked for a "# >>> GPY prompt init >>>" marker that no installer ever
# wrote -- so uninstalling never removed the rc-file block and every new
# shell errored forever. There was also no uninstaller at all for zsh/bash.
#
# This test proves, for every installer/shell combination in scope:
#   1. install -> uninstall round-trips the rc file byte-identically.
#   2. Running the install step twice in a row is idempotent (no duplicate
#      block).
#   3. Removal does not spill into unrelated content that follows the block
#      (e.g. another tool's config appended later).
#
# Runs the REAL install.sh / install-oneline.sh / uninstall.fish /
# uninstall.sh against a sandboxed $HOME, with network calls stubbed out via
# a fake `curl` on PATH, so no network access or fixture drift is required.

set -l script_dir (dirname (status --filename))
set -l repo_root (cd "$script_dir/../.." && pwd)
cd "$repo_root"

set -g failed 0

function __gpy_test_fail --argument-names msg
    echo "❌ $msg"
    set -g failed 1
end

function __gpy_test_pass --argument-names msg
    echo "✅ $msg"
end

# --- fake curl: stubs every network download install-oneline.sh performs --
# Writes a trivial `#!/bin/sh\nexit 0\n` stub to whatever `-o FILE` target was
# requested, regardless of URL. This lets install-oneline.sh run to completion
# (binary --version check, core-file existence checks) with zero network
# access.
#
# A request with no `-o` is a fetch-to-stdout. For a `<asset>.sha256` URL it
# answers with the digest of that same stub payload, standing in for the
# checksum sidecar every release publishes (#494) -- without it the installer
# correctly refuses to install an unverified binary and this test never
# reaches the rc-file behavior it exists to check.
function __gpy_write_fake_curl --argument-names bin_dir
    printf '%s' '#!/bin/sh
payload_to() {
    printf "#!/bin/sh\nexit 0\n" > "$1"
}

out=""
url=""
prev=""
for arg in "$@"; do
    if [ "$prev" = "-o" ]; then
        out="$arg"
    else
        case "$arg" in
            -*) ;;
            *) url="$arg" ;;
        esac
    fi
    prev="$arg"
done

if [ -n "$out" ]; then
    mkdir -p "$(dirname "$out")"
    payload_to "$out"
    chmod +x "$out"
    exit 0
fi

case "$url" in
    *.sha256)
        tmp="$(mktemp)"
        payload_to "$tmp"
        if command -v sha256sum >/dev/null 2>&1; then
            digest="$(sha256sum "$tmp" | awk "{print \$1}")"
        else
            digest="$(shasum -a 256 "$tmp" | awk "{print \$1}")"
        fi
        rm -f "$tmp"
        base="${url##*/}"
        printf "%s  %s\n" "$digest" "${base%.sha256}"
        ;;
esac
exit 0
' >"$bin_dir/curl"
    chmod +x "$bin_dir/curl"
end

# Write the SHA-256 sidecar install.sh now requires beside each packaged
# binary (#494). Mirrors what scripts/package-release.sh produces.
function __gpy_write_sidecar --argument-names file
    set -l digest
    if command -q sha256sum
        set digest (sha256sum "$file" | awk '{print $1}')
    else
        set digest (shasum -a 256 "$file" | awk '{print $1}')
    end
    printf '%s  %s\n' $digest (basename "$file") >"$file.sha256"
end

# --- shared round-trip assertion -------------------------------------------
# Compares $current_file against $original_file (both real files on disk).
function __gpy_assert_byte_identical --argument-names label original_file current_file
    if diff -q "$original_file" "$current_file" >/dev/null 2>&1
        __gpy_test_pass "$label: byte-identical round trip"
    else
        __gpy_test_fail "$label: round trip changed the rc file"
        echo "--- diff ($original_file vs $current_file) ---"
        diff -u "$original_file" "$current_file"
        echo "--- end diff ---"
    end
end

# ===========================================================================
# Part A: install.sh (fish-only dev installer)
# ===========================================================================

# install.sh installs from the package directory it runs in (bin/ plus the
# fish/ tree). Build one under a temp root rather than writing stub binaries
# into the checkout's gitignored bin/ (#649: no test mutates the checkout).
set -l pkg_a (mktemp -d)
mkdir -p "$pkg_a/bin"
cp -R "$repo_root/fish" "$pkg_a/fish"
cp "$repo_root/install.sh" "$pkg_a/install.sh"
set -l stub_binary_names gpy-agent-linux-x86_64 gpy-agent-linux-aarch64 gpy-agent-macos-x86_64 gpy-agent-macos-aarch64
for name in $stub_binary_names
    printf '#!/bin/sh\nexit 0\n' >"$pkg_a/bin/$name"
    chmod +x "$pkg_a/bin/$name"
    __gpy_write_sidecar "$pkg_a/bin/$name"
end

set -l home_a (mktemp -d)
mkdir -p "$home_a/.config/fish"
set -l original_a "$home_a/original_config.fish"
printf '%s\n' \
    "set -gx EXISTING_VAR before_gpy" \
    "alias ll 'ls -la'" >"$original_a"
cp "$original_a" "$home_a/.config/fish/config.fish"

# A child fish for the cd: a `cd` inside a command substitution changes this
# shell's own directory, and pkg_a is deleted below.
set -l install_a_out (fish -c "cd '$pkg_a'; and env HOME='$home_a' bash ./install.sh" 2>&1)
set -l install_a_status $status

if test $install_a_status -ne 0
    __gpy_test_fail "install.sh exited $install_a_status: $install_a_out"
else
    if grep -qF '# >>> gpy-init >>>' "$home_a/.config/fish/config.fish"
        __gpy_test_pass "install.sh: gpy-init marker present after install"
    else
        __gpy_test_fail "install.sh: gpy-init marker missing after install"
    end

    # Idempotency: running install.sh again must not duplicate the block.
    fish -c "cd '$pkg_a'; and env HOME='$home_a' bash ./install.sh" >/dev/null 2>&1
    set -l marker_count_a (grep -cF '# >>> gpy-init >>>' "$home_a/.config/fish/config.fish")
    if test "$marker_count_a" = 1
        __gpy_test_pass "install.sh: append is idempotent"
    else
        __gpy_test_fail "install.sh: not idempotent, found $marker_count_a marker occurrences after running twice"
    end

    # Uninstall (fish's own uninstaller) and diff against the pristine original.
    printf '\n' | env HOME=$home_a fish scripts/uninstall.fish >/dev/null 2>&1
    __gpy_assert_byte_identical "install.sh -> uninstall.fish" "$original_a" "$home_a/.config/fish/config.fish"
end

rm -rf "$pkg_a"
rm -rf "$home_a"

# ===========================================================================
# Part B: install-oneline.sh (fish / zsh / bash)
# ===========================================================================

function __gpy_test_install_oneline_roundtrip --argument-names shell_name rc_relpath uninstaller_kind
    set -l home_b (mktemp -d)
    set -l fake_bin (mktemp -d)
    __gpy_write_fake_curl "$fake_bin"

    set -l rc_file "$home_b/$rc_relpath"
    mkdir -p (dirname "$rc_file")

    set -l original_b "$home_b/original_rc"
    printf '%s\n' \
        "export EXISTING_VAR=before_gpy" \
        "alias ll='ls -la'" >"$original_b"
    cp "$original_b" "$rc_file"

    set -l env_prefix HOME=$home_b GPY_SHELL=$shell_name GPY_VERSION=test-version PATH="$fake_bin"":$PATH"

    set -l out1 (env $env_prefix sh install-oneline.sh 2>&1)
    set -l status1 $status
    if test $status1 -ne 0
        __gpy_test_fail "install-oneline.sh ($shell_name) exited $status1: $out1"
        rm -rf "$home_b" "$fake_bin"
        return
    end

    if grep -qF '# >>> gpy-init >>>' "$rc_file"
        __gpy_test_pass "install-oneline.sh ($shell_name): gpy-init marker present after install"
    else
        __gpy_test_fail "install-oneline.sh ($shell_name): gpy-init marker missing after install"
    end

    # Idempotency
    env $env_prefix sh install-oneline.sh >/dev/null 2>&1
    set -l marker_count_b (grep -cF '# >>> gpy-init >>>' "$rc_file")
    if test "$marker_count_b" = 1
        __gpy_test_pass "install-oneline.sh ($shell_name): append is idempotent"
    else
        __gpy_test_fail "install-oneline.sh ($shell_name): not idempotent, found $marker_count_b marker occurrences after running twice"
    end

    # Uninstall
    if test "$uninstaller_kind" = fish
        printf '\n' | env HOME=$home_b fish scripts/uninstall.fish >/dev/null 2>&1
    else
        printf '\n' | env HOME=$home_b GPY_SHELL=$shell_name sh scripts/uninstall.sh >/dev/null 2>&1
    end

    __gpy_assert_byte_identical "install-oneline.sh ($shell_name) -> uninstall" "$original_b" "$rc_file"

    rm -rf "$home_b" "$fake_bin"
end

__gpy_test_install_oneline_roundtrip fish .config/fish/config.fish fish
__gpy_test_install_oneline_roundtrip zsh .zshrc sh
__gpy_test_install_oneline_roundtrip bash .bashrc sh

# ===========================================================================
# Part C: removal does not spill into unrelated trailing content
# ===========================================================================
#
# Simulates a block that was installed earlier, with another tool's config
# appended below it afterwards -- proving the delete range is bounded by the
# close marker and doesn't consume anything past it.

set -l home_c (mktemp -d)
set -l rc_file_fish "$home_c/.config/fish/config.fish"
mkdir -p (dirname "$rc_file_fish")
printf '%s\n' \
    "set -gx EXISTING_VAR before_gpy" \
    "" \
    "# >>> gpy-init >>>" \
    "# GPY Prompt Enhancement" \
    "if status is-interactive" \
    "    source ~/.config/fish/gpy/conf.d/gpy_init.fish" \
    end \
    "# <<< gpy-init <<<" \
    "set -gx ANOTHER_TOOL after_gpy" >"$rc_file_fish"

printf '\n' | env HOME=$home_c fish scripts/uninstall.fish >/dev/null 2>&1

set -l expected_fish "$home_c/expected"
printf '%s\n' \
    "set -gx EXISTING_VAR before_gpy" \
    "set -gx ANOTHER_TOOL after_gpy" >"$expected_fish"

if grep -qF gpy-init "$rc_file_fish"
    __gpy_test_fail "uninstall.fish trailing-content: gpy-init block still present after removal"
else
    __gpy_test_pass "uninstall.fish trailing-content: gpy-init block removed"
end
__gpy_assert_byte_identical "uninstall.fish trailing-content: surrounding content preserved" "$expected_fish" "$rc_file_fish"
rm -rf "$home_c"

set -l home_d (mktemp -d)
set -l rc_file_zsh "$home_d/.zshrc"
printf '%s\n' \
    "export EXISTING_VAR=before_gpy" \
    "" \
    "# >>> gpy-init >>>" \
    "# GPY Prompt Enhancement" \
    "source ~/.config/gpy/zsh/gpy.zsh" \
    "# <<< gpy-init <<<" \
    "export ANOTHER_TOOL=after_gpy" >"$rc_file_zsh"

printf '\n' | env HOME=$home_d GPY_SHELL=zsh sh scripts/uninstall.sh >/dev/null 2>&1

set -l expected_zsh "$home_d/expected"
printf '%s\n' \
    "export EXISTING_VAR=before_gpy" \
    "export ANOTHER_TOOL=after_gpy" >"$expected_zsh"

if grep -qF gpy-init "$rc_file_zsh"
    __gpy_test_fail "uninstall.sh trailing-content: gpy-init block still present after removal"
else
    __gpy_test_pass "uninstall.sh trailing-content: gpy-init block removed"
end
__gpy_assert_byte_identical "uninstall.sh trailing-content: surrounding content preserved" "$expected_zsh" "$rc_file_zsh"
rm -rf "$home_d"

# ===========================================================================
# Part D: XDG_CONFIG_HOME/XDG_CACHE_HOME-respecting install/uninstall (#615)
# ===========================================================================
#
# install-oneline.sh used to hardcode $HOME/.config for fish's shell files,
# completions, and config.fish regardless of XDG_CONFIG_HOME, and
# scripts/uninstall.sh hardcoded $HOME/.config/gpy and $HOME/.cache/gpy for
# the agent's own config/cache directories -- missing the real directories
# for anyone with either XDG variable set (gpy-agent/src/paths.rs
# config_root_for/cache_root_for is the source of truth both now follow).
# This proves the integration lands under $XDG_CONFIG_HOME, nothing leaks
# to $HOME/.config, and uninstall removes the XDG locations.

function __gpy_assert_no_default_config_dir --argument-names label home_dir
    if test -e "$home_dir/.config"
        __gpy_test_fail "$label: unexpected content under \$HOME/.config"
        find "$home_dir/.config"
    else
        __gpy_test_pass "$label: nothing created under \$HOME/.config"
    end
end

# --- fish: install-oneline.sh + scripts/uninstall.fish ----------------------
set -l home_e (mktemp -d)
set -l xdg_config_e "$home_e/xdg-config"
set -l xdg_cache_e "$home_e/xdg-cache"
set -l fake_bin_e (mktemp -d)
__gpy_write_fake_curl "$fake_bin_e"

set -l env_prefix_e HOME=$home_e XDG_CONFIG_HOME=$xdg_config_e XDG_CACHE_HOME=$xdg_cache_e GPY_SHELL=fish GPY_VERSION=test-version PATH="$fake_bin_e"":$PATH"

set -l out_e (env $env_prefix_e sh install-oneline.sh 2>&1)
set -l status_e $status

if test $status_e -ne 0
    __gpy_test_fail "install-oneline.sh (fish, XDG_CONFIG_HOME) exited $status_e: $out_e"
else
    if test -f "$xdg_config_e/fish/config.fish"; and grep -qF '# >>> gpy-init >>>' "$xdg_config_e/fish/config.fish"
        __gpy_test_pass "install-oneline.sh (fish, XDG_CONFIG_HOME): config.fish under \$XDG_CONFIG_HOME/fish"
    else
        __gpy_test_fail "install-oneline.sh (fish, XDG_CONFIG_HOME): config.fish missing/unmarked under \$XDG_CONFIG_HOME/fish"
    end

    if test -d "$xdg_config_e/fish/gpy/core"
        __gpy_test_pass "install-oneline.sh (fish, XDG_CONFIG_HOME): shell files under \$XDG_CONFIG_HOME/fish/gpy"
    else
        __gpy_test_fail "install-oneline.sh (fish, XDG_CONFIG_HOME): shell files missing under \$XDG_CONFIG_HOME/fish/gpy"
    end

    __gpy_assert_no_default_config_dir "install-oneline.sh (fish, XDG_CONFIG_HOME)" "$home_e"

    # install-oneline.sh never creates the agent's own config/cache
    # directories itself (gpy-agent does that lazily on first run); simulate
    # them under the XDG locations so uninstall.fish has something real to
    # remove there too.
    mkdir -p "$xdg_config_e/gpy" "$xdg_cache_e/gpy"
    touch "$xdg_config_e/gpy/config.toml" "$xdg_cache_e/gpy/agent.log"

    # Uninstall must target the same XDG-resolved fish config dir, and the
    # same XDG-resolved agent config/cache dirs (#615).
    printf '\n' | env HOME=$home_e XDG_CONFIG_HOME=$xdg_config_e XDG_CACHE_HOME=$xdg_cache_e fish scripts/uninstall.fish >/dev/null 2>&1

    if test -d "$xdg_config_e/fish/gpy"
        __gpy_test_fail "uninstall.fish (XDG_CONFIG_HOME): fish/gpy still present after uninstall"
    else
        __gpy_test_pass "uninstall.fish (XDG_CONFIG_HOME): fish/gpy removed"
    end

    if test -d "$xdg_config_e/gpy"
        __gpy_test_fail "uninstall.fish (XDG_CONFIG_HOME): \$XDG_CONFIG_HOME/gpy still present after uninstall"
    else
        __gpy_test_pass "uninstall.fish (XDG_CONFIG_HOME): \$XDG_CONFIG_HOME/gpy removed"
    end

    if test -d "$xdg_cache_e/gpy"
        __gpy_test_fail "uninstall.fish (XDG_CACHE_HOME): \$XDG_CACHE_HOME/gpy still present after uninstall"
    else
        __gpy_test_pass "uninstall.fish (XDG_CACHE_HOME): \$XDG_CACHE_HOME/gpy removed"
    end
end

rm -rf "$home_e" "$fake_bin_e"

# --- bash: install-oneline.sh + scripts/uninstall.sh ------------------------
set -l home_f (mktemp -d)
set -l xdg_config_f "$home_f/xdg-config"
set -l xdg_cache_f "$home_f/xdg-cache"
set -l fake_bin_f (mktemp -d)
__gpy_write_fake_curl "$fake_bin_f"

set -l env_prefix_f HOME=$home_f XDG_CONFIG_HOME=$xdg_config_f XDG_CACHE_HOME=$xdg_cache_f GPY_SHELL=bash GPY_VERSION=test-version PATH="$fake_bin_f"":$PATH"

set -l out_f (env $env_prefix_f sh install-oneline.sh 2>&1)
set -l status_f $status

if test $status_f -ne 0
    __gpy_test_fail "install-oneline.sh (bash, XDG_CONFIG_HOME) exited $status_f: $out_f"
else
    if test -d "$xdg_config_f/gpy/bash/core"
        __gpy_test_pass "install-oneline.sh (bash, XDG_CONFIG_HOME): shell files under \$XDG_CONFIG_HOME/gpy/bash"
    else
        __gpy_test_fail "install-oneline.sh (bash, XDG_CONFIG_HOME): shell files missing under \$XDG_CONFIG_HOME/gpy/bash"
    end

    __gpy_assert_no_default_config_dir "install-oneline.sh (bash, XDG_CONFIG_HOME)" "$home_f"

    # install-oneline.sh never creates the agent's own config/cache
    # directories itself (gpy-agent does that lazily on first run); simulate
    # them under the XDG locations so uninstall.sh has something real to
    # remove there.
    mkdir -p "$xdg_config_f/gpy" "$xdg_cache_f/gpy"
    touch "$xdg_config_f/gpy/config.toml" "$xdg_cache_f/gpy/agent.log"

    printf '\n' | env HOME=$home_f XDG_CONFIG_HOME=$xdg_config_f XDG_CACHE_HOME=$xdg_cache_f GPY_SHELL=bash sh scripts/uninstall.sh >/dev/null 2>&1

    if test -d "$xdg_config_f/gpy"
        __gpy_test_fail "uninstall.sh (XDG): \$XDG_CONFIG_HOME/gpy still present after uninstall"
    else
        __gpy_test_pass "uninstall.sh (XDG): \$XDG_CONFIG_HOME/gpy removed (shell files and config.toml)"
    end

    if test -d "$xdg_cache_f/gpy"
        __gpy_test_fail "uninstall.sh (XDG): \$XDG_CACHE_HOME/gpy still present after uninstall"
    else
        __gpy_test_pass "uninstall.sh (XDG): \$XDG_CACHE_HOME/gpy removed"
    end
end

rm -rf "$home_f" "$fake_bin_f"

# ===========================================================================

if test $failed -eq 1
    exit 1
end

echo "✅ install/uninstall gpy-init marker round trips are byte-identical across fish/zsh/bash"
