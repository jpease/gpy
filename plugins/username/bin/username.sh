#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# GPY's `username` segment, as Starship's `username`: shown as root, under
# sudo, over SSH, or always with the setting `show_always`. Line 1: the login
# name: $USER, else $LOGNAME (both part of the clean environment when set),
# else `id -un`, so a root shell in a container without $USER still shows.
# Line 2: the variant `root` as root, which the theme styles with
# `root_bg_color` / `root_text_color`.

if [ "${GPY_OPT_show_always:-false}" != true ] && [ "${GPY_IS_ROOT:-0}" != 1 ] &&
    [ "${GPY_IS_SUDO:-0}" != 1 ] && [ "${GPY_IS_SSH:-0}" != 1 ]; then
    exit 0
fi
name=${USER:-${LOGNAME:-}}
if [ -z "$name" ]; then
    name=$(id -un 2>/dev/null) || name=
fi
[ -n "$name" ] || exit 0

printf '%s\n' "$name"
if [ "${GPY_IS_ROOT:-0}" = 1 ]; then
    printf 'root\n'
fi
