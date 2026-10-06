#!/usr/bin/env python3
"""macOS release comparison of old and buffered legacy journal decoding.

Uses only generated data in a fresh target directory. Build the example first:
cargo build --locked --release -p inkstone-core --example history_read_benchmark
"""
import hashlib
import json
from pathlib import Path
import platform
import re
import statistics
import subprocess
import tempfile


def main():
    if platform.system() != "Darwin":
        raise SystemExit("This RSS runner requires macOS /usr/bin/time -l (RSS in bytes).")
    project = Path(__file__).resolve().parents[2]
    binary = project / "target/release/examples/history_read_benchmark"
    if not binary.is_file():
        raise SystemExit("Build the release history_read_benchmark example first.")
    root = Path(tempfile.mkdtemp(prefix="history-read-", dir=project / "target"))
    unit = '中文 😀 é "quoted" \\ path\r\n'
    body = unit * (32 * 1024 * 1024 // len(unit.encode()))
    fixture = root / "legacy.json"
    fixture.write_text(json.dumps({
        "root": "/synthetic/vault", "relative": "中文.md",
        "baseline": body, "draft": body,
    }, ensure_ascii=False), encoding="utf-8")
    metadata = {
        "fixture_sha256": hashlib.sha256(fixture.read_bytes()).hexdigest(),
        "file_bytes": fixture.stat().st_size,
        "body_bytes_each": len(body.encode()),
        "platform": platform.platform(),
        "machine": platform.machine(),
        "revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=project, text=True).strip(),
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "rustc": subprocess.check_output(["rustc", "--version"], cwd=project, text=True).strip(),
    }
    (root / "fixture.json").write_text(json.dumps(metadata, indent=2), encoding="utf-8")
    del body
    runs = []
    for iteration in range(3):
        modes = ["whole", "buffered"] if iteration % 2 == 0 else ["buffered", "whole"]
        for mode in modes:
            result = subprocess.run(
                ["/usr/bin/time", "-l", str(binary), mode, str(fixture)],
                capture_output=True, text=True, check=True,
            )
            (root / f"{mode}-{iteration}.log").write_text(result.stdout + result.stderr, encoding="utf-8")
            elapsed = float(re.search(r"elapsed_ms=([\d.]+)", result.stdout)[1])
            rss = int(re.search(r"(\d+)\s+maximum resident set size", result.stderr)[1])
            runs.append({"mode": mode, "iteration": iteration, "elapsed_ms": elapsed, "max_rss_bytes": rss})
    summary = {
        mode: {
            "elapsed_ms_median": statistics.median(r["elapsed_ms"] for r in runs if r["mode"] == mode),
            "rss_mib_median": statistics.median(r["max_rss_bytes"] for r in runs if r["mode"] == mode) / 1048576,
        }
        for mode in ["whole", "buffered"]
    }
    (root / "results.json").write_text(json.dumps({"runs": runs, "summary": summary}, indent=2), encoding="utf-8")
    print(root)
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
