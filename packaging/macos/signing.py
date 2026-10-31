"""Developer ID signing and notarization using credentials in the Keychain."""
import json
import os
from pathlib import Path
import subprocess
import tempfile


def run(*args):
    subprocess.run([str(arg) for arg in args], check=True)


class Signing:
    def __init__(self, identity, profile):
        self.identity = identity
        self.profile = profile

    @classmethod
    def from_env(cls, required=False):
        identity = os.environ.get("APPLE_SIGNING_IDENTITY")
        profile = os.environ.get("APPLE_NOTARY_PROFILE")
        if identity is None and profile is None and not required:
            return None
        if not identity or not identity.startswith("Developer ID Application: ") or not profile or not profile.strip():
            raise ValueError("Set APPLE_SIGNING_IDENTITY (Developer ID Application) and APPLE_NOTARY_PROFILE (notarytool Keychain profile)")
        return cls(identity, profile)

    def sign(self, path, runtime=False):
        args = ["codesign", "--force", "--timestamp", "--sign", self.identity]
        if runtime:
            args += ["--options", "runtime"]
        run(*args, path)

    def notarize(self, upload, staple):
        args = ["xcrun", "notarytool", "submit", str(upload),
                "--keychain-profile", self.profile, "--wait", "--output-format", "json"]
        if os.environ.get("APPLE_NOTARY_KEYCHAIN"):
            args += ["--keychain", os.environ["APPLE_NOTARY_KEYCHAIN"]]
        print(f"Submitting to Apple: {upload}", flush=True)
        result = subprocess.run(args, text=True, capture_output=True)
        if result.returncode:
            raise RuntimeError(f"Notarization failed: {result.stdout}\n{result.stderr}")
        response = json.loads(result.stdout)
        print(json.dumps(response), flush=True)
        if response.get("status") != "Accepted":
            raise RuntimeError("Apple did not accept the upload; do not distribute this output")
        run("xcrun", "stapler", "staple", staple)
        run("xcrun", "stapler", "validate", staple)

    def app(self, app):
        # The current bundle contains one executable and no nested code bundles.
        for library in sorted((app / "Contents/MacOS").glob("*.dylib")):
            self.sign(library, runtime=True)
        self.sign(app, runtime=True)
        run("codesign", "--verify", "--deep", "--strict", "--verbose=2", app)
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "notarization.zip"
            run("ditto", "-c", "-k", "--keepParent", app, archive)
            self.notarize(archive, app)
        run("spctl", "--assess", "--type", "execute", "--verbose=2", app)

    def dmg(self, dmg):
        self.sign(dmg)
        self.notarize(dmg, dmg)
        run("codesign", "--verify", "--strict", dmg)
        run("spctl", "--assess", "--type", "open", "--context",
            "context:primary-signature", "--verbose=2", dmg)
