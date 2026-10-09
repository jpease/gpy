#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Recording IPC stand-in for the cross-shell contract harness (#847).

Listens on a Unix socket the shells under test are pointed at and writes one
line per request (the request's `op`) to a log, so "which IPC requests did this
shell send" is observable without trusting any shell's own logging.

    ipc_proxy.py LISTEN LOG --upstream SOCKET
        forward each request to the real agent and relay its reply (the agent
        is up; the proxy only watches)
    ipc_proxy.py LISTEN LOG --upstream SOCKET --slow SECONDS
        the same for register/unregister/ping/status, but a data request (git,
        directory, character, ...) is recorded, held for SECONDS and then
        forwarded: an agent that is up and registered but late computing a
        segment. A shell whose IPC budget is shorter must neither resend nor
        recompute the segment itself (#845); one whose budget is longer uses
        the real reply.

A register request is forwarded with its cwd replaced by `/` (see unwarmed).
A connection that closes without sending a request is logged as `(empty)`.
The proxy exits after --lifetime seconds (default 120) as a backstop.
"""
import argparse
import json
import os
import socket
import sys
import threading
import time


CONTROL_OPS = {"register", "unregister", "ping", "status"}


def read_line(conn):
    data = b""
    while b"\n" not in data:
        chunk = conn.recv(65536)
        if not chunk:
            break
        data += chunk
    return data


def unwarmed(line):
    """A register request with its cwd pointing nowhere.

    The agent pre-warms the instant cache for a registering shell's directory,
    which would turn the first git render of a row into a cache hit. The shells
    register as they would, and the agent just has nothing to warm.
    """
    try:
        message = json.loads(line)
    except ValueError:
        return line
    if isinstance(message, dict) and message.get("op") == "register":
        message["cwd"] = "/"
        return json.dumps(message).encode() + b"\n"
    return line


def op_of(line):
    if not line.strip():
        return "(empty)"
    try:
        op = json.loads(line).get("op")
    except (ValueError, AttributeError):
        return "(unparsable)"
    return str(op) if op else "(no-op)"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("listen")
    parser.add_argument("log")
    parser.add_argument("--upstream", required=True)
    parser.add_argument("--slow", type=float)
    parser.add_argument("--lifetime", type=float, default=120)
    args = parser.parse_args()

    lock = threading.Lock()

    def record(op):
        with lock, open(args.log, "a") as handle:
            handle.write(op + "\n")

    def serve(conn):
        try:
            request = read_line(conn)
            record(op_of(request))
            if not request.strip():
                return
            if args.slow is not None and op_of(request) not in CONTROL_OPS:
                time.sleep(args.slow)
            request = unwarmed(request)
            upstream = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            try:
                upstream.connect(args.upstream)
                upstream.sendall(request if request.endswith(b"\n") else request + b"\n")
                conn.sendall(read_line(upstream))
            finally:
                upstream.close()
        except OSError:
            pass
        finally:
            conn.close()

    if os.path.exists(args.listen):
        os.unlink(args.listen)
    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.bind(args.listen)
    server.listen(16)
    deadline = time.time() + args.lifetime
    while True:
        left = deadline - time.time()
        if left <= 0:
            return 0
        server.settimeout(left)
        try:
            conn, _ = server.accept()
        except socket.timeout:
            return 0
        threading.Thread(target=serve, args=(conn,), daemon=True).start()


if __name__ == "__main__":
    sys.exit(main())
