#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Summarise one shell's observable side effects for the contract harness (#847).

    effects.py CALLS IPC

CALLS  one line per `gpy-agent` invocation the shell made (the argv), written
       by the recording wrapper on PATH
IPC    one line per IPC request (the op), written by ipc_proxy.py

Prints `key=value` lines, identical in shape for every shell so the three
outputs can be diffed and individual keys asserted:

    starts=N          `gpy-agent start` / `restart` invocations
    oneshots=a,b      the kinds of every `gpy-agent oneshot <kind>` (sorted), or -
    other_calls=a,b   any other `gpy-agent <word>` invocation (sorted), or -
    registers=N       `register` IPC requests
    ipc_total=N       every IPC request except the probes below
    ipc.<op>=N        IPC requests per op

Not counted: `ping`, `status` and `unregister` requests and `gpy-agent status`
calls (readiness probes and shell-exit teardown, which the shells do their own way).
"""
import collections
import sys


# Liveness probes and teardown, not behaviour: shells legitimately differ in how
# they check that the agent is ready (`ping`, `gpy-agent status`) and in whether
# an exiting shell says goodbye (`unregister`), and a shell's exit is not part of
# a prompt render. Everything else is part of the contract.
PROBE_OPS = {"ping", "status", "unregister"}


def lines(path):
    try:
        with open(path) as handle:
            return [line.rstrip("\n") for line in handle if line.strip()]
    except FileNotFoundError:
        return []


def main():
    calls = [line.split() for line in lines(sys.argv[1])]
    ipc = [op for op in lines(sys.argv[2]) if op not in PROBE_OPS]
    starts = sum(1 for argv in calls if argv[0] in ("start", "restart"))
    oneshots = sorted(argv[1] if len(argv) > 1 else "?" for argv in calls if argv[0] == "oneshot")
    other = sorted(
        argv[0] for argv in calls if argv[0] not in ("start", "restart", "oneshot", "status")
    )
    counts = collections.Counter(ipc)
    print(f"starts={starts}")
    print(f"oneshots={','.join(oneshots) or '-'}")
    print(f"other_calls={','.join(other) or '-'}")
    print(f"registers={counts.get('register', 0)}")
    print(f"ipc_total={len(ipc)}")
    for op in sorted(counts):
        print(f"ipc.{op}={counts[op]}")


if __name__ == "__main__":
    main()
