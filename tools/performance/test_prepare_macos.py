"""Verify fixture isolation using real macOS signing, without launching an app."""
import hashlib
import json
from pathlib import Path
import plistlib
import sys
import tempfile
import unittest

import prepare_macos


@unittest.skipUnless(sys.platform == "darwin", "requires macOS codesign")
class PreparationTests(unittest.TestCase):
    def test_existing_directory_and_dangling_symlink_are_not_reused(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            existing = root / "existing"
            existing.mkdir()
            sentinel = existing / "note.md"
            sentinel.write_text("keep this content")
            link = root / "link"
            link.symlink_to(root / "missing")
            for destination in (existing, link):
                with self.assertRaises(FileExistsError):
                    prepare_macos.prepare(destination, Path("/usr/bin/true"), "ordinary", "live")
            self.assertEqual(sentinel.read_text(), "keep this content")
            self.assertEqual(list(existing.iterdir()), [sentinel])
            self.assertTrue(link.is_symlink())
            self.assertFalse((root / "missing").exists())

    def test_invalid_request_does_not_create_output(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / "new"
            with self.assertRaises(ValueError):
                prepare_macos.prepare(output, Path("/usr/bin/true"), "ordinary", "invalid")
            self.assertFalse(output.exists())
            with self.assertRaises(FileNotFoundError):
                prepare_macos.prepare(output, Path(tmp) / "missing", "ordinary", "live")
            self.assertFalse(output.exists())

    def test_all_scenarios_and_modes_keep_relaunch_inside_their_own_fixture(self):
        with tempfile.TemporaryDirectory() as tmp:
            ids = set()
            for scenario in prepare_macos.corpus.manifest()["scenarios"]:
                for mode in prepare_macos.MODES:
                    root = Path(tmp).resolve() / f"{scenario}-{mode}"
                    report = prepare_macos.prepare(root, Path("/usr/bin/true"), scenario, mode)
                    info = plistlib.loads((Path(report["bundle"]) / "Contents/Info.plist").read_bytes())
                    self.assertNotIn(info["CFBundleIdentifier"], ids)
                    ids.add(info["CFBundleIdentifier"])
                    data = Path(info["LSEnvironment"]["XDG_DATA_HOME"])
                    self.assertEqual(data.parent, root)
                    vault = Path((data / "Inkstone/recent.txt").read_text())
                    self.assertEqual(vault, root / "Vault")
                    prefs = json.loads((vault / ".inkstone-workspace.json").read_text())
                    self.assertEqual(len(prefs["open_paths"]), 12 if scenario == "multiple-tabs" else 1)
                    self.assertEqual(prefs["active_path"], prefs["open_paths"][0])
                    self.assertEqual(prefs["default_reading"], mode == "reading")
                    self.assertEqual(prefs["default_live_preview"], mode != "source")
                    for entry in report["corpus"]["files"]:
                        self.assertEqual(hashlib.sha256((vault / entry["path"]).read_bytes()).hexdigest(), entry["sha256"])


if __name__ == "__main__":
    unittest.main()
