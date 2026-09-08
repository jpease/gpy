#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Drive an interactive shell in a pseudo-terminal from a shell test (#645).

Standard library only. A session lives in a directory:

    transcript   everything the program wrote, appended as it arrives
    control      a FIFO; bytes written here go to the program's stdin
    pid          the driver's PID (the program's parent)
    exit         written when the program exits, holding its wait status

Subcommands:

    start DIR -- CMD [ARG...]      fork a driver, spawn CMD in a pty, return
    send DIR TEXT                  write TEXT to the program (\\r \\n \\t \\e
                                   escapes are decoded; ^D style: \\x04)
    wait-for DIR REGEX TIMEOUT [OFFSET]
                                   poll the transcript from byte OFFSET until
                                   REGEX matches; print the transcript length
                                   on success, dump the tail and exit 1 on
                                   timeout
    size DIR                       print the transcript length (an OFFSET for
                                   a later wait-for: "anything new after now")
    stop DIR                       end the program and the driver

`wait-for` only reads the transcript, so a test can watch for output that
arrives with no keystroke sent -- the live-repaint case this exists for.
"""
import os
import re
import select
import signal
import sys
import time

ENV = {"TERM": "xterm", "COLUMNS": "120", "LINES": "40"}

# Fish 4 probes the terminal at startup (kitty keyboard protocol, OSC 11
# background colour, XTGETTCAP) and then sends a Primary Device Attributes
# query as the terminator: it renders no prompt until the DA reply arrives,
# taking the earlier probes as unsupported. A real terminal answers; this
# driver answers like a plain VT100-with-AVO xterm would.
DA_QUERIES = (b"\x1b[0c", b"\x1b[c")
DA_REPLY = b"\x1b[?1;2c"
# Fish also asks where the cursor is (DSR 6, `CSI 6 n`) around prompt
# redraws and waits for the Cursor Position Report before reading keys
# again; answer "bottom row, first column".
CPR_QUERY = b"\x1b[6n"
CPR_REPLY = b"\x1b[%s;1R" % ENV["LINES"].encode()


def paths(directory):
    return {
        name: os.path.join(directory, name)
        for name in ("transcript", "control", "pid", "exit")
    }


def start(directory, command):
    import pty

    os.makedirs(directory, exist_ok=True)
    p = paths(directory)
    for stale in (p["transcript"], p["pid"], p["exit"]):
        if os.path.exists(stale):
            os.unlink(stale)
    if not os.path.exists(p["control"]):
        os.mkfifo(p["control"])

    # Double fork so the caller's shell never waits on the driver.
    if os.fork():
        return 0
    os.setsid()
    if os.fork():
        os._exit(0)

    with open(p["pid"], "w") as f:
        f.write(str(os.getpid()))
    transcript = open(p["transcript"], "ab", buffering=0)
    pid, master = pty.fork()
    if pid == 0:
        os.environ.update(ENV)
        os.execvp(command[0], command)
    # O_RDWR keeps the FIFO open with no writer, so reads never see EOF.
    control = os.open(p["control"], os.O_RDWR | os.O_NONBLOCK)

    def stop_child(*_):
        os.kill(pid, signal.SIGHUP)

    signal.signal(signal.SIGTERM, stop_child)
    status = None
    while status is None:
        try:
            ready, _, _ = select.select([master, control], [], [], 0.5)
        except InterruptedError:
            ready = []
        if master in ready:
            try:
                chunk = os.read(master, 65536)
            except OSError:
                chunk = b""  # EIO: the program closed its side (it exited)
            if chunk:
                transcript.write(chunk)
                if CPR_QUERY in chunk:
                    os.write(master, CPR_REPLY)
                if any(query in chunk for query in DA_QUERIES):
                    os.write(master, DA_REPLY)
            else:
                _, status = os.waitpid(pid, 0)
                break
        if control in ready:
            data = os.read(control, 65536)
            if data:
                os.write(master, data)
        done, code = os.waitpid(pid, os.WNOHANG)
        if done:
            status = code
    # Drain whatever the program wrote before it exited.
    while True:
        try:
            ready, _, _ = select.select([master], [], [], 0.2)
            chunk = os.read(master, 65536) if master in ready else b""
        except OSError:
            chunk = b""
        if not chunk:
            break
        transcript.write(chunk)
    with open(p["exit"], "w") as f:
        f.write(str(status))
    transcript.close()
    os._exit(0)


def decode(text):
    return (
        text.replace("\\r", "\r")
        .replace("\\n", "\n")
        .replace("\\t", "\t")
        .replace("\\e", "\x1b")
        .replace("\\x04", "\x04")
    )


def send(directory, text):
    fd = os.open(paths(directory)["control"], os.O_WRONLY | os.O_NONBLOCK)
    os.write(fd, decode(text).encode())
    os.close(fd)
    return 0


def read_transcript(directory):
    try:
        with open(paths(directory)["transcript"], "rb") as f:
            return f.read()
    except FileNotFoundError:
        return b""


def wait_for(directory, pattern, timeout, offset):
    regex = re.compile(pattern.encode(), re.DOTALL)
    deadline = time.monotonic() + float(timeout)
    while True:
        data = read_transcript(directory)
        if regex.search(data[offset:]):
            print(len(data))
            return 0
        if time.monotonic() >= deadline:
            tail = data[-2000:].decode("utf-8", "replace")
            sys.stderr.write(
                "pty_session: no match for %r after %ss (from offset %d); transcript tail:\n%s\n"
                % (pattern, timeout, offset, tail)
            )
            return 1
        time.sleep(0.05)


def stop(directory):
    p = paths(directory)
    try:
        with open(p["pid"]) as f:
            pid = int(f.read().strip())
    except (FileNotFoundError, ValueError):
        return 0
    for sig in (signal.SIGTERM, signal.SIGKILL):
        try:
            os.kill(pid, sig)
        except ProcessLookupError:
            return 0
        for _ in range(20):
            time.sleep(0.05)
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                return 0
    return 0


def main(argv):
    if len(argv) < 2:
        sys.stderr.write(__doc__)
        return 2
    verb, directory = argv[0], argv[1]
    if verb == "start":
        if len(argv) < 4 or argv[2] != "--":
            sys.stderr.write("usage: start DIR -- CMD [ARG...]\n")
            return 2
        return start(directory, argv[3:])
    if verb == "send":
        return send(directory, argv[2])
    if verb == "wait-for":
        offset = int(argv[4]) if len(argv) > 4 else 0
        return wait_for(directory, argv[2], argv[3], offset)
    if verb == "size":
        print(len(read_transcript(directory)))
        return 0
    if verb == "stop":
        return stop(directory)
    sys.stderr.write("unknown subcommand %r\n" % verb)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
