#!/usr/bin/env python3
"""PROTOTYPE bench for the Android headless-Dart push wake. Throwaway.

Usage: android_push_proto_bench.py RUNS [--serial S] [--relay URL] [--drop-caches]
                                         [--mem] [--label L]
Each run: kill the app process (am kill, or root kill), verify pidof is empty,
broadcast the wake, wait for the handler to finish, parse PUSHPROTO logcat.
"""
import argparse, json, re, statistics, subprocess, sys, time

PKG = "xyz.block.buzz.mobile.androidpushproto"
RECV = f"{PKG}/xyz.block.buzz.mobile.PushProtoReceiver"
STAGES = ["wake", "placeholder_posted", "engine_created", "dart_started",
          "dart_entered", "binding_ready", "storage_loaded", "connect_start",
          "ws_connected", "auth_done", "req_sent", "event_received",
          "profile_received", "notification_updated", "handler_finished"]


def adb(serial, *args, check=False, timeout=60):
    cmd = ["adb"] + (["-s", serial] if serial else []) + list(args)
    return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout).stdout


def kill_app(serial, root):
    for _ in range(10):
        if not adb(serial, "shell", "pidof", PKG).strip():
            return True
        adb(serial, "shell", "am", "kill", PKG)
        if root:
            for pid in adb(serial, "shell", "pidof", PKG).split():
                adb(serial, "shell", "kill", "-9", pid)
        time.sleep(0.5)
    return not adb(serial, "shell", "pidof", PKG).strip()


def one_run(a, root):
    assert kill_app(a.serial, root), "could not kill app process"
    assert not adb(a.serial, "shell", "pidof", PKG).strip(), "process exists before wake"
    adb(a.serial, "shell", "cmd", "notification", "cancel", PKG, "4242")  # best effort
    if a.drop_caches and root:
        adb(a.serial, "shell", "sync")
        adb(a.serial, "shell", "echo 3 > /proc/sys/vm/drop_caches")
    adb(a.serial, "logcat", "-c")
    mem = []
    extra = ["--es", "relay", a.relay] if a.relay else []
    if a.mem:
        extra += ["--el", "hold", "4000"]
    t0 = time.time()
    adb(a.serial, "shell", "am", "broadcast", "-n", RECV, "-a",
        "xyz.block.buzz.mobile.PROTO_WAKE", *extra)
    deadline = time.time() + 15
    log = ""
    while time.time() < deadline:
        if a.mem:
            out = adb(a.serial, "shell", "dumpsys", "meminfo", "-s", PKG)
            m = re.search(r"TOTAL PSS:\s+(\d+)", out)
            if m:
                mem.append(int(m.group(1)))
        else:
            time.sleep(0.3)
        log = adb(a.serial, "logcat", "-d", "-s", "PUSHPROTO")
        if "stage handler_finished" in log:
            break
    st = {m.group(1): int(m.group(2)) for m in re.finditer(r"stage (\w+) t=(\d+)", log)}
    r = {"mem_samples_kb": mem}
    proc = re.search(r"procStart=(\d+)", log)
    fin = re.search(r"finish (\w+) pssKb=(\d+) nativeHeapKb=(\d+)", log)
    if fin:
        r["result"], r["pss_at_finish_kb"] = fin.group(1), int(fin.group(2))
    if "wake" in st:
        w = st["wake"]
        if proc:
            r["proc_start_to_wake"] = w - int(proc.group(1))
        for s in STAGES:
            if s in st:
                r[s] = st[s] - w
    return r


def summarize(runs, keys):
    for k in keys:
        v = [r[k] for r in runs if k in r]
        if v:
            print(f"{k:22s} n={len(v):2d} median={statistics.median(v):7.0f} "
                  f"worst={max(v):7.0f} best={min(v):7.0f}")


def main():
    p = argparse.ArgumentParser()
    p.add_argument("runs", type=int)
    p.add_argument("--serial")
    p.add_argument("--relay", default="")
    p.add_argument("--drop-caches", action="store_true")
    p.add_argument("--mem", action="store_true")
    p.add_argument("--label", default="run")
    a = p.parse_args()
    root = False
    if a.serial is None or a.serial.startswith("emulator"):
        out = subprocess.run(["adb"] + (["-s", a.serial] if a.serial else []) + ["root"],
                             capture_output=True, text=True).stdout
        root = "cannot" not in out
        time.sleep(2)
    adb(a.serial, "reverse", "tcp:3100", "tcp:3100")  # adb root drops reverses
    runs = []
    for i in range(a.runs):
        r = one_run(a, root)
        runs.append(r)
        print(i, json.dumps(r), flush=True)
    print(f"== {a.label}")
    summarize(runs, ["proc_start_to_wake"] + STAGES + ["pss_at_finish_kb"])
    peaks = [max(r["mem_samples_kb"]) for r in runs if r["mem_samples_kb"]]
    if peaks:
        print(f"peak dumpsys PSS KB median={statistics.median(peaks):.0f} max={max(peaks)}")
    print("results:", [r.get("result") for r in runs])


main()
