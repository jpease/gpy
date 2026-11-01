#!/usr/bin/env fish
# tests/fish/install_dev_shadowing_binaries.test.fish
# SPDX-License-Identifier: GPL-3.0-or-later
#
# install-dev.fish's _warn_shadowing_binaries only reports: it prints a
# `warn:` line with an `rm` command for every gpy/gpy-agent earlier on PATH
# than the install dir, and deletes nothing:
#   (a) a stale copy of ours under $HOME is kept and warned about
#   (b) an unrelated tool that merely shares the name is kept and warned about
#   (c) a tool-manager shim (a */shims/* path) is kept, with no warning
#   (d) the install dir's own binaries and a copy AFTER it are untouched,
#       with no warning for the later copy
#   (e) with the install dir absent from PATH, the copy is kept, the `Note:`
#       line is printed and there is no `comes before` warning
#   (f) a shadowing copy outside $HOME is kept and warned about
#   (g) _config_home honours XDG_CONFIG_HOME
#
# The function is sourced from the real install-dev.fish (run with
# --files-only against a throwaway HOME so sourcing has no side effects on
# this machine), the way install_dev_stop_running_agent.test.fish does.
# Every stub lives under the sandbox, and each driver runs under `env -i`
# with only system dirs on PATH, so an exported `fish_user_paths` or a real
# ~/.local/bin from the caller can never reach a candidate.

set -l script_dir (dirname (status filename))
set -l repo_root (realpath "$script_dir/../..")
source "$repo_root/tests/lib/test_helpers.fish"
init_test_env

set -l sandbox (mktemp -d)
set -l home_dir "$sandbox/home"
mkdir -p "$home_dir"
mkdir -p "$home_dir/.local/bin"
printf '#!/bin/sh\nexit 0\n' >"$home_dir/.local/bin/gpy"
chmod +x "$home_dir/.local/bin/gpy"

# PATH layout: early/ (ours, stale, under $HOME) : outside/ (ours, outside
# $HOME) : other/ (unrelated tool) : tools/shims/ (shim) : install/ (the
# target) : late/ (ours, after target).
set -l early "$home_dir/.cargo/bin"
set -l outside "$sandbox/outside"
set -l other "$sandbox/other"
set -l shims "$sandbox/tools/shims"
set -l install "$sandbox/install"
set -l late "$sandbox/late"
# Only on PATH in the install-dir-absent run (e).
set -l off_path_copy "$home_dir/.cargo/bin2"
mkdir -p $early $outside $other $shims $install $late $off_path_copy

function write_stub --argument-names path version_line
    printf '#!/bin/sh\necho "%s"\n' "$version_line" >"$path"
    chmod +x "$path"
end
write_stub "$early/gpy-agent" "gpy-agent 0.0.1"
write_stub "$early/gpy" "gpy 0.0.1"
write_stub "$outside/gpy-agent" "gpy-agent 0.0.1"
write_stub "$other/gpy-agent" "other-tool 1.0"
write_stub "$shims/gpy-agent" "gpy-agent 0.0.1"
write_stub "$install/gpy-agent" "gpy-agent 9.9.9"
write_stub "$install/gpy" "gpy 9.9.9"
write_stub "$late/gpy-agent" "gpy-agent 0.0.2"
write_stub "$off_path_copy/gpy-agent" "gpy-agent 0.0.1"

set -l safe_path (string join : /usr/bin /bin /usr/sbin /sbin /opt/homebrew/bin /usr/local/bin)

# Writes a driver that sources install-dev.fish, sets PATH to the given
# entries, and calls _warn_shadowing_binaries on the install dir.
function write_driver --argument-names driver repo_root home_dir safe_path install
    printf '%s\n' \
        "cd '$repo_root'" \
        "set -gx HOME '$home_dir'" \
        "set -gx XDG_CONFIG_HOME '$home_dir/.config'" \
        "set -gx XDG_DATA_HOME '$home_dir/.local/share'" \
        "set -gx XDG_CACHE_HOME '$home_dir/.cache'" \
        "set -gx PATH '$safe_path'" \
        "source install-dev.fish --files-only --allow-shadow >/dev/null 2>&1" \
        "set -gx PATH "(string escape -- $argv[6..-1] | string join ' ') \
        "_warn_shadowing_binaries '$install'" >"$driver"
end

set -l driver "$sandbox/driver.fish"
write_driver $driver $repo_root $home_dir $safe_path $install \
    $early $outside $other $shims $install $late $safe_path

set -l out (env -i HOME=$home_dir PATH=$safe_path fish --no-config "$driver" 2>&1)
set -l rc $status
if test $rc -ne 0
    fail "driver exited $rc: $out"
end

# (a) the stale copies of ours are kept and each gets the warning
if test -e "$early/gpy-agent"; and test -e "$early/gpy"
    pass "stale gpy and gpy-agent earlier on PATH were kept"
else
    fail "a binary earlier on PATH was deleted: "(ls $early)
end
for name in gpy gpy-agent
    if string match -q "*$early/$name comes before $install*rm '$early/$name'*" -- (string join ' ' $out)
        pass "the shadowing $name under \$HOME was warned about"
    else
        fail "no warning with an rm command for $early/$name in: $out"
    end
end

# (b) an unrelated tool sharing the name is kept and gets the same warning
if test -e "$other/gpy-agent"
    pass "an unrelated tool named gpy-agent was kept"
else
    fail "the unrelated tool at $other/gpy-agent was deleted"
end
if string match -q "*$other/gpy-agent comes before*" -- (string join ' ' $out)
    pass "the unrelated tool was warned about"
