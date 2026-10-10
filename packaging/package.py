#!/usr/bin/env python3
"""Build and package the native host target (Python 3.10+)."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tarfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def run(*args):
    subprocess.run([str(arg) for arg in args], cwd=ROOT, check=True)


def metadata():
    data = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"], cwd=ROOT
    ))
    return next(p for p in data["packages"] if p["name"] == "inkstone-desktop")


def digest(path):
    result = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def inventory(stage, version, target, manifest="resources.json"):
    files = {p.relative_to(stage).as_posix(): digest(p)
             for p in sorted(stage.rglob("*")) if p.is_file() and p.name != manifest}
    (stage / manifest).write_text(json.dumps({
        "version": version, "target": target, "files": files,
    }, indent=2) + "\n", encoding="utf-8")


def verify(stage, manifest="resources.json"):
    data = json.loads((stage / manifest).read_text(encoding="utf-8"))
    actual = {p.relative_to(stage).as_posix(): digest(p)
              for p in stage.rglob("*") if p.is_file() and p.name != manifest}
    if actual != data["files"]:
        raise ValueError("Package inventory mismatch")


def dmg_size_mb(directory):
    """Reserve filesystem overhead instead of relying on hdiutil's size estimate."""
    size = sum(p.stat().st_size for p in directory.rglob("*")
               if p.is_file() and not p.is_symlink())
    return (size * 12 + 10 * 1024 * 1024 - 1) // (10 * 1024 * 1024) + 64


def prepare_dmg(stage, image):
    """Show only the drag-to-install pair; keep attribution files hidden."""
    image.mkdir()
    shutil.copytree(stage / "墨砚.app", image / "墨砚.app")
    (image / "Applications").symlink_to("/Applications")
    support = image / ".support"
    support.mkdir()
    for path in stage.iterdir():
        if path.name in ("墨砚.app", "GUIDE.md"):
            continue
        if path.is_dir():
            shutil.copytree(path, support / path.name)
        else:
            shutil.copy2(path, support / path.name)
    # Paths changed, so the DMG gets its own inventory covering the whole payload.
    (support / "resources.json").unlink()
    data = json.loads((stage / "resources.json").read_text(encoding="utf-8"))
    # Keep the machine-readable inventory out of Finder's install view.
    inventory(image, data["version"], data["target"], manifest=".resources.json")
    verify(image, manifest=".resources.json")


def package(output):
    system = platform.system()
    host = subprocess.check_output(["rustc", "-vV"], text=True).split("host: ")[1].splitlines()[0]
    if system not in ("Windows", "Darwin") or host not in (
        "x86_64-pc-windows-msvc", "aarch64-apple-darwin"
    ):
        raise ValueError(f"Unsupported native host: {host}")
    if system == "Darwin":
        # Fail before building if the DMG metadata dependencies are unavailable.
        from macos.dmg import build_dmg
    version = metadata()["version"]
    compiler = None
    if system == "Windows":
        compiler = os.environ.get("NSIS_MAKENSIS") or shutil.which("makensis")
        if not compiler:
            candidate = Path(os.environ.get("ProgramFiles(x86)", "C:/Program Files (x86)")) / "NSIS/makensis.exe"
            compiler = str(candidate) if candidate.is_file() else None
        if not compiler:
            raise ValueError("NSIS 3 required; set NSIS_MAKENSIS to makensis.exe")
    output = output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    stage = output / "stage"
    stage.mkdir()
    if system == "Windows":
        # Explicit --target keeps this flag off host proc-macro/build-script crates.
        # Static CRT makes the portable archive usable without a VC++ installation.
        subprocess.run([
            "cargo", "build", "--release", "--locked", "--bin", "inkstone", "--target", host,
            "--target-dir", str(ROOT / "target"),
        ], cwd=ROOT, check=True, env={
            **os.environ,
            "CARGO_ENCODED_RUSTFLAGS": "-C\x1ftarget-feature=+crt-static",
        })
        shutil.copy2(ROOT / "target" / host / "release/inkstone.exe", stage)
    else:
        subprocess.run(["bash", str(ROOT / "packaging/macos/bundle.sh"), "--release"],
                       cwd=ROOT, check=True,
                       env={**os.environ, "INKSTONE_BUNDLE_PATH": str(stage / "墨砚.app")})
        run("codesign", "--verify", "--deep", "--strict", stage / "墨砚.app")
    shutil.copy2(ROOT / "LICENSE", stage)
    shutil.copytree(ROOT / "licenses", stage / "licenses")
    shutil.copy2(ROOT / "docs/GUIDE.md", stage / "GUIDE.md")
    inventory(stage, version, host)
    verify(stage)
    stem = f"inkstone-{version}-{host}"
    if system == "Windows":
        with zipfile.ZipFile(output / f"{stem}.zip", "w", zipfile.ZIP_DEFLATED) as archive:
            for path in sorted(stage.rglob("*")):
                if path.is_file():
                    archive.write(path, path.relative_to(stage))
        run(compiler, "/INPUTCHARSET", "UTF8", f"/DVERSION={version}", f"/DSTAGE={stage}",
            f"/DOUTPUT={output / (stem + '-setup.exe')}", ROOT / "packaging/windows/installer.nsi")
    else:
        with tarfile.open(output / f"{stem}.app.tar.gz", "w:gz") as archive:
            for path in sorted(stage.iterdir()):
                archive.add(path, arcname=path.name)
        image = output / "dmg-root"
        prepare_dmg(stage, image)
        build_dmg(image, output / f"{stem}.dmg", dmg_size_mb(image))
    assets = sorted(p for p in output.iterdir() if p.is_file())
    (output / f"{stem}.sha256").write_text(
        "".join(f"{digest(p)}  {p.name}\n" for p in assets), encoding="utf-8")
    print(f"Packaged {version}: {output}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="New output directory; existing paths are rejected")
    args = parser.parse_args()
    package(args.output)
