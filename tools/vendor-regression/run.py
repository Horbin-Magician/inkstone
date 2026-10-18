#!/usr/bin/env python3
"""Test fresh copies of patched dependencies without changing the app lockfile."""
import argparse
from pathlib import Path
import shutil
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--prepare-only", action="store_true")
args = parser.parse_args()
source = Path(__file__).resolve().parent
root = source.parents[1]
staging = root / "target" / "vendor-regression"
staging.mkdir(parents=True, exist_ok=True)
for package in ("gpui-base", "gpui-component"):
    destination = staging / "crates" / package.removeprefix("gpui-")
    if destination.exists():
        shutil.rmtree(destination)
    shutil.copytree(root / "vendor" / package, destination,
                    ignore=shutil.ignore_patterns("target", "Cargo.lock", ".cargo-ok"))
for name in ("Cargo.toml", "Cargo.lock"):
    shutil.copyfile(source / name, staging / name)
shutil.copytree(source / "fixtures", staging, dirs_exist_ok=True)
if not args.prepare_only:
    # Observation wraps native elements only in the feature-enabled build.
    # Keep both configurations for each package: neither replaces the other.
    for package, features in (("gpui-base", []),
                              ("gpui-base", ["--features", "test-support"]),
                              ("gpui-component", []),
                              ("gpui-component", ["--features", "test-support"])):
        subprocess.run(["cargo", "test", "--locked", "--manifest-path",
                        str(staging / "Cargo.toml"), "--target-dir", str(root / "target"),
                        "-p", package, "--lib", *features, "--", "--test-threads=1"],
                       cwd=root, check=True)
