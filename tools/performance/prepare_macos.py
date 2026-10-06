#!/usr/bin/env python3
"""Prepare (never launch) an isolated macOS bundle and generated acceptance vault."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import uuid

import corpus

MODES = ("live", "source", "reading")


def prepare(output, binary, scenario, mode, trace_activity=False):
    data = corpus.manifest()
    # Fail before creating output if the generator has drifted from the fixed corpus.
    expected = json.loads(Path(__file__).with_name("corpus-v1.json").read_text())
    if data != expected:
        raise ValueError("generated corpus differs from corpus-v1.json")
    paths = data["scenarios"][scenario]
    if mode not in MODES:
        raise ValueError("unknown editor mode")
    binary = binary.resolve(strict=True)
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise ValueError("binary must be an executable file")
    output = output.absolute()
    # Do not resolve away a dangling symlink or reuse an existing test/user directory.
    output.mkdir(parents=True, exist_ok=False)
    output = output.resolve()
    vault = output / "Vault"
    vault.mkdir()
    for name, text in corpus.sources():
        path = vault / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(text.encode("utf-8"))
    (vault / ".performance-corpus.json").write_text(json.dumps(data, indent=2))
    prefs = dict(
        theme="dark", light=False, font_size=16.0, line_spacing=1.5,
        left_open=True, right_open=True, left_width=250.0, right_width=260.0,
        default_live_preview=mode != "source", default_reading=mode == "reading",
        open_paths=paths, active_path=paths[0], active_tab_index=0,
        main_path=paths[0], main_tab_index=0,
        views=[dict(path=p, live=mode != "source", reading=mode == "reading") for p in paths],
    )
    (vault / ".inkstone-workspace.json").write_text(json.dumps(prefs, indent=2))
    app_data = output / "data"
    (app_data / "Inkstone").mkdir(parents=True)
    (app_data / "Inkstone/recent.txt").write_text(str(vault))
    bundle = output / "Inkstone Performance Test.app"
    contents = bundle / "Contents"
    (contents / "MacOS").mkdir(parents=True)
    executable = contents / "MacOS/inkstone"
    shutil.copyfile(binary, executable)
    executable.chmod(0o755)
    identifier = "app.inkstone.performance-" + uuid.uuid4().hex
    info = dict(
        CFBundleIdentifier=identifier, CFBundleName="Inkstone Performance Test",
        CFBundleDisplayName="Inkstone Performance Test", CFBundleExecutable="inkstone",
        CFBundlePackageType="APPL", CFBundleVersion="1", CFBundleShortVersionString="0.1.0",
        NSHighResolutionCapable=True, LSEnvironment={"XDG_DATA_HOME": str(app_data)},
    )
    if trace_activity:
        info["LSEnvironment"]["INKSTONE_TRACE_ACTIVITY"] = str(output / "activity.jsonl")
    (contents / "Info.plist").write_bytes(plistlib.dumps(info))
    subprocess.run(["codesign", "--force", "--deep", "--sign", "-", str(bundle)], check=True)
    subprocess.run(["codesign", "--verify", "--deep", "--strict", str(bundle)], check=True)
    report = dict(
        bundle=str(bundle), bundle_id=identifier, vault=str(vault),
        environment=info["LSEnvironment"], binary=str(binary),
        signed_binary_sha256=hashlib.sha256(executable.read_bytes()).hexdigest(),
        scenario=scenario, mode=mode, corpus=data,
        note="Preparation only; no application launched or performance acceptance measured.",
    )
    (output / "preparation.json").write_text(json.dumps(report, indent=2))
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--binary", type=Path, default=Path("target/release/inkstone"))
    parser.add_argument("--scenario", choices=corpus.manifest()["scenarios"], required=True)
    parser.add_argument("--mode", choices=MODES, default="live")
    parser.add_argument("--trace-activity", action="store_true", help="opt in to bounded window/focus samples in the isolated output")
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("this preparation tool requires macOS codesign")
    print(json.dumps(prepare(args.output, args.binary, args.scenario, args.mode, args.trace_activity), indent=2))


if __name__ == "__main__":
    main()
