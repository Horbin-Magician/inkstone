import os
from pathlib import Path
import unittest
from unittest.mock import patch, Mock

from macos.signing import Signing


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
    @patch("macos.signing.subprocess.run")
    def test_accepted_notarization_staples_and_validates(self, submit, run):
        submit.return_value = Mock(returncode=0, stdout='{"status":"Accepted"}')
        Signing("identity", "profile").notarize(Path("app.zip"), Path("app.app"))
        self.assertEqual([call.args[:3] for call in run.call_args_list],
                         [("xcrun", "stapler", "staple"), ("xcrun", "stapler", "validate")])
