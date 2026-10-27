#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# GPY's `username` segment, as Starship's `username`: shown as root, under
# sudo, over SSH, or always with the setting `show_always`. Line 1: the login
# name ($USER is part of the clean environment). Line 2: the variant `root` as
# root, which the theme styles with `root_bg_color` / `root_text_color`.

if [ "${GPY_OPT_show_always:-false}" != true ] && [ "${GPY_IS_ROOT:-0}" != 1 ] &&
    [ "${GPY_IS_SUDO:-0}" != 1 ] && [ "${GPY_IS_SSH:-0}" != 1 ]; then
    exit 0
fi
[ -n "${USER:-}" ] || exit 0

printf '%s\n' "$USER"
if [ "${GPY_IS_ROOT:-0}" = 1 ]; then
    printf 'root\n'
fi
