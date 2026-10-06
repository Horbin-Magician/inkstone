"""Exercise real process lifecycle checks without starting the editor."""
import json
from pathlib import Path
import subprocess
import sys
import time
import unittest


SAMPLER = Path(__file__).with_name("sample_process.py")


@unittest.skipUnless(sys.platform in ("darwin", "linux"), "sampler supports macOS/Linux")
class ProcessSamplingTests(unittest.TestCase):
    def sample(self, pid):
        return subprocess.run(
            [sys.executable, str(SAMPLER), str(pid), "--settle-seconds", ".2",
             "--seconds", ".2", "--interval", ".1"],
            capture_output=True, text=True, timeout=5,
        )

    def test_live_process_records_separate_settle_and_sample_intervals(self):
        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(10)"])
        try:
            result = self.sample(child.pid)
            self.assertEqual(result.returncode, 0, result.stderr)
            report = json.loads(result.stdout)
            self.assertGreaterEqual(report["observed_settle_s"], .2)
            self.assertGreaterEqual(report["duration_s"], .2)
            self.assertEqual(report["pid"], child.pid)
        finally:
            child.terminate()
            child.wait(timeout=5)

    def test_exited_unreaped_process_cannot_produce_a_report(self):
        child = subprocess.Popen([sys.executable, "-c", "pass"])
        try:
            # Do not poll/wait yet: retain the exited process as a zombie.
            time.sleep(.2)
            result = self.sample(child.pid)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, "")
        finally:
            child.wait(timeout=5)

    def test_short_lived_process_cannot_produce_a_report(self):
        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(.1)"])
        try:
            result = self.sample(child.pid)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, "")
        finally:
            child.wait(timeout=5)


if __name__ == "__main__":
    unittest.main()
