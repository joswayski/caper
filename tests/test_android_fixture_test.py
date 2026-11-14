"""Exercise the diagnostic runner's exit/artifact contract with fake adb/Gradle.

These process tests do not validate Android behavior or device collection permissions.
"""

import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import unittest


RUNNER = Path(__file__).resolve().parents[1] / "scripts/android-fixture-test.py"
FAKE_ADB = """import os, sys, time
from pathlib import Path
if sys.argv[1] == 'logcat':
    Path('logcat.pid').write_text(str(os.getpid()))
    print('emulator boot evidence', flush=True)
    if 'COLLECTOR_EXIT' in os.environ:
        sys.exit(int(os.environ['COLLECTOR_EXIT']))
    if os.environ.get('EMIT_FAILURE') == '1':
        print('I TestRunner: fai', end='', flush=True)
        time.sleep(.2)
        print('led: clipboard(test)', flush=True)
    while True:
        time.sleep(.05)
        print('continuous evidence', flush=True)
print('snapshot ' + ' '.join(sys.argv[1:]))
sys.exit(int(os.environ.get('CAPTURE_EXIT', '0')))
"""
FAKE_GRADLE = """import json, os, sys, time
from pathlib import Path
Path('gradle-args.json').write_text(json.dumps(sys.argv[1:]))
if os.environ.get('EMIT_FAILURE') == '1':
    # Require the first snapshot while tests are still running, not only on exit.
    for _ in range(500):
        if Path('dist/ui/diagnostics/first-failure/system-anr.txt').exists():
            break
        time.sleep(.01)
    else:
        sys.exit(99)
else:
    time.sleep(.5)
sys.exit(int(os.environ.get('TEST_EXIT', '0')))
"""


class AndroidFixtureRunnerTests(unittest.TestCase):
    def test_preserves_suite_failure_and_rejects_lost_diagnostics(self):
        cases = [
            ({}, 0),
            ({"TEST_EXIT": "19", "EMIT_FAILURE": "1"}, 19),
            ({"TEST_EXIT": "19", "EMIT_FAILURE": "1", "CAPTURE_EXIT": "17"}, 19),
            ({"TEST_EXIT": "0", "EMIT_FAILURE": "1", "CAPTURE_EXIT": "17"}, 74),
            ({"COLLECTOR_EXIT": "0"}, 74),
            ({"COLLECTOR_EXIT": "42"}, 74),
            ({"TEST_EXIT": "19", "COLLECTOR_EXIT": "42"}, 19),
        ]
        for overrides, expected in cases:
            with self.subTest(overrides=overrides), tempfile.TemporaryDirectory() as directory:
                root = Path(directory) / "apps/native/android"
                root.mkdir(parents=True)
                script = Path(directory) / "scripts/android-fixture-test.py"
                script.parent.mkdir()
                shutil.copyfile(RUNNER, script)
                for name, body in [("adb", FAKE_ADB), ("gradlew", FAKE_GRADLE)]:
                    path = root / name
                    path.write_text(f"#!{sys.executable}\n{body}")
                    path.chmod(0o755)
                result = subprocess.run(
                    [sys.executable, str(script)],
                    cwd=root,
                    env={
                        **os.environ,
                        "PATH": f"{root}{os.pathsep}{os.environ['PATH']}",
                        "ANDROID_SERIAL": "emulator-5556",
                        **overrides,
                    },
                    capture_output=True,
                    text=True,
                    timeout=15,
                )
                self.assertEqual(result.returncode, expected, result.stderr)
                output = root / "dist/ui/diagnostics"
                status = json.loads((output / "status.json").read_text())
                self.assertEqual(status["tests"], int(overrides.get("TEST_EXIT", "0")))
                self.assertEqual(
                    status["logcatEarlyExit"],
                    int(overrides["COLLECTOR_EXIT"]) if "COLLECTOR_EXIT" in overrides else None,
                )
                if "COLLECTOR_EXIT" not in overrides:
                    self.assertEqual(status["logcatExit"], -signal.SIGTERM)
                self.assertIn("emulator boot evidence", (output / "logcat.txt").read_text())
                if overrides.get("EMIT_FAILURE"):
                    self.assertIn("first-failure", status["snapshots"])
                    self.assertIn("clipboard(test)", (output / "logcat.txt").read_text())
                if overrides.get("TEST_EXIT") == "19":
                    self.assertIn("suite-failure", status["snapshots"])
                for label, captures in status["snapshots"].items():
                    self.assertEqual(captures["system-anr.txt"], int(overrides.get("CAPTURE_EXIT", "0")))
                    self.assertIn("dumpsys window", (output / label / "window.txt").read_text())
                    self.assertIn("dumpsys activity lastanr", (output / label / "lastanr.txt").read_text())
                    self.assertTrue((output / label / "screen.png").is_file())
                arguments = json.loads((root / "gradle-args.json").read_text())
                self.assertIn("connectedDebugAndroidTest", arguments)
                self.assertFalse(any("notClass" in argument or "notAnnotation" in argument for argument in arguments))
                with self.assertRaises(ProcessLookupError):
                    os.kill(int((root / "logcat.pid").read_text()), 0)

    def test_refuses_to_collect_system_logs_from_a_personal_device(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            script = root / "scripts/android-fixture-test.py"
            script.parent.mkdir()
            shutil.copyfile(RUNNER, script)
            result = subprocess.run(
                [sys.executable, str(script)],
                env={**os.environ, "ANDROID_SERIAL": "personal-device"},
                capture_output=True,
                text=True,
                timeout=5,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Refusing system-log collection", result.stderr)
            self.assertFalse((root / "apps").exists())


if __name__ == "__main__":
    unittest.main()
