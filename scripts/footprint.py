#!/usr/bin/env python3
"""Relaunch Mojito.app, drive the switcher with posted keys, and sample its
physical footprint (Activity Monitor's "Memory") every 50 ms.

Prints, per phase, the peak and the settled value at the phase's end, and
writes the full timeline as CSV when --csv is given.

Usage: scripts/footprint.py [--cycles N] [--settle SECONDS] [--csv PATH] [--env KEY=VALUE]
"""

import argparse
import ctypes
import os
import subprocess
import threading
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
APP = os.path.join(ROOT, "target/app/Mojito.app")
BIN = os.path.join(APP, "Contents/MacOS/mojito")
F18, E, ESC = "4f", "0e", "35"
RUSAGE_INFO_V0 = 0
MB = 1024 * 1024


class RusageInfoV0(ctypes.Structure):
    _fields_ = [
        ("ri_uuid", ctypes.c_uint8 * 16),
        ("ri_user_time", ctypes.c_uint64),
        ("ri_system_time", ctypes.c_uint64),
        ("ri_pkg_idle_wkups", ctypes.c_uint64),
        ("ri_interrupt_wkups", ctypes.c_uint64),
        ("ri_pageins", ctypes.c_uint64),
        ("ri_wired_size", ctypes.c_uint64),
        ("ri_resident_size", ctypes.c_uint64),
        ("ri_phys_footprint", ctypes.c_uint64),
        ("ri_proc_start_abstime", ctypes.c_uint64),
        ("ri_proc_exit_abstime", ctypes.c_uint64),
    ]


libproc = ctypes.CDLL("/usr/lib/libproc.dylib")


def footprint(pid):
    info = RusageInfoV0()
    if libproc.proc_pid_rusage(pid, RUSAGE_INFO_V0, ctypes.byref(info)) != 0:
        return None
    return info.ri_phys_footprint


class Sampler(threading.Thread):
    def __init__(self, pid):
        super().__init__(daemon=True)
        self.pid = pid
        self.samples = []
        self.phase = "launch"
        self.stop = threading.Event()

    def run(self):
        start = time.monotonic()
        while not self.stop.is_set():
            value = footprint(self.pid)
            if value is not None:
                self.samples.append((time.monotonic() - start, self.phase, value))
            time.sleep(0.05)


def key(code, direction):
    subprocess.run([BIN, "key", code, direction], check=True)


def tap(code):
    key(code, "down")
    key(code, "up")


def open_switcher(live):
    key(F18, "down")
    tap(E)
    if live:
        tap(E)
    key(F18, "up")


def relaunch(env):
    subprocess.run(["pkill", "-x", "mojito"], check=False)
    time.sleep(1)
    subprocess.run(["open", *[f"--env={pair}" for pair in env], APP], check=True)
    for _ in range(100):
        out = subprocess.run(["pgrep", "-x", "mojito"], capture_output=True, text=True).stdout.split()
        if out:
            return int(out[0])
        time.sleep(0.1)
    raise SystemExit("mojito did not start")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cycles", type=int, default=3)
    parser.add_argument("--settle", type=float, default=8.0)
    parser.add_argument("--csv")
    parser.add_argument("--env", action="append", default=[], metavar="KEY=VALUE")
    args = parser.parse_args()

    pid = relaunch(args.env)
    sampler = Sampler(pid)
    sampler.start()
    time.sleep(args.settle)
    phases = ["launch"]
    for cycle in range(1, args.cycles + 1):
        sampler.phase = f"shown-{cycle}"
        phases.append(sampler.phase)
        open_switcher(live=cycle % 2 == 0)
        time.sleep(1.5)
        sampler.phase = f"hidden-{cycle}"
        phases.append(sampler.phase)
        tap(ESC)
        time.sleep(args.settle)
    sampler.stop.set()
    sampler.join()

    print(f"{'phase':<10} {'peak MB':>8} {'end MB':>8}")
    for phase in phases:
        values = [v for _, p, v in sampler.samples if p == phase]
        if values:
            print(f"{phase:<10} {max(values) / MB:8.1f} {values[-1] / MB:8.1f}")
    if args.csv:
        with open(args.csv, "w") as out:
            out.write("t,phase,footprint_mb\n")
            for t, phase, value in sampler.samples:
                out.write(f"{t:.2f},{phase},{value / MB:.2f}\n")


if __name__ == "__main__":
    main()
