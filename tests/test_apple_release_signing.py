import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


UPLOAD = Path(__file__).resolve().parents[1] / "apps/native/apple/upload-testflight.sh"


class AppleReleaseSigningTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.app = self.root / "app files"
        self.app.mkdir()
        shutil.copyfile(UPLOAD, self.app / "upload-testflight.sh")
        prepare = self.app / "prepare.sh"
        prepare.write_text('#!/usr/bin/env bash\ntouch "$(dirname "$0")/prepared"\n')
        prepare.chmod(0o755)
        binary = self.root / "bin"
        binary.mkdir()
        xcodebuild = binary / "xcodebuild"
        xcodebuild.write_text("""#!/usr/bin/env python3
import json
import os
from pathlib import Path
import plistlib
import sys

assert (Path(os.environ["TEST_APP_ROOT"]) / "prepared").is_file()
args = sys.argv[1:]
stage = "export" if "-exportArchive" in args else "archive"
record = {"args": args}
if stage == "export":
    with open(args[args.index("-exportOptionsPlist") + 1], "rb") as options:
        record["options"] = plistlib.load(options)
with open(os.environ["TEST_LOG"], "a") as log:
    log.write(json.dumps(record) + "\\n")
if os.environ.get("FAIL_STAGE") == stage:
    sys.exit(23)
""")
        xcodebuild.chmod(0o755)
        self.log = self.root / "commands.jsonl"
        self.env = {
            **os.environ,
            "PATH": f"{binary}:{os.environ['PATH']}",
            "TEST_APP_ROOT": str(self.app),
            "TEST_LOG": str(self.log),
            "APPLE_TEAM_ID": "FIXTURETEAM",
            "NOTARY_KEY_PATH": str(self.root / "key with spaces.p8"),
            "NOTARY_KEY_ID": "fixture-key-id",
            "NOTARY_ISSUER": "fixture-issuer",
            "CAPER_IOS_BUNDLE_ID": "chat.fixture.custom",
            "BUILD_NUMBER": "37.2",
        }

    def upload(self, fail_stage=""):
        result = subprocess.run(
            ["bash", str(self.app / "upload-testflight.sh")],
            env={**self.env, "FAIL_STAGE": fail_stage},
            capture_output=True, text=True, check=False,
        )
        commands = [json.loads(line) for line in self.log.read_text().splitlines()]
        return result, commands

    def test_archive_is_certificate_free_but_export_uses_cloud_distribution(self):
        result, commands = self.upload()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(commands), 2)
        archive, export = commands
        args = archive["args"]
        self.assertEqual(args[-1], "archive")
        self.assertEqual(args[args.index("-configuration") + 1], "Release")
        self.assertIn("CODE_SIGN_IDENTITY=-", args)
        self.assertIn("AD_HOC_CODE_SIGNING_ALLOWED=YES", args)
        self.assertIn("DEVELOPMENT_TEAM=FIXTURETEAM", args)
        self.assertIn("CAPER_IOS_BUNDLE_ID=chat.fixture.custom", args)
        self.assertIn("CURRENT_PROJECT_VERSION=37.2", args)
        self.assertNotIn("CODE_SIGNING_ALLOWED=NO", args)
        self.assertNotIn("CODE_SIGN_ENTITLEMENTS=", args)
        for flag in ("-allowProvisioningUpdates", "-authenticationKeyPath",
                     "-authenticationKeyID", "-authenticationKeyIssuerID"):
            self.assertNotIn(flag, args)
            self.assertIn(flag, export["args"])
        self.assertEqual(export["args"][export["args"].index("-authenticationKeyPath") + 1],
                         self.env["NOTARY_KEY_PATH"])
        self.assertFalse(any(arg.startswith(("CODE_SIGN_IDENTITY=", "AD_HOC_CODE_SIGNING_ALLOWED="))
                             for arg in export["args"]))
        self.assertEqual(export["options"], {
            "method": "app-store-connect", "destination": "upload", "teamID": "FIXTURETEAM",
            "signingStyle": "automatic", "manageAppVersionAndBuildNumber": False, "uploadSymbols": True,
        })
        self.assertIn("Uploaded chat.fixture.custom build 37.2", result.stdout)

    def test_archive_failure_stops_before_export_or_success_message(self):
        result, commands = self.upload("archive")
        self.assertEqual(result.returncode, 23)
        self.assertEqual(len(commands), 1)
        self.assertNotIn("Uploaded", result.stdout)

    def test_export_failure_does_not_report_an_upload(self):
        result, commands = self.upload("export")
        self.assertEqual(result.returncode, 23)
        self.assertEqual(len(commands), 2)
        self.assertNotIn("Uploaded", result.stdout)


if __name__ == "__main__":
    unittest.main()
