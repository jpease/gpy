#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Fail when a Rust source file is longer than the cap, or a listed file grew.

Size is a file's line count minus its top-level inline test modules: a
`mod name {` line at column 0 whose attribute block above it holds
`#[cfg(test)]` or a `#[cfg(all(...))]` with `test` as one of its direct
arguments (in any position; `test` nested in `any`/`not` does not count),
through the first `}` at column 0 after it (rustfmt
layout; a `}` at column 0 inside a raw string in a test module ends the skip
early, so such a file counts slightly short). `cfg(any(test, ...))` modules are
counted, since they also compile outside tests. File-backed test modules
(`tests.rs`, `*_tests.rs`), everything under `tests/`, `benches/` and
`examples/` directories, and tracked files missing from the worktree are not
checked.

Files already over the cap are listed in the allowlist, one `<size> <path>` per
line (full-line `#` comments allowed). A listed file may not grow past its recorded size;
when it shrinks, or drops under the cap, the check fails until the list is
updated, so the recorded size only ever goes down. `--update` rewrites the list
that way (it lowers sizes and drops files under the cap; it never raises a size
or adds a file). `--init` writes the first list, every file now over the cap at
its current size, and refuses if the list already exists.

Usage, from anywhere in the repository (paths are taken from its root; an
explicit --allowlist is taken from the current directory):
    check_file_size.py [--cap N] [--allowlist PATH] [--exclude GLOB ...] [--update | --init]
Exit 0 clean, 1 on a violation, 2 on a usage error. Standard library only.
"""

import argparse
import fnmatch
import os
import re
import subprocess
import sys

DEFAULT_CAP = 800
DEFAULT_ALLOWLIST = "scripts/file-size-allowlist.txt"
CFG = re.compile(r"^#\[cfg\((.*)\)\]\s*$")
MOD_OPEN = re.compile(r"^(pub(\([^)]*\))?\s+)?mod\s+\w+\s*\{")
SKIP_DIRS = {"tests", "benches", "examples"}


def top_level_args(text):
    """Split `a,b(c,d),e` on commas that are not inside parentheses."""
    args, depth, current = [], 0, ""
    for char in text:
        if char == "," and depth == 0:
            args.append(current)
            current = ""
            continue
        depth += (char == "(") - (char == ")")
        current += char
    return args + [current]


def is_test_cfg(line):
    """True for `#[cfg(test)]` and `#[cfg(all(..., test, ...))]` with `test` a direct argument."""
    match = CFG.match(line)
    if not match:
        return False
    inner = match.group(1).replace(" ", "")
    if inner == "test":
        return True
    if not (inner.startswith("all(") and inner.endswith(")")):
        return False
    return "test" in top_level_args(inner[len("all("):-1])


def code_size(text):
    """Line count without top-level inline `cfg(test)` modules."""
    lines = text.split("\n")
    if lines and lines[-1] == "":
        lines.pop()
    skipped = 0
    i = 0
    while i < len(lines):
        if MOD_OPEN.match(lines[i]):
            # Walk the attribute block directly above the `mod` line.
            j = i - 1
            is_test = False
            while j >= 0 and lines[j].startswith("#["):
                if is_test_cfg(lines[j]):
                    is_test = True
                j -= 1
            if is_test:
                end = i + 1
                while end < len(lines) and lines[end] != "}":
                    end += 1
                skipped += end - j  # attributes, the module, its closing brace
                i = end + 1
                continue
        i += 1
    return len(lines) - skipped


def checked_files(excludes):
    out = subprocess.run(
        ["git", "ls-files", "-z", "--", "*.rs"],
        check=True, capture_output=True, text=True,
    ).stdout
    for path in out.split("\0"):
        if not path:
            continue
        parts = path.split("/")
        name = parts[-1]
        if SKIP_DIRS.intersection(parts[:-1]):
            continue
        if name == "tests.rs" or name.endswith("_tests.rs"):
            continue
        if any(fnmatch.fnmatch(path, glob) for glob in excludes):
            continue
        yield path


def read_allowlist(path):
    listed = {}
    try:
        with open(path, encoding="utf-8") as handle:
            for number, raw in enumerate(handle, 1):
                line = raw.strip()
                if not line or line.startswith("#"):
                    continue
                size, _, file_path = line.partition(" ")
                if not size.isdigit() or not file_path.strip():
                    sys.exit(f"{path}:{number}: expected '<size> <path>'")
                listed[file_path.strip()] = int(size)
    except FileNotFoundError:
        pass
    return listed


def write_allowlist(path, entries, cap):
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(
            f"# Rust files over the {cap}-line cap (check_file_size.py). A listed file may\n"
            "# not grow; lower its size when it shrinks. Never add a file or raise a size\n"
            "# without saying why in the commit.\n"
        )
        for file_path in sorted(entries):
            handle.write(f"{entries[file_path]} {file_path}\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--cap", type=int, default=DEFAULT_CAP)
    parser.add_argument("--allowlist", default=None)
    parser.add_argument("--exclude", action="append", default=[], metavar="GLOB")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--update", action="store_true",
                      help="lower recorded sizes and drop files now under the cap")
    mode.add_argument("--init", action="store_true",
                      help="write the first allowlist from the current tree")
    args = parser.parse_args()

    root = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"], check=True, capture_output=True, text=True,
    ).stdout.strip()
    args.allowlist = (
        os.path.join(root, DEFAULT_ALLOWLIST) if args.allowlist is None
        else os.path.abspath(args.allowlist)
    )
    os.chdir(root)

    if args.init:
        os.makedirs(os.path.dirname(args.allowlist), exist_ok=True)
        try:
            with open(args.allowlist, "x", encoding="utf-8"):
                pass
        except FileExistsError:
            print(f"{args.allowlist} already exists; use --update", file=sys.stderr)
            return 2

    listed = read_allowlist(args.allowlist)
    sizes = {}
    for path in checked_files(args.exclude):
        try:
            with open(path, encoding="utf-8", errors="replace") as handle:
                sizes[path] = code_size(handle.read())
        except FileNotFoundError:
            continue

    if args.init:
        over = {path: size for path, size in sizes.items() if size > args.cap}
        write_allowlist(args.allowlist, over, args.cap)
        print(f"listed {len(over)} files over the {args.cap}-line cap in {args.allowlist}")
        return 0

    problems = []
    updated = {}
    for path, size in sorted(sizes.items()):
        recorded = listed.get(path)
        if recorded is None:
            if size > args.cap:
                problems.append(f"{path}: {size} lines, over the {args.cap}-line cap; split it")
            continue
        if size > recorded:
            problems.append(f"{path}: grew to {size} lines (listed at {recorded}); split it")
            updated[path] = recorded
        elif size <= args.cap:
            problems.append(f"{path}: now {size} lines, under the cap; remove it from {args.allowlist}")
        elif size < recorded:
            problems.append(f"{path}: shrank to {size} lines; lower its entry in {args.allowlist}")
            updated[path] = size
        else:
            updated[path] = recorded
    for path in sorted(set(listed) - set(sizes)):
        problems.append(f"{path}: listed in {args.allowlist} but no longer checked; remove it")

    if args.update:
        write_allowlist(args.allowlist, updated, args.cap)
        problems = [p for p in problems if "grew to" in p or "over the" in p]

    for problem in problems:
        print(problem)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
