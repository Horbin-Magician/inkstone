"""Require the full platform matrix and verify every downloadable package."""
import sys
from pathlib import Path

from package import digest


def verify_release(directory, version):
    expected = set()
    for target, suffixes in (
        ("x86_64-pc-windows-msvc", (".zip", "-setup.exe")),
        ("aarch64-apple-darwin", (".dmg", ".app.tar.gz")),
    ):
        stem = f"inkstone-{version}-{target}"
        packages = {stem + suffix for suffix in suffixes}
        manifest = stem + ".sha256"
        expected.update(packages | {manifest})
        lines = (directory / manifest).read_text(encoding="utf-8").splitlines()
        checksums = dict(line.split("  ", 1)[::-1] for line in lines)
        if set(checksums) != packages or len(lines) != len(packages):
            raise ValueError(f"Unexpected manifest entries: {manifest}")
        for name, checksum in checksums.items():
            if digest(directory / name) != checksum:
                raise ValueError(f"Checksum mismatch: {name}")
    if {p.name for p in directory.iterdir()} != expected:
        raise ValueError("Unexpected release attachments")


if __name__ == "__main__":
    verify_release(Path(sys.argv[1]), sys.argv[2])
