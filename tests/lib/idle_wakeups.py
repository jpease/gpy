#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Count a process's wakeups over a window.

Usage: idle_wakeups.py PID SECONDS

Prints only the integer delta of the process's wakeup counter across a
SECONDS-long sleep. macOS: `ri_pkg_idle_wkups + ri_interrupt_wkups` from
`proc_pid_rusage(RUSAGE_INFO_V2)`. Linux: `voluntary_ctxt_switches +
nonvoluntary_ctxt_switches` summed over every thread. Exits 2 on bad
arguments.
"""
import ctypes
import glob
import os
import sys
import time

RUSAGE_INFO_V2 = 2


class RusageInfoV2(ctypes.Structure):
    """`struct rusage_info_v2` from the macOS SDK's `sys/resource.h`."""

    _fields_ = [("ri_uuid", ctypes.c_uint8 * 16)] + [
        (name, ctypes.c_uint64)
        for name in (
            "ri_user_time",
            "ri_system_time",
            "ri_pkg_idle_wkups",
            "ri_interrupt_wkups",
            "ri_pageins",
            "ri_wired_size",
            "ri_resident_size",
            "ri_phys_footprint",
            "ri_proc_start_abstime",
            "ri_proc_exit_abstime",
            "ri_child_user_time",
            "ri_child_system_time",
            "ri_child_pkg_idle_wkups",
            "ri_child_interrupt_wkups",
            "ri_child_pageins",
            "ri_child_elapsed_abstime",
            "ri_diskio_bytesread",
            "ri_diskio_byteswritten",
        )
    ]


def darwin_wakeups(pid):
    libc = ctypes.CDLL("/usr/lib/libSystem.B.dylib", use_errno=True)
    libc.proc_pid_rusage.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
    libc.proc_pid_rusage.restype = ctypes.c_int
    info = RusageInfoV2()
    if libc.proc_pid_rusage(pid, RUSAGE_INFO_V2, ctypes.byref(info)) != 0:
        err = ctypes.get_errno()
        raise OSError(err, os.strerror(err))
    return info.ri_pkg_idle_wkups + info.ri_interrupt_wkups


def linux_wakeups(pid):
    paths = glob.glob(f"/proc/{pid}/task/*/status")
    if not paths:
        raise OSError(f"no task status files for pid {pid}")
    total = 0
    for path in paths:
        try:
            with open(path, encoding="utf-8") as f:
                for line in f:
                    key, _, value = line.partition(":")
                    if key in ("voluntary_ctxt_switches", "nonvoluntary_ctxt_switches"):
                        total += int(value)
        except FileNotFoundError:
            continue
    return total


def main(argv):
    try:
        pid, seconds = int(argv[0]), float(argv[1])
    except (IndexError, ValueError):
        sys.stderr.write(__doc__)
        return 2
    if len(argv) != 2 or pid <= 0 or seconds < 0:
        sys.stderr.write(__doc__)
        return 2
    read = darwin_wakeups if sys.platform == "darwin" else linux_wakeups
    before = read(pid)
    time.sleep(seconds)
    print(read(pid) - before)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
