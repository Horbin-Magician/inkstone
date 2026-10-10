"""Build a styled DMG without driving Finder or requesting UI permissions."""
import json
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile

from .dmg_layout import write_layout


def build_dmg(image, output, size_mb):
    from package import inventory, verify

    with tempfile.TemporaryDirectory(prefix="inkstone-dmg-") as temporary:
        writable = Path(temporary) / "layout.dmg"
        mount = Path(temporary) / "volume"
        subprocess.run(["hdiutil", "create", "-volname", "墨砚", "-srcfolder", str(image),
                        "-size", f"{size_mb}m", "-fs", "HFS+", "-format", "UDRW",
                        str(writable)], check=True)
        attached = subprocess.check_output([
            "hdiutil", "attach", "-nobrowse", "-mountpoint", str(mount), "-plist", str(writable)
        ])
        device = next(item["dev-entry"] for item in plistlib.loads(attached)["system-entities"]
                      if item.get("mount-point"))
        try:
            shutil.copy2(Path(__file__).with_name("background.tiff"), mount / ".background.tiff")
            write_layout(mount)
            data = json.loads((mount / ".resources.json").read_text(encoding="utf-8"))
            inventory(mount, data["version"], data["target"], manifest=".resources.json")
            verify(mount, manifest=".resources.json")
        finally:
            subprocess.run(["hdiutil", "detach", device], check=True)
        subprocess.run(["hdiutil", "convert", str(writable), "-format", "UDZO",
                        "-o", str(output)], check=True)
