"""Exercise failure logging with real subprocesses and temporary output files."""
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from process_log import run_logged


class ProcessLogTests(unittest.TestCase):
    def test_outputs_are_saved_before_failure_is_reported(self):
        for combined in (False, True):
            for status in (0, 7):
                with self.subTest(combined=combined, status=status), tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    stdout = root / "stdout.log"
                    stderr = None if combined else root / "stderr.log"
                    command = [sys.executable, "-c", (
                        "import sys; "
                        "sys.stdout.reconfigure(encoding='utf-8'); "
                        "sys.stderr.reconfigure(encoding='utf-8'); "
                        "print('正文😀'); print('诊断信息', file=sys.stderr); "
                        f"sys.exit({status})"
                    )]
                    if status:
                        with self.assertRaises(subprocess.CalledProcessError) as raised:
                            run_logged(command, stdout, stderr)
                        self.assertEqual(raised.exception.returncode, status)
                        self.assertEqual(raised.exception.stdout, "正文😀\n")
                        self.assertEqual(raised.exception.stderr, "诊断信息\n")
                    else:
                        result = run_logged(command, stdout, stderr)
                        self.assertEqual(result.returncode, 0)
                        self.assertEqual(result.stdout, "正文😀\n")
                        self.assertEqual(result.stderr, "诊断信息\n")
                    expected = "正文😀\n诊断信息\n" if combined else "正文😀\n"
                    self.assertEqual(stdout.read_text(encoding="utf-8"), expected)
                    if stderr is not None:
                        self.assertEqual(stderr.read_text(encoding="utf-8"), "诊断信息\n")


if __name__ == "__main__":
    unittest.main()
