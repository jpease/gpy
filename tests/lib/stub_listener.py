#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""A stand-in gpy agent on a Unix socket, for the shell test suites.

    stub_listener.py SOCKET MODE [OPTION...]

Modes:
  bind-only  bind SOCKET and exit: a socket file nothing listens on, which is
             what a crashed agent leaves behind
  silent     accept and read every request, never answer
  reply      answer a connection's first request with REPLY and a newline
             after --delay seconds, then close it
  partial    send REPLY with no newline after --delay seconds, then hold the
             connection open: a reply cut off mid-write

Every mode but bind-only returns once SOCKET accepts connections and prints
the PID of the background process serving it. That process exits after
--lifetime seconds at the latest, so a test that never stops it is still
bounded. SIGTERM stops it early: connections still queued are accepted and
their requests logged first, so both logs are complete once it is gone. The
socket file is left in place either way.

Tests start it through shell_e2e_stub_listener (tests/lib/shell_e2e.sh) or
test_stub_listener (tests/lib/test_helpers.fish).
"""

import argparse
import os
import re
import selectors
import signal
import socket
import sys
import time


def parse_args(argv):
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("socket")
    parser.add_argument("mode", choices=("bind-only", "silent", "reply", "partial"))
    parser.add_argument("--reply", type=os.fsencode, help="the answer (reply, partial)")
    parser.add_argument(
        "--route",
        nargs=2,
        action="append",
        default=[],
        metavar=("REGEX", "TEXT"),
        help="answer TEXT instead to a request REGEX matches; the first match wins",
    )
    parser.add_argument(
        "--delay",
        type=float,
        default=0.0,
        help="seconds between a request and its answer",
    )
    parser.add_argument(
        "--lifetime",
        type=float,
        default=30.0,
        help="seconds before exiting (default 30)",
    )
    parser.add_argument("--connections", help="append a line per accepted connection")
    parser.add_argument("--requests", help="append every request line received")
    parser.add_argument("--term-log", help="append TERM when SIGTERM arrives")
    parser.add_argument(
        "--ignore-term",
        action="store_true",
        help="ignore SIGTERM: only SIGKILL stops it",
    )
    args = parser.parse_args(argv)
    if args.mode in ("reply", "partial") and args.reply is None:
        parser.error(f"{args.mode} needs --reply")
    args.route = [
        (re.compile(os.fsencode(regex)), os.fsencode(text))
        for regex, text in args.route
    ]
    return args


def append(path, data):
    if path and data:
        with open(path, "ab") as log:
            log.write(data)


class Connection:
    def __init__(self, sock):
        self.sock = sock
        self.buffer = b""
        self.requested = False
        self.reading = True
        self.answer = b""
        self.due = None


class Stub:
    def __init__(self, server, args):
        self.server = server
        self.args = args
        self.selector = selectors.DefaultSelector()
        self.connections = []
        self.stopping = False

    def on_term(self, _signum, _frame):
        append(self.args.term_log, b"TERM\n")
        self.stopping = True

    def run(self):
        wake_read, wake_write = os.pipe()
        os.set_blocking(wake_read, False)
        os.set_blocking(wake_write, False)
        signal.set_wakeup_fd(wake_write)
        signal.signal(
            signal.SIGTERM, signal.SIG_IGN if self.args.ignore_term else self.on_term
        )

        self.server.setblocking(False)
        self.selector.register(self.server, selectors.EVENT_READ)
        self.selector.register(wake_read, selectors.EVENT_READ)
        deadline = time.monotonic() + self.args.lifetime
        while not self.stopping:
            now = time.monotonic()
            if now >= deadline:
                break
            wake_at = min(
                [deadline] + [c.due for c in self.connections if c.due is not None]
            )
            for key, _ in self.selector.select(max(0.0, wake_at - now)):
                if key.fileobj is self.server:
                    self.accept_pending()
                elif key.fileobj == wake_read:
                    while True:
                        try:
                            if not os.read(wake_read, 512):
                                break
                        except BlockingIOError:
                            break
                else:
                    self.receive(key.data)
            self.send_due(time.monotonic())

        # Whatever the client managed to send before it returned is queued
        # by now: take it in so the logs are complete, an unterminated
        # request included.
        self.accept_pending()
        for conn in list(self.connections):
            if conn.reading:
                self.receive(conn)
            append(self.args.requests, conn.buffer)
        for conn in self.connections:
            conn.sock.close()
        self.server.close()

    def accept_pending(self):
        while True:
            try:
                sock, _ = self.server.accept()
            except (BlockingIOError, InterruptedError):
                return
            sock.setblocking(False)
            append(self.args.connections, b"connection\n")
            conn = Connection(sock)
            self.connections.append(conn)
            self.selector.register(sock, selectors.EVENT_READ, conn)

    def receive(self, conn):
        while True:
            try:
                chunk = conn.sock.recv(65536)
            except (BlockingIOError, InterruptedError):
                return
            except OSError:
                chunk = b""
            if not chunk:
                break
            conn.buffer += chunk
            while b"\n" in conn.buffer:
                line, conn.buffer = conn.buffer.split(b"\n", 1)
                self.request(conn, line + b"\n")
        # End of input: an unterminated request still counts as one.
        if conn.buffer:
            self.request(conn, conn.buffer)
            conn.buffer = b""
        conn.reading = False
        self.selector.unregister(conn.sock)
        if not conn.requested:
            self.close(conn)

    def request(self, conn, data):
        append(self.args.requests, data)
        if conn.requested:
            return
        conn.requested = True
        if self.args.mode == "silent":
            return
        conn.answer = next(
            (text for regex, text in self.args.route if regex.search(data)),
            self.args.reply,
        )
        conn.due = time.monotonic() + self.args.delay

    def send_due(self, now):
        for conn in list(self.connections):
            if conn.due is None or conn.due > now:
                continue
            conn.due = None
            if self.args.mode == "partial":
                self.send(conn, conn.answer)
            else:
                self.send(conn, conn.answer + b"\n")
                self.close(conn)

    @staticmethod
    def send(conn, data):
        try:
            conn.sock.setblocking(True)
            conn.sock.sendall(data)
            conn.sock.setblocking(False)
        except OSError:
            pass

    def close(self, conn):
        if conn.reading:
            conn.reading = False
            self.selector.unregister(conn.sock)
        conn.sock.close()
        self.connections.remove(conn)


def main(argv):
    signal.signal(signal.SIGINT, signal.SIG_DFL)
    args = parse_args(argv)
    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.bind(args.socket)
    if args.mode == "bind-only":
        server.close()
        return 0
    server.listen(16)
    pid = os.fork()
    if pid:
        print(pid, flush=True)
        return 0
    # The caller reads the PID up to end of file: let go of its pipe.
    devnull = os.open(os.devnull, os.O_RDWR)
    os.dup2(devnull, 0)
    os.dup2(devnull, 1)
    os.close(devnull)
    Stub(server, args).run()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
