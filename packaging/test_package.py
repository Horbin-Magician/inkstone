import sys
import tempfile
import unittest
from pathlib import Path

from package import inventory, verify, prepare_dmg
from package import digest, dmg_size_mb
from verify_release import verify_release


class InventoryTests(unittest.TestCase):
    @unittest.skipUnless(sys.platform == "darwin", "DMG staging requires macOS symlinks")
    def test_dmg_layout_preserves_payload_and_rebuilds_inventory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            stage, image = root / "stage", root / "dmg"
            app = stage / "墨砚.app/Contents/MacOS/inkstone"
            app.parent.mkdir(parents=True)
            app.write_bytes(b"app fixture")
            (stage / "licenses").mkdir()
            (stage / "licenses/font.txt").write_text("font license")
            (stage / "GUIDE.md").write_text("guide")
            (stage / "LICENSE").write_text("license")
            inventory(stage, "0.1.0", "aarch64-apple-darwin")
            prepare_dmg(stage, image)
            self.assertEqual({p.name for p in image.iterdir() if not p.name.startswith(".")},
                             {"墨砚.app", "Applications"})
            self.assertEqual((image / "Applications").readlink(), Path("/Applications"))
            self.assertEqual((image / ".support/licenses/font.txt").read_text(), "font license")
            self.assertFalse((image / ".support/GUIDE.md").exists())
            verify(stage)
            verify(image, manifest=".resources.json")
            (image / "墨砚.app/Contents/MacOS/inkstone").write_bytes(b"corrupt")
            with self.assertRaises(ValueError):
                verify(image, manifest=".resources.json")

    def test_dmg_capacity_includes_nested_payload_and_filesystem_headroom(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.assertEqual(dmg_size_mb(root), 64)
            binary = root / "app/Contents/MacOS/inkstone"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"x" * (5 * 1024 * 1024))
            (root / "GUIDE.md").write_bytes(b"x")
            self.assertEqual(dmg_size_mb(root), 71)

    def test_release_requires_all_platforms_and_valid_checksums(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            for target, suffixes in (
                ("x86_64-pc-windows-msvc", (".zip", "-setup.exe")),
                ("aarch64-apple-darwin", (".dmg", ".app.tar.gz")),
            ):
                stem = f"inkstone-0.1.0-{target}"
                lines = []
                for suffix in suffixes:
                    path = output / (stem + suffix)
                    path.write_bytes(b"fixture")
                    lines.append(f"{digest(path)}  {path.name}\n")
                (output / (stem + ".sha256")).write_text("".join(lines))
            verify_release(output, "0.1.0")
            path.write_bytes(b"corrupt")
            with self.assertRaises(ValueError):
                verify_release(output, "0.1.0")
            path.unlink()
            with self.assertRaises(FileNotFoundError):
                verify_release(output, "0.1.0")

    def test_integrity_and_unexpected_files(self):
        with tempfile.TemporaryDirectory() as directory:
            stage = Path(directory)
            binary = stage / "inkstone.exe"
            binary.write_bytes(b"test binary")
            inventory(stage, "0.1.0", "x86_64-pc-windows-msvc")
            verify(stage)
            binary.write_bytes(b"tampered")
            with self.assertRaises(ValueError):
                verify(stage)
            binary.write_bytes(b"test binary")
            extra = stage / "extra.dll"
            extra.write_bytes(b"unexpected")
            with self.assertRaises(ValueError):
                verify(stage)
            extra.unlink()
            binary.unlink()
            with self.assertRaises(ValueError):
                verify(stage)


if __name__ == "__main__":
    unittest.main()