else
    fail "no warning for the unrelated tool in: $out"
end

# (c) a tool-manager shim is kept and not named in a warning
if test -e "$shims/gpy-agent"
    pass "a */shims/* entry was kept"
else
    fail "the shim at $shims/gpy-agent was deleted"
end
if string match -q "*$shims/gpy-agent*" -- (string join ' ' $out)
    fail "the shim was named in a warning: $out"
else
    pass "the shim produced no warning"
end

# (d) the install dir itself and anything after it are untouched and unreported
if test -e "$install/gpy-agent"; and test -e "$install/gpy"
    pass "the install dir's own binaries are untouched"
else
    fail "the install dir lost a binary: "(ls $install)
end
if test -e "$late/gpy-agent"
    pass "a copy after the install dir on PATH was kept"
else
    fail "the copy after the install dir was deleted"
end
if string match -q "*$late/gpy-agent*" -- (string join ' ' $out)
    fail "the later copy was named in a warning: $out"
else
    pass "the later copy produced no warning"
end

# (f) a shadowing copy outside $HOME is kept and warned about
if test -e "$outside/gpy-agent"
    pass "a shadowing copy outside \$HOME was kept"
else
    fail "the copy outside \$HOME at $outside/gpy-agent was deleted"
end
if string match -q "*$outside/gpy-agent comes before*rm '$outside/gpy-agent'*" -- (string join ' ' $out)
    pass "the copy outside \$HOME was warned about"
else
    fail "no warning for $outside/gpy-agent in: $out"
end

# (e) install dir absent from PATH: copy kept, only the Note line is printed
set -l driver_off "$sandbox/driver_off.fish"
write_driver $driver_off $repo_root $home_dir $safe_path $install \
    $off_path_copy $safe_path
set -l out_off (env -i HOME=$home_dir PATH=$safe_path fish --no-config "$driver_off" 2>&1)
set -l rc_off $status
if test $rc_off -ne 0
    fail "off-PATH driver exited $rc_off: $out_off"
end
if test -e "$off_path_copy/gpy-agent"
    pass "with the install dir off PATH, the copy on PATH was kept"
else
    fail "with the install dir off PATH, $off_path_copy/gpy-agent was deleted"
end
if string match -q "*Note: $install is not on PATH*" -- (string join ' ' $out_off)
    pass "with the install dir off PATH, the Note line was printed"
else
    fail "with the install dir off PATH, no Note line in: $out_off"
end
if string match -q "*comes before*" -- (string join ' ' $out_off)
    fail "with the install dir off PATH, a shadow warning was printed: $out_off"
else
    pass "with the install dir off PATH, no shadow warning was printed"
end

# (g) _config_home: $HOME/.config by default, $XDG_CONFIG_HOME when set
set -l driver_cfg "$sandbox/driver_cfg.fish"
printf '%s\n' \
    "cd '$repo_root'" \
    "set -gx HOME '$home_dir'" \
    "set -gx XDG_CACHE_HOME '$home_dir/.cache'" \
    "set -gx PATH '$safe_path'" \
    "set -e XDG_CONFIG_HOME" \
    "source install-dev.fish --files-only --allow-shadow >/dev/null 2>&1" \
    "echo default=(_config_home)" \
    "set -gx XDG_CONFIG_HOME '$sandbox/xdg'" \
    "echo xdg=(_config_home)" >"$driver_cfg"
set -l out_cfg (env -i HOME=$home_dir PATH=$safe_path fish --no-config "$driver_cfg" 2>&1)
if contains -- "default=$home_dir/.config" $out_cfg
    pass "_config_home defaults to \$HOME/.config"
else
    fail "_config_home default wrong in: $out_cfg"
end
if contains -- "xdg=$sandbox/xdg" $out_cfg
    pass "_config_home honours XDG_CONFIG_HOME"
else
    fail "_config_home ignored XDG_CONFIG_HOME in: $out_cfg"
end

# (h) _data_root: $HOME/.local/share/gpy by default, $XDG_DATA_HOME/gpy when it
# is absolute, the default again when it is relative
set -l driver_data "$sandbox/driver_data.fish"
printf '%s\n' \
    "cd '$repo_root'" \
    "set -gx HOME '$home_dir'" \
    "set -gx XDG_CACHE_HOME '$home_dir/.cache'" \
    "set -gx PATH '$safe_path'" \
    "set -e XDG_DATA_HOME" \
    "source install-dev.fish --files-only --allow-shadow >/dev/null 2>&1" \
    "echo default=(_data_root)" \
    "set -gx XDG_DATA_HOME '$sandbox/xdg-data'" \
    "echo xdg=(_data_root)" \
    "set -gx XDG_DATA_HOME relative/data" \
    "echo relative=(_data_root)" >"$driver_data"
set -l out_data (env -i HOME=$home_dir PATH=$safe_path fish --no-config "$driver_data" 2>&1)
contains -- "default=$home_dir/.local/share/gpy" $out_data
and pass "_data_root defaults to \$HOME/.local/share/gpy"
or fail "_data_root default wrong in: $out_data"
contains -- "xdg=$sandbox/xdg-data/gpy" $out_data
and pass "_data_root honours an absolute XDG_DATA_HOME"
or fail "_data_root ignored XDG_DATA_HOME in: $out_data"
contains -- "relative=$home_dir/.local/share/gpy" $out_data
and pass "_data_root ignores a relative XDG_DATA_HOME"
or fail "_data_root accepted a relative XDG_DATA_HOME in: $out_data"

rm -rf "$sandbox"
test_finish "_warn_shadowing_binaries reports earlier copies on PATH and deletes nothing"
