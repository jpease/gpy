#!/usr/bin/env fish
# Verifies that switching themes in an open shell does not leak the previous
# theme's optional variables (#791): __time_format and the status icons.

set -l temp_root (mktemp -d)
set -l config_dir "$temp_root/gpy"
set -l theme_dir "$config_dir/themes"
mkdir -p $theme_dir
set -l config_path "$config_dir/config.toml"

set -l default_theme config/themes/default.toml
sed 's/^time_format = "12"/time_format = "24"/' $default_theme >"$theme_dir/a24.toml"
sed -E 's/^ok_icon = .*/ok_icon = "OK"/' "$theme_dir/a24.toml" >"$theme_dir/a24.tmp"
mv "$theme_dir/a24.tmp" "$theme_dir/a24.toml"
grep -v -E '^(time_format|ok_icon|fail_icon) ' $default_theme >"$theme_dir/bplain.toml"

function _write_config --argument-names theme path
    printf '[ui]\nshow_icons = false\ntheme = "%s"\n' $theme >$path
end

_write_config a24 $config_path

set -lx XDG_CONFIG_HOME $temp_root
set -lx XDG_CACHE_HOME "$temp_root/cache"
set -lx PATH $PWD/gpy-agent/target/debug $PATH
set -lx MISE_DISABLE 1

source fish/core/init.fish

__gpy_reload_theme 0
if test "$__time_format" != 24; or test "$__icon_status_ok" != OK
    echo "❌ precondition: a24 theme not applied (time_format='$__time_format' ok='$__icon_status_ok')"
    rm -rf $temp_root
    exit 1
end

_write_config bplain $config_path
__gpy_reload_theme 0

set -l failed 0
set -l fmt (__gpy_clock_date_format)
if test "$fmt" != "%-I:%M %p"
    echo "❌ expected 12-hour clock format '%-I:%M %p' after switch, got '$fmt'"
    set failed 1
end

if test "$__prompt_icons" = nerd
    set expected_ok $__gpy_icon_nerd_status_ok
    set expected_fail $__gpy_icon_nerd_status_fail
else
    set expected_ok $__gpy_icon_ascii_status_ok
    set expected_fail $__gpy_icon_ascii_status_fail
end
if test "$__icon_status_ok" != "$expected_ok"
    echo "❌ __icon_status_ok='$__icon_status_ok', expected fallback '$expected_ok'"
    set failed 1
end
if test "$__icon_status_fail" != "$expected_fail"
    echo "❌ __icon_status_fail='$__icon_status_fail', expected fallback '$expected_fail'"
    set failed 1
end

rm -rf $temp_root
if test $failed -ne 0
    exit 1
end
echo "✅ Theme reload clears optional theme variables"
