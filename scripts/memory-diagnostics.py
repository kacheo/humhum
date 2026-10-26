#!/usr/bin/env python3
"""Opt-in macOS memory/provenance capture; all output shares a 5 MB budget.

Explicit helper PIDs must be attributed to this app by the operator. No process
name guessing: unrelated WebKit processes must never be charged to Humhum.
"""
import argparse
import ctypes
import hashlib
import json
from pathlib import Path
import platform
import selectors
import subprocess
import time

LIMIT = 5_000_000
GROWTH = 256 * 1024 * 1024


class Usage(ctypes.Structure):
    _fields_ = [("uuid", ctypes.c_ubyte * 16)] + [
        (name, ctypes.c_uint64) for name in (
            "user_time", "system_time", "pkg_idle_wkups", "interrupt_wkups",
            "pageins", "wired_size", "resident_size", "phys_footprint",
            "proc_start_abstime", "proc_exit_abstime",
        )
    ]


class Budget:
    def __init__(self, directory):
        self.directory = directory
        self.remaining = LIMIT

    def append(self, name, data):
        data = data[:self.remaining]
        with (self.directory / name).open("ab") as output:
            output.write(data)
        self.remaining -= len(data)

    def record(self, value):
        self.append("processes.jsonl", (json.dumps(value) + "\n").encode())


def identity(binary):
    digest = hashlib.sha256()
    with binary.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    root = Path(__file__).resolve().parents[1]
    revision = subprocess.run(["git", "rev-parse", "HEAD"], cwd=root, capture_output=True, text=True)
    lock = root / "src-tauri" / "Cargo.lock"
    return {"kind": "identity", "binary_sha256": digest.hexdigest(),
            "source_revision": revision.stdout.strip() if revision.returncode == 0 else None,
            "lockfile_sha256": hashlib.sha256(lock.read_bytes()).hexdigest(),
            "macos": platform.mac_ver()[0], "kernel": platform.release(),
            "backend": "See app build diagnostics and native initialization logs; never inferred from app version"}


def snapshot(budget, pid, index):
    # Read incrementally so even a verbose native tool cannot exceed the budget.
    for tool, args in (("vmmap", ["-summary"]), ("heap", ["-s"])):
        name = f"{pid}-{index}-{tool}.txt"
        try:
            with subprocess.Popen([tool, *args, str(pid)], stdout=subprocess.PIPE,
                                  stderr=subprocess.STDOUT) as process:
                deadline = time.monotonic() + 30
                try:
                    with selectors.DefaultSelector() as selector:
                        selector.register(process.stdout, selectors.EVENT_READ)
                        while budget.remaining and time.monotonic() < deadline:
                            if not selector.select(min(1, max(0, deadline - time.monotonic()))):
                                continue
                            data = process.stdout.read1(min(8192, budget.remaining))
                            if not data:
                                break
                            budget.append(name, data)
                finally:
                    if process.poll() is None:
                        process.kill()
                    process.wait()
                budget.record({"kind": "snapshot", "pid": pid, "tool": tool,
                               "exit_code": process.returncode, "file": name})
        except OSError as error:
            budget.record({"kind": "snapshot_error", "pid": pid, "tool": tool,
                           "error": str(error)})
        if not budget.remaining:
            return


def run(args):
    if platform.system() != "Darwin":
        raise SystemExit("macOS is required")
    if args.pid <= 0 or any(pid <= 0 for pid in args.helper_pid):
        raise SystemExit("PIDs must be positive")
    args.output.mkdir(parents=True, exist_ok=False)
    budget = Budget(args.output)
    budget.record(identity(args.binary))
    libc = ctypes.CDLL("/usr/lib/libSystem.B.dylib", use_errno=True)
    libc.proc_pid_rusage.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
    libc.proc_pid_rusage.restype = ctypes.c_int
    pids = dict.fromkeys([args.pid, *args.helper_pid])
    starts, baselines, snapshots = {}, {}, {}
    begin = time.monotonic()
    while time.monotonic() - begin <= args.duration and budget.remaining and pids:
        elapsed = time.monotonic() - begin
        for pid in list(pids):
            usage = Usage()
            if libc.proc_pid_rusage(pid, 0, ctypes.byref(usage)) != 0:
                budget.record({"kind": "process_gone", "pid": pid, "errno": ctypes.get_errno()})
                del pids[pid]
                continue
            if pid in starts and starts[pid] != usage.proc_start_abstime:
                budget.record({"kind": "pid_reused", "pid": pid})
                del pids[pid]
                continue
            starts[pid] = usage.proc_start_abstime
            budget.record({"kind": "memory", "unix_secs": time.time(), "elapsed_secs": elapsed,
                           "pid": pid, "role": "main" if pid == args.pid else "attributed_helper",
                           "rss_bytes": usage.resident_size, "footprint_bytes": usage.phys_footprint})
            if elapsed >= args.warmup:
                if pid not in baselines:
                    baselines[pid] = usage.phys_footprint
                    snapshots[pid] = 0
                    snapshot(budget, pid, 0)
                elif usage.phys_footprint - baselines[pid] >= GROWTH and snapshots[pid] < 3:
                    snapshots[pid] += 1
                    snapshot(budget, pid, snapshots[pid])
                    baselines[pid] = usage.phys_footprint
        time.sleep(min(30, max(0, args.duration - (time.monotonic() - begin))))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--helper-pid", type=int, action="append", default=[])
    parser.add_argument("--output", type=Path, required=True, help="new directory")
    parser.add_argument("--warmup", type=float, default=300)
    parser.add_argument("--duration", type=float, default=8 * 60 * 60)
    args = parser.parse_args()
    if args.duration <= 0 or args.warmup < 0:
        parser.error("duration must be positive and warmup nonnegative")
    run(args)
