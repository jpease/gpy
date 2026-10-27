#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# GPY's `hostname` segment, as Starship's `hostname`: shown over SSH, or
# always with the setting `show_always`. Line 1: the host name, cut at the
# first `trim_at` (empty keeps it whole). Line 2: the variant `ssh` over SSH,
# so the theme can show `ssh_icon` there only.

if [ "${GPY_OPT_show_always:-false}" != true ] && [ "${GPY_IS_SSH:-0}" != 1 ]; then
    exit 0
fi
name=$(uname -n 2>/dev/null) || exit 0
trim=${GPY_OPT_trim_at-.}
if [ -n "$trim" ]; then
    name=${name%%"$trim"*}
fi
[ -n "$name" ] || exit 0

printf '%s\n' "$name"
if [ "${GPY_IS_SSH:-0}" = 1 ]; then
    printf 'ssh\n'
fi
