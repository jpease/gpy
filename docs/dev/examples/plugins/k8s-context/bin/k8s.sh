#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Reads `current-context` from the first kubeconfig file instead of running
# kubectl, which would take far longer than the 100 ms this segment may use.
# Line 1: the context. Line 2: the colour variant `prod` for production-like names.

config=${KUBECONFIG:-$HOME/.kube/config}
config=${config%%:*}
[ -r "$config" ] || exit 0

context=$(sed -n "/^current-context:/{s/^current-context: *[\"']*\\([^\"']*\\).*/\\1/p;q;}" "$config")
[ -n "$context" ] || exit 0

printf '%s\n' "$context"
case $context in
*prod*) printf 'prod\n' ;;
esac
