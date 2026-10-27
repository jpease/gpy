#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Line 1: the login name (USER is part of the clean environment). Line 2: the
# colour variant `root` when GPY_IS_ROOT is 1, so the theme can style it.

[ -n "${USER:-}" ] || exit 0

printf '%s\n' "$USER"
if [ "${GPY_IS_ROOT:-0}" = 1 ]; then
    printf 'root\n'
fi
