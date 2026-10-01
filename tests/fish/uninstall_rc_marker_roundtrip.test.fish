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
# Part C: Multi-shell global uninstall regressions (#671)
# ===========================================================================

# --- 1. Bash + Zsh installed; run sh uninstaller with GPY_SHELL=bash and zsh ---
for gpy_shell_val in bash zsh
    set -l h_multi (mktemp -d)
    set -l xdg_c_multi "$h_multi/xdg-config"
    set -l xdg_cache_multi "$h_multi/xdg-cache"
    set -l xdg_rt_multi "$h_multi/xdg-runtime"
    mkdir -p "$xdg_c_multi/gpy/bash" "$xdg_c_multi/gpy/zsh" "$xdg_cache_multi/gpy" "$xdg_rt_multi"
    set -l bash_entry "$xdg_c_multi/gpy/bash/gpy.bash"
    set -l zsh_entry "$xdg_c_multi/gpy/zsh/gpy.zsh"
    printf '%s\n' "# bash entry" >"$bash_entry"
    printf '%s\n' "# zsh entry" >"$zsh_entry"

    set -l bashrc "$h_multi/.bashrc"
    set -l zshrc "$h_multi/.zshrc"
    printf '%s\n' "export PRE_BASH=1" "" "# >>> gpy-init >>>" "source \"$bash_entry\"" "# <<< gpy-init <<<" "export POST_BASH=1" >"$bashrc"
    printf '%s\n' "export PRE_ZSH=1" "" "# >>> gpy-init >>>" "source \"$zsh_entry\"" "# <<< gpy-init <<<" "export POST_ZSH=1" >"$zshrc"

    printf '\n' | env HOME=$h_multi XDG_CONFIG_HOME=$xdg_c_multi XDG_CACHE_HOME=$xdg_cache_multi XDG_RUNTIME_DIR=$xdg_rt_multi GPY_CONFIG_PATH="$xdg_c_multi/gpy/config.toml" GPY_AGENT_SOCKET_PATH="$xdg_rt_multi/gpy.sock" GPY_SHELL=$gpy_shell_val sh scripts/uninstall.sh >/dev/null 2>&1

    if not grep -qF gpy-init "$bashrc"; and not grep -qF gpy-init "$zshrc"
        __gpy_test_pass "#671 (sh uninstaller, GPY_SHELL=$gpy_shell_val): neither rc keeps gpy-init block"
    else
        __gpy_test_fail "#671 (sh uninstaller, GPY_SHELL=$gpy_shell_val): gpy-init block remained in bashrc or zshrc"
    end

    set -l exp_b "$h_multi/exp_b"
    set -l exp_z "$h_multi/exp_z"
    printf '%s\n' "export PRE_BASH=1" "export POST_BASH=1" >"$exp_b"
    printf '%s\n' "export PRE_ZSH=1" "export POST_ZSH=1" >"$exp_z"
    __gpy_assert_byte_identical "#671 (sh uninstaller, GPY_SHELL=$gpy_shell_val): bashrc bytes preserved" "$exp_b" "$bashrc"
    __gpy_assert_byte_identical "#671 (sh uninstaller, GPY_SHELL=$gpy_shell_val): zshrc bytes preserved" "$exp_z" "$zshrc"
    rm -rf "$h_multi"
end

