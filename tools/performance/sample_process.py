#!/usr/bin/env python3
"""Sample one explicit native process on macOS/Linux; emit JSON, no UI interaction."""
import argparse
import json
import platform
import statistics
import subprocess
import time


def cpu_seconds(value):
    """ps cumulative CPU time: [[days-]hours:]minutes:seconds[.fraction]."""
    days, value = value.split("-", 1) if "-" in value else ("0", value)
    total = 0.0
    for part in value.split(":"):
        total = total * 60 + float(part)
    return int(days) * 86400 + total


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pid", type=int)
    parser.add_argument("--seconds", type=float, default=30)
    parser.add_argument("--interval", type=float, default=0.5)
    parser.add_argument("--settle-seconds", type=float, default=0, help="observe the same PID before measurement; exclude settling CPU")
    args = parser.parse_args()
    if args.pid <= 0 or not 0 < args.seconds <= 60 or not 0.1 <= args.interval <= args.seconds or not 0 <= args.settle_seconds <= 60:
        parser.error("require PID > 0, 0 < seconds <= 60, 0.1 <= interval <= seconds, 0 <= settle-seconds <= 60")
    # Start time detects PID reuse instead of sampling an unrelated process.
    def read():
        result = subprocess.run(
            ["ps", "-p", str(args.pid), "-o", "lstart=", "-o", "rss=", "-o", "%cpu=", "-o", "time=", "-o", "stat="],
            check=True, capture_output=True, text=True,
        )
        fields = result.stdout.strip().rsplit(None, 4)
        if len(fields) != 5:
            raise RuntimeError("target process no longer exists")
        if fields[4].startswith(("Z", "X")):
            raise RuntimeError("target process has exited; discard this run")
        return fields[0], int(fields[1]), float(fields[2]), cpu_seconds(fields[3])

    identity, _, _, _ = read()
    settle_start = time.monotonic()
    while time.monotonic() - settle_start < args.settle_seconds:
        time.sleep(min(args.interval, max(0, args.settle_seconds - (time.monotonic() - settle_start))))
        if read()[0] != identity:
            raise RuntimeError("target PID was reused during settling; discard this run")
    settle_duration = time.monotonic() - settle_start
    current, _, _, first_cpu = read()
    if current != identity:
        raise RuntimeError("target PID was reused before sampling; discard this run")
    start = time.monotonic()
    samples = []
    while True:
        current, rss, cpu, cumulative = read()
        if current != identity:
            raise RuntimeError("target PID was reused; discard this run")
        elapsed = time.monotonic() - start
        samples.append({"elapsed_s": elapsed, "rss_kib": rss, "ps_cpu_percent": cpu, "cumulative_cpu_s": cumulative})
        if elapsed >= args.seconds:
            break
        time.sleep(min(args.interval, args.seconds - elapsed))
    print(json.dumps({
        "platform": platform.platform(), "pid": args.pid, "process_start": identity,
        "duration_s": samples[-1]["elapsed_s"], "interval_s": args.interval,
        "requested_settle_s": args.settle_seconds, "observed_settle_s": settle_duration,
        "observed_peak_rss_kib": max(s["rss_kib"] for s in samples),
        "interval_cpu_percent": (samples[-1]["cumulative_cpu_s"] - first_cpu) / samples[-1]["elapsed_s"] * 100,
        "median_ps_cpu_percent": statistics.median(s["ps_cpu_percent"] for s in samples),
        "caveat": "Sampled RSS is a lower bound on peak, excludes GPU/helper processes. Interval CPU uses cumulative process CPU delta / monotonic wall time (100% = one core), subject to ps time precision. ps CPU is OS-defined averaged usage. Neither measures input latency.",
        "samples": samples,
    }, indent=2))


if __name__ == "__main__":
    main()
