#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Line 1: the host name, cut at the first GPY_OPT_trim_at. Line 2: the colour
# variant `ssh` when GPY_IS_SSH is 1.

name=$(hostname) || exit 1
trim=${GPY_OPT_trim_at:-}
if [ -n "$trim" ]; then
    name=${name%%"$trim"*}
fi
[ -n "$name" ] || exit 0

printf '%s\n' "$name"
if [ "${GPY_IS_SSH:-0}" = 1 ]; then
    printf 'ssh\n'
fi