# --- 2. Fish + Bash + Zsh installed: run each entry point in separate fixtures ---
for uninstaller_entry in "fish:scripts/uninstall.fish" "sh:scripts/uninstall.sh"
    set -l parts (string split ':' -- "$uninstaller_entry")
    set -l u_type $parts[1]
    set -l u_script $parts[2]

    set -l h_all (mktemp -d)
    set -l xdg_c_all "$h_all/xdg-config"
    set -l xdg_cache_all "$h_all/xdg-cache"
    set -l xdg_rt_all "$h_all/xdg-runtime"
    mkdir -p "$xdg_c_all/gpy/bash" "$xdg_c_all/gpy/zsh" "$xdg_c_all/fish/functions" "$xdg_c_all/fish/conf.d" "$xdg_c_all/fish/completions" "$xdg_c_all/fish/gpy/functions" "$xdg_cache_all/gpy" "$xdg_rt_all"

    # Shared entries and rc files
    set -l bashrc_all "$h_all/.bashrc"
    set -l bashprof_all "$h_all/.bash_profile"
    set -l zshrc_all "$h_all/.zshrc"
    set -l fishrc_all "$xdg_c_all/fish/config.fish"

    printf '%s\n' "alias b=1" "" "# >>> gpy-init >>>" "source gpy.bash" "# <<< gpy-init <<<" "alias b_post=1" >"$bashrc_all"
    printf '%s\n' "alias bp=1" "" "# >>> gpy-init >>>" "source gpy.bash" "# <<< gpy-init <<<" "alias bp_post=1" >"$bashprof_all"
    printf '%s\n' "alias z=1" "" "# >>> gpy-init >>>" "source gpy.zsh" "# <<< gpy-init <<<" "alias z_post=1" >"$zshrc_all"
    printf '%s\n' "set -g f 1" "" "# >>> gpy-init >>>" "source gpy.fish" "# <<< gpy-init <<<" "set -g f_post 1" >"$fishrc_all"

    # Fish integration files
    set -l gpy_prompt_target "$xdg_c_all/fish/gpy/functions/fish_prompt.fish"
    printf '%s\n' "function fish_prompt; echo gpy; end" >"$gpy_prompt_target"
    ln -s "$gpy_prompt_target" "$xdg_c_all/fish/functions/fish_prompt.fish"
    set -l fish_backup "$xdg_c_all/fish/functions/fish_prompt.fish.backup.20260101_000000"
    printf '%s\n' "function fish_prompt; echo custom_restored; end" >"$fish_backup"

    set -l gpy_init_fish "$xdg_c_all/fish/conf.d/gpy_init.fish"
    printf '%s\n' "# gpy init" >"$gpy_init_fish"
    set -l gpy_comp_fish "$xdg_c_all/fish/completions/gpy.fish"
    printf '%s\n' "# gpy comp" >"$gpy_comp_fish"
    set -l custom_comp "$xdg_c_all/fish/completions/other.fish"
    printf '%s\n' "# other comp" >"$custom_comp"

    if test "$u_type" = fish
        printf '\n' | env HOME=$h_all XDG_CONFIG_HOME=$xdg_c_all XDG_CACHE_HOME=$xdg_cache_all XDG_RUNTIME_DIR=$xdg_rt_all GPY_CONFIG_PATH="$xdg_c_all/gpy/config.toml" GPY_AGENT_SOCKET_PATH="$xdg_rt_all/gpy.sock" fish "$u_script" >/dev/null 2>&1
    else
        printf '\n' | env HOME=$h_all XDG_CONFIG_HOME=$xdg_c_all XDG_CACHE_HOME=$xdg_cache_all XDG_RUNTIME_DIR=$xdg_rt_all GPY_CONFIG_PATH="$xdg_c_all/gpy/config.toml" GPY_AGENT_SOCKET_PATH="$xdg_rt_all/gpy.sock" sh "$u_script" >/dev/null 2>&1
    end

    # Verify all 4 rc files cleaned of gpy-init
    set -l rcs_cleaned 1
    for f in "$bashrc_all" "$bashprof_all" "$zshrc_all" "$fishrc_all"
        if grep -qF gpy-init "$f"
            set rcs_cleaned 0
        end
    end
    if test $rcs_cleaned -eq 1
        __gpy_test_pass "#671 ($u_type uninstaller): all 4 rc locations cleaned"
    else
        __gpy_test_fail "#671 ($u_type uninstaller): one or more rc files still have gpy-init"
    end

    # Verify fish prompt restored and backup consumed
    set -l active_prompt "$xdg_c_all/fish/functions/fish_prompt.fish"
    if test -f "$active_prompt"; and not test -L "$active_prompt"; and test (cat "$active_prompt") = "function fish_prompt; echo custom_restored; end"; and not test -e "$fish_backup"
        __gpy_test_pass "#671 ($u_type uninstaller): fish prompt restored from backup"
    else
        __gpy_test_fail "#671 ($u_type uninstaller): fish prompt restoration failed"
    end

    # Verify gpy init and completions removed, but unrelated completion untouched
    if not test -e "$gpy_init_fish"; and not test -e "$gpy_comp_fish"; and test -f "$custom_comp"
        __gpy_test_pass "#671 ($u_type uninstaller): GPY fish files removed, unrelated completion untouched"
    else
        __gpy_test_fail "#671 ($u_type uninstaller): fish completions/conf.d cleanup failed"
    end

    rm -rf "$h_all"
end

