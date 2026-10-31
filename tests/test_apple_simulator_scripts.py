import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


APPLE = Path(__file__).resolve().parents[1] / "apps/native/apple"
DEFAULT_DESTINATION = "platform=iOS Simulator,name=iPhone 16,OS=latest"
CUSTOM_DESTINATION = "platform=iOS Simulator,name=Fixture Phone Duo,OS=27.1"


class AppleSimulatorScriptTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name) / "repo with spaces"
        self.apple = self.root / "apps/native/apple"
        self.apple.mkdir(parents=True)
        for name in ("build.sh", "parity-screenshots.sh"):
            shutil.copyfile(APPLE / name, self.apple / name)
        prepare = self.apple / "prepare.sh"
        prepare.write_text('#!/usr/bin/env bash\ntouch "$(dirname "$0")/prepared"\n')
        prepare.chmod(0o755)
        config = self.apple / ".build/SourceRevision.xcconfig"
        config.parent.mkdir()
        config.write_text(f"CAPER_SOURCE_REVISION = {'a' * 40}\n")
        binary = self.root / "bin"
        binary.mkdir()
        for name in ("node", "curl", "defaults"):
            tool = binary / name
            # No fixture server or network is needed for shell argument tests.
            tool.write_text("#!/usr/bin/env bash\nexit 0\n")
            tool.chmod(0o755)
        for name in ("xcodebuild", "xcrun"):
            tool = binary / name
            tool.write_text("""#!/usr/bin/env python3
import json
import os
from pathlib import Path
import sys

tool, args = Path(sys.argv[0]).name, sys.argv[1:]
assert (Path(os.environ["TEST_APP_ROOT"]) / "prepared").is_file()
with open(os.environ["TEST_LOG"], "a") as log:
    log.write(json.dumps([tool, *args]) + "\\n")
if tool == "xcodebuild":
    assert args[-1] == "test", "these tests must stop before release packaging"
    if "-resultBundlePath" in args and os.environ.get("TEST_SKIP_RESULT_BUNDLE") != "1":
        Path(args[args.index("-resultBundlePath") + 1]).mkdir()
    scheme = args[args.index("-scheme") + 1]
    sys.exit(0 if scheme == "CaperMacOS" else int(os.environ["TEST_XCODEBUILD_STATUS"]))
if tool == "xcrun":
    assert args[:3] == ["xcresulttool", "export", "attachments"]
    assert Path(args[args.index("--path") + 1]).is_dir()
    status = int(os.environ["TEST_EXPORT_STATUS"])
    if not status:
        output = Path(args[args.index("--output-path") + 1])
        output.mkdir()
        (output / "attachment.txt").write_text("fixture attachment")
    sys.exit(status)
""")
            tool.chmod(0o755)
        self.log = self.root / "commands.jsonl"
        self.env = {
            **os.environ,
            "PATH": f"{binary}:{os.environ['PATH']}",
            "TMPDIR": str(self.root),
            "TEST_APP_ROOT": str(self.apple),
            "TEST_LOG": str(self.log),
            "TEST_XCODEBUILD_STATUS": "0",
            "TEST_EXPORT_STATUS": "0",
            "TEST_SKIP_RESULT_BUNDLE": "0",
            "CAPER_MACOS_ARCH": "arm64",
        }
        self.env.pop("CAPER_IOS_SIMULATOR_DESTINATION", None)

    def run_script(self, script, *arguments, **overrides):
        self.log.unlink(missing_ok=True)
        result = subprocess.run(
            ["bash", str(self.apple / script), *arguments],
            cwd=self.root,
            env={**self.env, **overrides},
            capture_output=True,
            text=True,
            timeout=10,
            check=False,
        )
        commands = [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []
        return result, commands

    def test_build_ios_uses_default_or_custom_simulator_and_preserves_test_failure(self):
        for destination in (None, "", CUSTOM_DESTINATION):
            with self.subTest(destination=destination):
                overrides = {} if destination is None else {"CAPER_IOS_SIMULATOR_DESTINATION": destination}
                # Stop at the iOS test failure: release packaging is unchanged
                # and does not need a fake app bundle or Apple toolchain here.
                result, commands = self.run_script("build.sh", "ios", TEST_XCODEBUILD_STATUS="23", **overrides)
                self.assertEqual(result.returncode, 23, result.stderr)
                self.assertEqual(len(commands), 2)
                mac, ios = commands
                self.assertEqual(mac[mac.index("-scheme") + 1], "CaperMacOS")
                self.assertEqual(mac[mac.index("-destination") + 1], "platform=macOS,arch=arm64")
                self.assertEqual(ios[ios.index("-scheme") + 1], "CaperIOS")
                self.assertEqual(ios[ios.index("-destination") + 1], destination or DEFAULT_DESTINATION)
                self.assertNotIn("simulator only", result.stdout)

    def test_parity_ios_uses_default_or_custom_simulator_and_exports_attachments(self):
        for destination in (None, "", CUSTOM_DESTINATION):
            with self.subTest(destination=destination):
                overrides = {} if destination is None else {"CAPER_IOS_SIMULATOR_DESTINATION": destination}
                result, commands = self.run_script("parity-screenshots.sh", "ios", **overrides)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(len(commands), 2)
                args = commands[0]
                self.assertEqual(args[args.index("-scheme") + 1], "CaperIOSParityTests")
                self.assertEqual(args[args.index("-destination") + 1], destination or DEFAULT_DESTINATION)
                self.assertEqual(
                    commands[1],
                    [
                        "xcrun",
                        "xcresulttool",
                        "export",
                        "attachments",
                        "--path",
                        str(self.apple / "ParityResults.xcresult"),
                        "--output-path",
                        str(self.apple / "parity-artifacts"),
                    ],
                )
                self.assertEqual((self.apple / "parity-artifacts/attachment.txt").read_text(), "fixture attachment")
                self.assertIn(str(self.apple / "parity-artifacts"), result.stdout)

    def test_parity_forwards_extra_arguments_without_splitting_or_expansion_on_both_platforms(self):
        extra = [
            "-only-testing:FixtureParityTests/ResizeTests/testResize",
            "-testLanguage",
            "en GB",
            'FIXTURE_SETTING=spaces "quotes" * $HOME ; literal',
        ]
        for platform, scheme, destination in (
            ("ios", "CaperIOSParityTests", CUSTOM_DESTINATION),
            ("macos", "CaperMacOSParityTests", "platform=macOS,arch=arm64"),
        ):
            with self.subTest(platform=platform):
                result, commands = self.run_script(
                    "parity-screenshots.sh", platform, *extra, CAPER_IOS_SIMULATOR_DESTINATION=CUSTOM_DESTINATION
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(len(commands), 2)
                args = commands[0]
                self.assertEqual(args[args.index("-scheme") + 1], scheme)
                self.assertEqual(args[args.index("-destination") + 1], destination)
                self.assertEqual(args[-len(extra) - 1 :], [*extra, "test"])
                self.assertNotIn(platform, args)

    def test_parity_preserves_xcodebuild_status_even_if_attachment_export_fails(self):
        for test_status, export_status in ((65, 0), (65, 19), (0, 19)):
            with self.subTest(test_status=test_status, export_status=export_status):
                result, commands = self.run_script(
                    "parity-screenshots.sh",
                    "ios",
                    "-only-testing:FixtureParityTests/ResizeTests/testResize",
                    TEST_XCODEBUILD_STATUS=str(test_status),
                    TEST_EXPORT_STATUS=str(export_status),
                )
                self.assertEqual(result.returncode, test_status, result.stderr)
                self.assertEqual([command[0] for command in commands], ["xcodebuild", "xcrun"])
                if export_status:
                    self.assertIn("warning: could not export parity attachments", result.stderr)
                else:
                    self.assertTrue((self.apple / "parity-artifacts/attachment.txt").is_file())

    def test_parity_missing_result_bundle_preserves_test_failure_and_skips_export(self):
        result, commands = self.run_script(
            "parity-screenshots.sh", "ios", TEST_XCODEBUILD_STATUS="66", TEST_SKIP_RESULT_BUNDLE="1"
        )
        self.assertEqual(result.returncode, 66, result.stderr)
        self.assertEqual([command[0] for command in commands], ["xcodebuild"])
        self.assertIn("warning: xcodebuild did not produce ParityResults.xcresult", result.stderr)
        self.assertFalse((self.apple / "parity-artifacts").exists())

    def test_invalid_or_missing_platform_stops_before_side_effects(self):
        sentinels = [self.apple / name / "keep.txt" for name in ("ParityResults.xcresult", "parity-artifacts")]
        for sentinel in sentinels:
            sentinel.parent.mkdir()
            sentinel.write_text("keep")
        for script in ("build.sh", "parity-screenshots.sh"):
            for arguments in ((), ("android",), ("-only-testing:FixtureParityTests",)):
                with self.subTest(script=script, arguments=arguments):
                    result, commands = self.run_script(script, *arguments)
                    self.assertEqual(result.returncode, 2, result.stderr)
                    self.assertIn("macos|ios", result.stderr)
                    self.assertEqual(commands, [])
                    self.assertFalse((self.apple / "prepared").exists())
                    self.assertFalse((self.apple / "dist").exists())
                    self.assertFalse((self.root / "caper-native-parity-fixture.log").exists())
                    for sentinel in sentinels:
                        self.assertEqual(sentinel.read_text(), "keep")


if __name__ == "__main__":
    unittest.main()
