import os
import subprocess
from pathlib import Path
import unittest
from unittest.mock import patch, Mock

from macos.signing import Signing, staple_ticket


class SigningTests(unittest.TestCase):
    @patch.dict(os.environ, {}, clear=True)
    def test_unsigned_is_explicit_and_partial_configuration_fails(self):
        self.assertIsNone(Signing.from_env())
        with self.assertRaises(ValueError):
            Signing.from_env(required=True)
        os.environ["APPLE_SIGNING_IDENTITY"] = "Developer ID Application: Test"
        with self.assertRaises(ValueError):
            Signing.from_env()
        os.environ["APPLE_NOTARY_PROFILE"] = "test"
        self.assertIsInstance(Signing.from_env(required=True), Signing)
        os.environ["APPLE_SIGNING_IDENTITY"] = "Apple Development: Test"
        with self.assertRaises(ValueError):
            Signing.from_env()

    @patch("macos.signing.run")
    @patch("macos.signing.subprocess.run")
    def test_rejected_notarization_never_staples(self, submit, run):
        submit.return_value = Mock(returncode=0, stdout='{"status":"Invalid"}')
        with self.assertRaises(RuntimeError):
            Signing("identity", "profile").notarize(Path("app.zip"), Path("app.app"))
        run.assert_not_called()

    @patch("macos.signing.run")
    @patch("macos.signing.staple_ticket")
    @patch("macos.signing.subprocess.run")
    def test_accepted_notarization_staples_and_validates(self, submit, staple, run):
        submit.return_value = Mock(returncode=0, stdout='{"status":"Accepted"}')
        Signing("identity", "profile").notarize(Path("app.zip"), Path("app.app"))
        staple.assert_called_once_with(Path("app.app"))
        run.assert_called_once_with("xcrun", "stapler", "validate", Path("app.app"))

    @patch("macos.signing.time.sleep")
    @patch("macos.signing.subprocess.run")
    def test_ticket_network_failure_recovers(self, execute, sleep):
        execute.side_effect = [
            Mock(returncode=68, stdout="", stderr="NSURLErrorDomain Code=-1005"),
            Mock(returncode=0, stdout="Ticket stapled", stderr=""),
        ]
        staple_ticket(Path("app.app"))
        self.assertEqual(execute.call_count, 2)
        sleep.assert_called_once_with(10)

    @patch("macos.signing.time.sleep")
    @patch("macos.signing.subprocess.run")
    def test_ticket_retries_are_bounded_and_non_network_errors_fail(self, execute, sleep):
        execute.return_value = Mock(returncode=68, stdout="", stderr="NSURLErrorDomain Code=-1005")
        with self.assertRaises(subprocess.CalledProcessError):
            staple_ticket(Path("app.app"))
        self.assertEqual(execute.call_count, 5)
        self.assertEqual(sleep.call_count, 4)
        execute.reset_mock()
        sleep.reset_mock()
        execute.return_value = Mock(returncode=68, stdout="", stderr="Invalid ticket")
        with self.assertRaises(subprocess.CalledProcessError):
            staple_ticket(Path("app.app"))
        execute.assert_called_once()
        sleep.assert_not_called()
