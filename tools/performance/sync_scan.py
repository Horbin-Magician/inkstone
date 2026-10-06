#!/usr/bin/env python3
"""macOS sync scan benchmark using fresh generated fixtures, never user notes."""
import hashlib
import json
from pathlib import Path
import platform
import re
import statistics
import subprocess
import sys
import tempfile


def main():
    if sys.platform != "darwin":
        raise SystemExit("RSS collection currently requires macOS /usr/bin/time -l")
    project = Path(__file__).resolve().parents[2]
    binary = project / "target/release/examples/sync_scan_benchmark"
    if not binary.is_file():
        raise SystemExit("build the sync_scan_benchmark release example first")
    root = Path(tempfile.mkdtemp(prefix="sync-scan-", dir=project / "target"))
    manifest = {
        "version": 1,
        "revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=project, text=True).strip(),
        "platform": platform.platform(),
        "rustc": subprocess.check_output(["rustc", "--version"], cwd=project, text=True).strip(),
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "example_sha256": hashlib.sha256((project / "crates/inkstone-core/examples/sync_scan_benchmark.rs").read_bytes()).hexdigest(),
        "fixtures": [],
    }
    print(root, flush=True)
    for name, count, size in [("small-files", 10_000, 1024), ("large-files", 9, 64 * 1024 * 1024)]:
        vault = root / name
        vault.mkdir()
        entries = []
        for i in range(count):
            path = vault / f"group-{i // 100:03}" / f"file-{i:05}.bin"
            path.parent.mkdir(exist_ok=True)
            unit = hashlib.sha256(f"sync-scan-v1/{name}/{i}".encode()).digest()
            chunk = unit * (min(size, 64 * 1024) // len(unit))
            digest = hashlib.sha256()
            with path.open("xb") as output:
                remaining = size
                while remaining:
                    part = chunk[:remaining]
                    output.write(part)
                    digest.update(part)
                    remaining -= len(part)
            entries.append({"path": str(path.relative_to(vault)), "bytes": size, "sha256": digest.hexdigest()})
        manifest["fixtures"].append({"name": name, "count": count, "bytes": count * size, "files": entries})
    (root / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    results = []
    for round_number in range(1, 4):
        for fixture in manifest["fixtures"]:
            name = fixture["name"]
            result = subprocess.run(
                ["/usr/bin/time", "-l", str(binary), str(root / name),
                 str(root / f"recovery-{name}-{round_number}")],
                capture_output=True, text=True, check=True,
            )
            (root / f"{name}-{round_number}.stdout").write_text(result.stdout)
            (root / f"{name}-{round_number}.time").write_text(result.stderr)
            measured = json.loads(result.stdout)
            assert measured["files"] == fixture["count"]
            assert measured["bytes"] == fixture["bytes"]
            assert measured["cancelled_before_remote"]
            measured.update({"fixture": name, "round": round_number,
                             "peak_rss_bytes": int(re.search(r"(\d+)\s+maximum resident set size", result.stderr)[1])})
            results.append(measured)
    # Independent content verification is outside the measured processes.
    for fixture in manifest["fixtures"]:
        for entry in fixture["files"]:
            with (root / fixture["name"] / entry["path"]).open("rb") as source:
                assert hashlib.file_digest(source, "sha256").hexdigest() == entry["sha256"]
    medians = {fixture["name"]: {
        key: statistics.median(result[key] for result in results if result["fixture"] == fixture["name"])
        for key in ["enumeration_ms", "file_processing_ms", "manifest_validation_ms", "scan_ms", "peak_rss_bytes"]
    } for fixture in manifest["fixtures"]}
    (root / "results.json").write_text(json.dumps({"runs": results, "medians": medians, "content_verified": True}, indent=2) + "\n")
    print(json.dumps(medians, indent=2), flush=True)


if __name__ == "__main__":
    main()
