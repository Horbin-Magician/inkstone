import tempfile
import unittest
from pathlib import Path

from package import inventory, verify
from package import digest
from verify_release import verify_release


class InventoryTests(unittest.TestCase):
    def test_release_requires_all_platforms_and_valid_checksums(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            for target, suffixes in (
                ("x86_64-pc-windows-msvc", (".zip", "-setup.exe")),
                ("aarch64-apple-darwin", (".dmg", ".app.tar.gz")),
                ("x86_64-apple-darwin", (".dmg", ".app.tar.gz")),
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
