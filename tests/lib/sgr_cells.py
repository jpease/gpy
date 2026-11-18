#!/usr/bin/env python3
"""Print what a terminal shows for ANSI text: one line per run of cells
with the same colours, as `FG|BG|TEXT`. Two prompts that draw the same
characters in the same colours print the same, whichever SGR spelling
wrote them (Fish's `set_color` writes terminfo sequences, `gpy prompt
fish` plain SGR).

Usage: sgr_cells.py FILE  (`-` reads stdin)

Colours: `d` (terminal default), `cN` (16-colour index 0-15), `xN`
(256-colour index), `#rrggbb`. Attributes other than colour are ignored.
Non-SGR escapes (`ESC ( B`, OSC, other CSI) are dropped.
"""

import re
import sys

CSI = re.compile(r"\x1b\[([0-9;:]*)([A-Za-z])")


def apply(params, fg, bg):
    codes = [int(p) if p else 0 for p in params.replace(":", ";").split(";")] if params else [0]
    i = 0
    while i < len(codes):
        c = codes[i]
        if c == 0:
            fg, bg = "d", "d"
        elif 30 <= c <= 37:
            fg = f"c{c - 30}"
        elif 90 <= c <= 97:
            fg = f"c{c - 82}"
        elif c == 39:
            fg = "d"
        elif 40 <= c <= 47:
            bg = f"c{c - 40}"
        elif 100 <= c <= 107:
            bg = f"c{c - 92}"
        elif c == 49:
            bg = "d"
        elif c in (38, 48) and i + 1 < len(codes):
            if codes[i + 1] == 5 and i + 2 < len(codes):
                n = codes[i + 2]
                value = f"c{n}" if n < 16 else f"x{n}"
                i += 2
            elif codes[i + 1] == 2 and i + 4 < len(codes):
                value = "#%02x%02x%02x" % tuple(codes[i + 2 : i + 5])
                i += 4
            else:
                value = "d"
            if c == 38:
                fg = value
            else:
                bg = value
        i += 1
    return fg, bg


def cells(text):
    fg, bg = "d", "d"
    runs = []
    pos = 0
    while pos < len(text):
        if text[pos] == "\x1b":
            m = CSI.match(text, pos)
            if m:
                if m.group(2) == "m":
                    fg, bg = apply(m.group(1), fg, bg)
                pos = m.end()
                continue
            # ESC ( B and other two-byte escapes.
            pos += 3 if text[pos + 1 : pos + 2] in ("(", ")") else 2
            continue
        ch = text[pos]
        pos += 1
        # A space shows only its background.
        key = (fg if ch != " " else "-", bg)
        if runs and runs[-1][0] == key:
            runs[-1][1].append(ch)
        else:
            runs.append((key, [ch]))
    return runs


def main():
    data = sys.stdin.buffer.read() if sys.argv[1] == "-" else open(sys.argv[1], "rb").read()
    for (fg, bg), chars in cells(data.decode("utf-8", "replace")):
        print(f"{fg}|{bg}|{''.join(chars)!r}")


if __name__ == "__main__":
    main()
