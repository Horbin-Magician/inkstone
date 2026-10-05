#!/usr/bin/env python3
"""Sample one explicit native process on macOS/Linux; emit JSON, no UI interaction."""
import argparse
import json
import platform
import statistics
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pid", type=int)
    parser.add_argument("--seconds", type=float, default=30)
    parser.add_argument("--interval", type=float, default=0.5)
    args = parser.parse_args()
    if args.pid <= 0 or not 0 < args.seconds <= 60 or not 0.1 <= args.interval <= args.seconds:
        parser.error("require PID > 0, 0 < seconds <= 60, 0.1 <= interval <= seconds")
    # Start time detects PID reuse instead of sampling an unrelated process.
    def read():
        result = subprocess.run(
            ["ps", "-p", str(args.pid), "-o", "lstart=", "-o", "rss=", "-o", "%cpu="],
            check=True, capture_output=True, text=True,
        )
        fields = result.stdout.strip().rsplit(None, 2)
        if len(fields) != 3:
            raise RuntimeError("target process no longer exists")
        return fields[0], int(fields[1]), float(fields[2])

    identity, _, _ = read()
    start = time.monotonic()
    samples = []
    while True:
        current, rss, cpu = read()
        if current != identity:
            raise RuntimeError("target PID was reused; discard this run")
        elapsed = time.monotonic() - start
        samples.append({"elapsed_s": elapsed, "rss_kib": rss, "ps_cpu_percent": cpu})
        if elapsed >= args.seconds:
            break
        time.sleep(min(args.interval, args.seconds - elapsed))
    print(json.dumps({
        "platform": platform.platform(), "pid": args.pid, "process_start": identity,
        "duration_s": samples[-1]["elapsed_s"], "interval_s": args.interval,
        "observed_peak_rss_kib": max(s["rss_kib"] for s in samples),
        "median_ps_cpu_percent": statistics.median(s["ps_cpu_percent"] for s in samples),
        "caveat": "Sampled RSS is a lower bound on peak, excludes GPU/helper processes. ps CPU is OS-defined averaged usage, not interval CPU or input latency.",
        "samples": samples,
    }, indent=2))


if __name__ == "__main__":
    main()
