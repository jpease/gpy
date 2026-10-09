#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Normalize an expanded shell prompt so Fish, Bash and Zsh output can be diffed.

Reads the text a terminal would receive for a prompt (stdin) and prints one
line per visible run: the effective style, then the text, e.g.

    [fg=black bg=green]' OK '
    []'$ '

Fish, Bash and Zsh reach the same picture through different bytes: Fish's
`set_color` emits `ESC[30m` then `ESC[42m` and `ESC(B ESC[m` for a reset, the
agent and the Bash/Zsh local renderer emit `ESC[42;30m` and `ESC[0m`, Bash's
readline wraps non-printing runs in \\001/\\002, and a style may be
re-asserted any number of times. None of that is visible, so it is folded
away: only the foreground, background and attributes in force for each
visible character survive, and adjacent characters in the same style merge
into one run.

Options:
    --mask-seconds   replace the seconds of an `H:MM:SS` time with `SS`
                     (a clock that shows seconds cannot match across shells
                     that each sample the time at a different instant)
"""
import re
import sys

BASIC = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"]
ATTRS = {
    "1": "bold",
    "2": "dim",
    "3": "italic",
    "4": "underline",
    "5": "blink",
    "7": "reverse",
    "8": "hidden",
    "9": "strike",
}
# SGR codes that switch attributes off, and the attributes each one clears.
ATTRS_OFF = {
    22: {"bold", "dim"},
    23: {"italic"},
    24: {"underline"},
    25: {"blink"},
    27: {"reverse"},
    28: {"hidden"},
    29: {"strike"},
}
# Fish resets with `ESC ( B ESC [ m`; the `ESC ( B` charset select is not an
# SGR sequence and carries no style.
TOKEN = re.compile(r"\x1b\(B|\x1b\[([0-9;:]*)m|\x1b\[[0-9;?]*[A-Za-z]|[\x01\x02]")


def color_at(params, i):
    """Consume an extended-colour spec at params[i] (38 or 48); return (name, next_i)."""
    if i + 1 < len(params) and params[i + 1] == "5" and i + 2 < len(params):
        return f"idx{params[i + 2]}", i + 3
    if i + 1 < len(params) and params[i + 1] == "2" and i + 4 < len(params):
        r, g, b = params[i + 2 : i + 5]
        return f"#{int(r):02x}{int(g):02x}{int(b):02x}", i + 5
    return None, len(params)


def apply_sgr(state, raw):
    params = [p for p in re.split(r"[;:]", raw)] if raw else ["0"]
    i = 0
    while i < len(params):
        p = params[i] or "0"
        n = int(p)
        if n == 0:
            state.update(fg=None, bg=None, attrs=set())
        elif p in ATTRS:
            state["attrs"].add(ATTRS[p])
        elif n in ATTRS_OFF:
            state["attrs"] -= ATTRS_OFF[n]
        elif 30 <= n <= 37:
            state["fg"] = BASIC[n - 30]
        elif n == 38:
            state["fg"], i = color_at(params, i)
            continue
        elif n == 39:
            state["fg"] = None
        elif 40 <= n <= 47:
            state["bg"] = BASIC[n - 40]
        elif n == 48:
            state["bg"], i = color_at(params, i)
            continue
        elif n == 49:
            state["bg"] = None
        elif 90 <= n <= 97:
            state["fg"] = "bright-" + BASIC[n - 90]
        elif 100 <= n <= 107:
            state["bg"] = "bright-" + BASIC[n - 100]
        i += 1


def normalize(text, mask_seconds=False):
    state = {"fg": None, "bg": None, "attrs": set()}
    runs = []  # (style-tuple, text)
    pos = 0

    def emit(chunk):
        if not chunk:
            return
        style = (state["fg"], state["bg"], tuple(sorted(state["attrs"])))
        if runs and runs[-1][0] == style:
            runs[-1] = (style, runs[-1][1] + chunk)
        else:
            runs.append((style, chunk))

    for m in TOKEN.finditer(text):
        emit(text[pos : m.start()])
        pos = m.end()
        if m.group(0).startswith("\x1b[") and m.group(0).endswith("m"):
            apply_sgr(state, m.group(1))
    emit(text[pos:])

    lines = []
    for (fg, bg, attrs), chunk in runs:
        if mask_seconds:
            chunk = re.sub(r"(\d{1,2}:\d\d):\d\d", r"\1:SS", chunk)
        bits = list(attrs)
        if fg:
            bits.append(f"fg={fg}")
        if bg:
            bits.append(f"bg={bg}")
        lines.append("[" + " ".join(bits) + "]" + repr(chunk))
    return "\n".join(lines)


def main(argv):
    mask_seconds = "--mask-seconds" in argv
    data = sys.stdin.buffer.read().decode("utf-8", "replace")
    out = normalize(data, mask_seconds)
    sys.stdout.write(out + ("\n" if out else ""))


if __name__ == "__main__":
    main(sys.argv[1:])
