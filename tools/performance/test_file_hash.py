import tempfile
import unittest
from pathlib import Path

from file_hash import sha256, verify_unchanged


class ArtifactIdentityTests(unittest.TestCase):
    def test_same_length_rebuild_is_rejected_and_missing_binary_is_not_accepted(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "benchmark"
            binary.write_bytes(b"first build")
            expected = sha256(binary)
            verify_unchanged(binary, expected)
            binary.write_bytes(b"other build")
            with self.assertRaisesRegex(ValueError, "changed during measurement"):
                verify_unchanged(binary, expected)
            binary.unlink()
            with self.assertRaises(FileNotFoundError):
                verify_unchanged(binary, expected)


if __name__ == "__main__":
    unittest.main()
