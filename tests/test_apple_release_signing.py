import json
import os
from pathlib import Path
import plistlib
import runpy
import shutil
import subprocess
import sys
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


class MacDiskImageTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.apple = self.root / "app files"
        self.apple.mkdir()
        source = UPLOAD.parent
        for name in ("sign-and-notarize.sh", "dmg-settings.py"):
            shutil.copyfile(source / name, self.apple / name)
        self.app = self.apple / "dist/Caper.app"
        (self.app / "Contents/MacOS").mkdir(parents=True)
        (self.app / "Contents/Frameworks").mkdir()
        with (self.app / "Contents/Info.plist").open("wb") as info:
            plistlib.dump({"CFBundleName": "Caper", "CFBundleExecutable": "Caper",
                          "CFBundleIdentifier": "chat.caper.fixture", "CFBundlePackageType": "APPL"}, info)
        (self.app / "Contents/MacOS/Caper").write_bytes(b"fixture-executable")
        self.updater = self.root / "fixture-updater"
        self.updater.write_bytes(b"updater")
        binary = self.root / "bin"
        binary.mkdir()
        for name in ("codesign", "ditto", "xcrun", "spctl", "dmgbuild"):
            tool = binary / name
            tool.write_text("""#!/usr/bin/env python3
import json
import os
from pathlib import Path
import sys
tool, args = Path(sys.argv[0]).name, sys.argv[1:]
with open(os.environ["TEST_LOG"], "a") as log:
    log.write(json.dumps([tool, *args]) + "\\n")
target = Path(args[-1])
app = Path(os.environ["TEST_APP"])
if tool == "codesign" and "-d" in args:
    print("<key>com.apple.security.device.audio-input</key><true/>")
elif tool == "ditto":
    target.write_bytes(b"stapled" if (app / "stapled-fixture").exists() else b"unstapled")
elif tool == "dmgbuild":
    assert (app / "stapled-fixture").exists(), "DMG must carry the stapled app"
    if os.environ.get("FAIL_STAGE") == "dmg-build":
        sys.exit(23)
    target.write_bytes(b"disk-image")
elif tool == "xcrun" and args[:2] == ["notarytool", "submit"]:
    kind = Path(args[2]).suffix[1:]
    status = "Invalid" if os.environ.get("FAIL_NOTARY") == kind else "Accepted"
    print(json.dumps({"status": status, "id": "fixture-id"}))
elif tool == "xcrun" and args[:2] == ["stapler", "staple"]:
    kind = "app" if target.suffix == ".app" else "dmg"
    if os.environ.get("FAIL_STAGE") == kind + "-staple":
        sys.exit(23)
    if kind == "app":
        (app / "stapled-fixture").touch()
elif tool == "spctl":
    if target.suffix == ".dmg" and os.environ.get("FAIL_STAGE") == "dmg-assess":
        sys.exit(23)
    print("source=Notarized Developer ID")
""")
            tool.chmod(0o755)
        self.log = self.root / "commands.jsonl"
        self.env = {
            **os.environ, "PATH": f"{binary}:{os.environ['PATH']}",
            "TEST_LOG": str(self.log), "TEST_APP": str(self.app),
            "APPLE_SIGNING_IDENTITY": "Developer ID Application: Fixture",
            "NOTARY_KEY_PATH": str(self.root / "fixture-key.p8"),
            "NOTARY_KEY_ID": "fixture-id", "NOTARY_ISSUER": "fixture-issuer",
            "CAPER_UPDATER_BINARY": str(self.updater),
        }

    def sign(self, **failure):
        result = subprocess.run(["bash", str(self.apple / "sign-and-notarize.sh"), "Intel"],
                                env={**self.env, **failure}, capture_output=True, text=True)
        commands = [json.loads(line) for line in self.log.read_text().splitlines()]
        return result, commands

    def test_notarizes_app_then_signed_dmg_and_preserves_update_zip(self):
        result, commands = self.sign()
        self.assertEqual(result.returncode, 0, result.stderr)
        submissions = [c for c in commands if c[:3] == ["xcrun", "notarytool", "submit"]]
        self.assertEqual([Path(c[3]).suffix for c in submissions], [".zip", ".dmg"])
        dmg = self.apple / "dist/Caper-macOS-Intel.dmg"
        self.assertEqual((self.apple / "dist/Caper-macOS-Intel.zip").read_bytes(), b"stapled")
        self.assertEqual(dmg.read_bytes(), b"disk-image")
        build = ["dmgbuild", "-s", str(self.apple / "dmg-settings.py"), "-D",
                 f"app={self.app}", "Caper", str(dmg)]
        self.assertIn(build, commands)
        sign = ["codesign", "--force", "--timestamp", "--sign",
                self.env["APPLE_SIGNING_IDENTITY"], str(dmg)]
        staple = ["xcrun", "stapler", "staple", str(dmg)]
        self.assertLess(commands.index(sign), commands.index(submissions[1]))
        self.assertLess(commands.index(submissions[1]), commands.index(staple))
        self.assertIn(["xcrun", "stapler", "validate", str(dmg)], commands)
        self.assertIn(["spctl", "--assess", "--type", "open", "--context",
                       "context:primary-signature", "--verbose=2", str(dmg)], commands)
        self.assertIn(str(dmg), result.stdout)

    def test_rejected_notarizations_stop_before_stapling_or_reporting_success(self):
        for kind in ("zip", "dmg"):
            with self.subTest(kind=kind):
                self.log.unlink(missing_ok=True)
                result, commands = self.sign(FAIL_NOTARY=kind)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("finished with status Invalid", result.stderr)
                self.assertNotIn(str(self.apple / "dist/Caper-macOS-Intel.dmg"), result.stdout)
                target = self.app if kind == "zip" else self.apple / "dist/Caper-macOS-Intel.dmg"
                self.assertNotIn(["xcrun", "stapler", "staple", str(target)], commands)

    def test_build_staple_and_assessment_failures_are_not_reported_as_success(self):
        for stage in ("dmg-build", "app-staple", "dmg-staple", "dmg-assess"):
            with self.subTest(stage=stage):
                self.log.unlink(missing_ok=True)
                result, _ = self.sign(FAIL_STAGE=stage)
                self.assertEqual(result.returncode, 23, result.stderr)
                self.assertNotIn(str(self.apple / "dist/Caper-macOS-Intel.dmg"), result.stdout)

    def test_finder_layout_has_both_icons_arrow_and_read_only_format(self):
        settings = runpy.run_path(str(self.apple / "dmg-settings.py"),
                                  init_globals={"defines": {"app": str(self.app)}})
        self.assertEqual(settings["files"], [str(self.app)])
        self.assertEqual(settings["symlinks"], {"Applications": "/Applications"})
        self.assertEqual(settings["icon_locations"], {"Caper.app": (140, 120), "Applications": (500, 120)})
        self.assertEqual(settings["background"], "builtin-arrow")
        self.assertEqual(settings["default_view"], "icon-view")
        self.assertEqual(settings["format"], "UDZO")

    @unittest.skipUnless(sys.platform == "darwin", "hdiutil requires macOS")
    def test_real_disk_image_contains_the_app_applications_link_and_finder_layout(self):
        # No signing material or real app is used in this packaging smoke check.
        dmg = self.root / "fixture.dmg"
        subprocess.run(["dmgbuild", "-s", str(self.apple / "dmg-settings.py"),
                        "-D", f"app={self.app}", "Caper", str(dmg)], check=True)
        result = subprocess.run(["hdiutil", "attach", "-readonly", "-nobrowse", "-plist", str(dmg)],
                                check=True, capture_output=True)
        entities = plistlib.loads(result.stdout)["system-entities"]
        mount = Path(next(entity["mount-point"] for entity in entities if "mount-point" in entity))
        try:
            self.assertEqual((mount / "Caper.app/Contents/MacOS/Caper").read_bytes(), b"fixture-executable")
            self.assertEqual(os.readlink(mount / "Applications"), "/Applications")
            self.assertTrue((mount / ".DS_Store").is_file())
        finally:
            subprocess.run(["hdiutil", "detach", str(mount)], check=True)


if __name__ == "__main__":
    unittest.main()
