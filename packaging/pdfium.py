"""Install the pinned PDFium runtime and its redistribution notices."""
import hashlib
import io
from pathlib import Path
import sys
import tarfile
import urllib.request

BUILDS = {
    "mac-arm64": ("lib/libpdfium.dylib", "4939bd935ac83ddc09a3a4f038bc43a19cea1660316a3667f26cb07a070e139d"),
    "win-x64": ("bin/pdfium.dll", "9a964261760cff52570aabf038ab346d3a5034dd8b30d0d5fe8861235d356b8b"),
}


def install(platform, destination):
    member, digest = BUILDS[platform]
    url = f"https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F8125/pdfium-{platform}.tgz"
    with urllib.request.urlopen(url, timeout=90) as response:
        data = response.read()
    if hashlib.sha256(data).hexdigest() != digest:
        raise ValueError("unexpected PDFium archive checksum")
    with tarfile.open(fileobj=io.BytesIO(data)) as archive:
        notices = destination / "pdfium-licenses"
        notices.mkdir(exist_ok=True)
        for entry in archive.getmembers():
            if entry.isfile() and (entry.name == "LICENSE" or entry.name.startswith("licenses/")):
                (notices / Path(entry.name).name).write_bytes(archive.extractfile(entry).read())
        (destination / Path(member).name).write_bytes(archive.extractfile(member).read())


if __name__ == "__main__":
    install(sys.argv[1], Path(sys.argv[2]))
