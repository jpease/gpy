#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Summarise a repaint-latency log written by the shell e2e tests.

Usage: latency_median.py LOG

Each LOG line is `<test> <step> <session> <ms>`. Prints one line per
(test, step, session), sorted: `<test> <step> <session> <median> n=<count>`.
Exits 1 when LOG is missing or empty, 2 on a malformed line.
"""
import statistics
import sys


def main(argv):
    if len(argv) != 1:
        sys.stderr.write(__doc__)
        return 2
    try:
        with open(argv[0], encoding="utf-8") as f:
            lines = [line.strip() for line in f if line.strip()]
    except FileNotFoundError:
        sys.stderr.write("latency_median: no log at %s\n" % argv[0])
        return 1
    if not lines:
        sys.stderr.write("latency_median: %s is empty\n" % argv[0])
        return 1
    samples = {}
    for line in lines:
        fields = line.split()
        try:
            if len(fields) != 4:
                raise ValueError
            value = float(fields[3])
        except ValueError:
            sys.stderr.write("latency_median: malformed line: %r\n" % line)
            return 2
        samples.setdefault(tuple(fields[:3]), []).append(value)
    for key in sorted(samples):
        values = samples[key]
        print("%s %s %s %g n=%d" % (*key, statistics.median(values), len(values)))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