# --- 3. Both .bashrc and .bash_profile cleaned; missing file not created ---
set -l h_bash_only (mktemp -d)
set -l xdg_c_bo "$h_bash_only/xdg-config"
mkdir -p "$xdg_c_bo"
set -l bashrc_only "$h_bash_only/.bashrc"
printf '%s\n' prefix "" "# >>> gpy-init >>>" "source gpy.bash" "# <<< gpy-init <<<" suffix >"$bashrc_only"
printf '\n' | env HOME=$h_bash_only XDG_CONFIG_HOME=$xdg_c_bo sh scripts/uninstall.sh >/dev/null 2>&1
set -l exp_bo "$h_bash_only/exp_bo"
printf '%s\n' prefix suffix >"$exp_bo"
__gpy_assert_byte_identical "#671: .bashrc cleaned byte-for-byte" "$exp_bo" "$bashrc_only"
if not test -e "$h_bash_only/.bash_profile"
    __gpy_test_pass "#671: missing .bash_profile not created"
else
    __gpy_test_fail "#671: .bash_profile erroneously created"
end
rm -rf "$h_bash_only"
# --- 4. Multiple complete blocks, trailing content, and incomplete marker ---
set -l h_blocks (mktemp -d)
set -l xdg_c_bl "$h_blocks/xdg-config"
mkdir -p "$xdg_c_bl"
set -l zshrc_blocks "$h_blocks/.zshrc"
printf '%s\n' \
    head \
    "" \
    "# >>> gpy-init >>>" \
    "first block" \
    "# <<< gpy-init <<<" \
    mid \
    "" \
    "# >>> gpy-init >>>" \
    "second block" \
    "# <<< gpy-init <<<" \
    tail \
    "" \
    "# >>> gpy-init >>>" \
    "incomplete marker without end" >"$zshrc_blocks"

printf '\n' | env HOME=$h_blocks XDG_CONFIG_HOME=$xdg_c_bl sh scripts/uninstall.sh >/dev/null 2>&1
set -l exp_zb "$h_blocks/exp_zb"
printf '%s\n' head mid tail "" "# >>> gpy-init >>>" "incomplete marker without end" >"$exp_zb"
__gpy_assert_byte_identical "#671: multiple complete blocks removed and incomplete marker preserved" "$exp_zb" "$zshrc_blocks"
rm -rf "$h_blocks"
# --- 5. Roots with spaces: operates cleanly without split-path errors ---
set -l h_space (mktemp -d "/tmp/gpy uninst space home.XXXXXX")
set -l xdg_c_space (mktemp -d "/tmp/gpy uninst space config.XXXXXX")
set -l xdg_cache_space (mktemp -d "/tmp/gpy uninst space cache.XXXXXX")
set -l xdg_rt_space (mktemp -d "/tmp/gpy uninst space rt.XXXXXX")
mkdir -p "$xdg_c_space/fish/functions" "$xdg_c_space/gpy/bash"
set -l bashrc_space "$h_space/.bashrc"
set -l fish_prompt_space "$xdg_c_space/fish/functions/fish_prompt.fish"
set -l fish_backup_space "$xdg_c_space/fish/functions/fish_prompt.fish.backup.20260101_000000"
printf '%s\n' start "" "# >>> gpy-init >>>" "source gpy.bash" "# <<< gpy-init <<<" end >"$bashrc_space"
printf '%s\n' "function fish_prompt; echo space_restored; end" >"$fish_backup_space"

printf '\n' | env HOME=$h_space XDG_CONFIG_HOME=$xdg_c_space XDG_CACHE_HOME=$xdg_cache_space XDG_RUNTIME_DIR=$xdg_rt_space GPY_CONFIG_PATH="$xdg_c_space/gpy/config.toml" GPY_AGENT_SOCKET_PATH="$xdg_rt_space/gpy.sock" sh scripts/uninstall.sh >/dev/null 2>&1
set -l exp_sp "$h_space/exp_sp"
printf '%s\n' start end >"$exp_sp"
__gpy_assert_byte_identical "#671: paths with spaces rc bytes preserved" "$exp_sp" "$bashrc_space"
if test (cat "$fish_prompt_space") = "function fish_prompt; echo space_restored; end"
    __gpy_test_pass "#671: paths with spaces fish prompt restored"
else
    __gpy_test_fail "#671: paths with spaces fish prompt restoration failed"
end
rm -rf "$h_space" "$xdg_c_space" "$xdg_cache_space" "$xdg_rt_space"

# ===========================================================================

if test $failed -eq 1
    exit 1
end

echo "✅ install/uninstall gpy-init marker round trips are byte-identical across fish/zsh/bash"
