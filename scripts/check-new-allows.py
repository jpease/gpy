#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Fail when a change adds an `allow` attribute without a `reason`.

Usage:
    check-new-allows.py --staged          # pre-commit: staged changes
    check-new-allows.py <base-rev>        # CI / pre-push: <base-rev>...HEAD

Only lines the change adds are checked, so existing allows are left alone.
An attribute is every `#[...]` / `#![...]` that starts on an added line, read
up to its closing bracket (multi-line attributes are joined); one that starts
inside a `//` comment (including doc comments) or a string literal on that
line is not an attribute. It is reasonless if it contains `allow(` (directly or
inside `cfg_attr(...)`) and no `reason =`. A reasonless allow the same change
also removes (same text, whitespace-insensitive, anywhere in the diff) is a
move, as when a file is split, and passes: each removed copy excuses one added
copy. Exit 0: clean. Exit 1: offending attributes printed as path:line.
Exit 2: usage error (including an empty base revision), or git could not produce the diff (for example a base
revision missing from a shallow clone). Standard library only (python3 >= 3.8).
"""
import collections
import re
import subprocess
import sys

ATTR_START = re.compile(r'#!?\[')
HUNK = re.compile(r'^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@')


def diff_text(args):
    cmd = ['git', 'diff', '--no-color', '--no-ext-diff', '--no-prefix', '-U0']
    if args == ['--staged']:
        cmd.append('--cached')
    elif len(args) == 1 and args[0] and not args[0].startswith('-'):
        cmd.append(f'{args[0]}...HEAD')
    else:
        print(__doc__.strip(), file=sys.stderr)
        sys.exit(2)
    cmd += ['--', ':(top)*.rs']
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        print(f'git diff failed: {result.stderr.strip()}', file=sys.stderr)
        sys.exit(2)
    return result.stdout


def runs(text, sign):
    """Yield (path, first_line_no, [lines]) for each run of consecutive lines starting with sign."""
    header = '+++ ' if sign == '+' else '--- '
    path, line_no, run, start = None, 0, [], 0
    for raw in text.splitlines():
        if raw.startswith('+++ ') or raw.startswith('--- '):
            if raw.startswith(header):
                path = None if raw[4:] == '/dev/null' else raw[4:]
            continue
        m = HUNK.match(raw)
        if m:
            if run:
                yield path, start, run
            run, line_no = [], int(m.group(2 if sign == '+' else 1))
            continue
        if raw.startswith(sign) and path:
            if not run:
                start = line_no
            run.append(raw[1:])
            line_no += 1
        elif run:
            yield path, start, run
            run = []
    if run:
        yield path, start, run


def in_comment_or_string(line_prefix):
    """True if text before an attribute start is inside a `//` comment or a string literal."""
    in_string, i = False, 0
    while i < len(line_prefix):
        c = line_prefix[i]
        if in_string:
            if c == '\\':
                i += 1
            elif c == '"':
                in_string = False
        elif c == '"':
            in_string = True
        elif line_prefix.startswith('//', i):
            return True
        i += 1
    return in_string


def attributes(lines):
    """Yield (offset, text) for each attribute starting in lines, joined to its close."""
    joined = '\n'.join(lines)
    for m in ATTR_START.finditer(joined):
        line_start = joined.rfind('\n', 0, m.start()) + 1
        if in_comment_or_string(joined[line_start:m.start()]):
            continue
        depth, i = 0, m.end() - 1
        while i < len(joined):
            c = joined[i]
            if c == '[':
                depth += 1
            elif c == ']':
                depth -= 1
                if depth == 0:
                    break
            i += 1
        yield joined.count('\n', 0, m.start()), joined[m.start():i + 1]


def reasonless(attr):
    """Normalized attribute text if it is a reasonless allow, else None."""
    code = re.sub(r'//[^\n]*', '', attr)
    if re.search(r'\ballow\s*\(', code) and not re.search(r'\breason\s*=', code):
        return ''.join(code.split())
    return None


def main():
    text = diff_text(sys.argv[1:])
    removed = collections.Counter()
    for _path, _start, lines in runs(text, '-'):
        for _offset, attr in attributes(lines):
            key = reasonless(attr)
            if key:
                removed[key] += 1
    bad = []
    for path, start, lines in runs(text, '+'):
        for offset, attr in attributes(lines):
            key = reasonless(attr)
            if not key:
                continue
            if removed[key] > 0:
                removed[key] -= 1
                continue
            shown = ' '.join(re.sub(r'//[^\n]*', '', attr).split())
            bad.append(f'{path}:{start + offset}: {shown}')
    if bad:
        print('allow attributes added without a reason (add reason = "..."):')
        print('\n'.join(bad))
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
