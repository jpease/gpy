#!/bin/sh
# One line (the runner and doctor's trial both show only line 1): the sorted
# NAME=value pairs of the names a script may or may not receive, joined by |.
for n in GPY_SENTINEL HOME LANG LC_ALL LOGNAME TERM TMPDIR TZ USER; do
    eval "v=\${$n-<unset>}"
    printf '%s=%s\n' "$n" "$v"
done | sort | tr '\n' '|'
printf '\n'
