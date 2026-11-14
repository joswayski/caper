"""Run the complete fixture UI suite while preserving emulator focus/ANR evidence.

Used inside android-emulator-runner, before it tears down its disposable emulator.
Do not run against a personal device: system logs can include other applications.
"""

from __future__ import annotations

import json
import os
from pathlib import Path
import signal
import subprocess
import time


ROOT = Path(__file__).resolve().parents[1] / "apps/native/android"
OUTPUT = ROOT / "dist/ui/diagnostics"


def snapshot(label: str) -> dict[str, int | str]:
    directory = OUTPUT / label
    directory.mkdir()
    (directory / "started.txt").write_text(time.strftime("%Y-%m-%dT%H:%M:%SZ\n", time.gmtime()))
    results = {}
    commands = {
        "window.txt": ["shell", "dumpsys", "window"],
        "activity.txt": ["shell", "dumpsys", "activity", "activities"],
        "power.txt": ["shell", "dumpsys", "power"],
        "screen.png": ["exec-out", "screencap", "-p"],
        "lastanr.txt": ["shell", "dumpsys", "activity", "lastanr"],
        "system-anr.txt": ["shell", "dumpsys", "dropbox", "--print", "system_app_anr"],
    }
    for name, command in commands.items():
        with (directory / name).open("wb") as output, (directory / f"{name}.stderr").open("wb") as errors:
            try:
                results[name] = subprocess.run(
                    ["adb", *command], stdout=output, stderr=errors, timeout=20, check=False
                ).returncode
            except (OSError, subprocess.TimeoutExpired) as error:
                results[name] = str(error)
    return results


def stop(process: subprocess.Popen) -> None:
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()


def main() -> int:
    # android-emulator-runner supplies this, including when its port is non-default.
    if not os.environ.get("ANDROID_SERIAL", "").startswith("emulator-"):
        raise SystemExit("Refusing system-log collection without an explicit emulator ANDROID_SERIAL.")
    OUTPUT.mkdir(parents=True, exist_ok=False)
    with (OUTPUT / "logcat.txt").open("wb") as output, (OUTPUT / "logcat.stderr").open("wb") as errors:
        logcat = subprocess.Popen(
            ["adb", "logcat", "-b", "all", "-v", "threadtime"],
            stdout=output,
            stderr=errors,
            start_new_session=True,
        )
        try:
            test = subprocess.Popen(
                [
                    str(ROOT / "gradlew"),
                    "--no-daemon",
                    "--max-workers=1",
                    "-PcaperFixtureMode=true",
                    "-PcaperApiBaseUrl=http://localhost:3001",
                    "connectedDebugAndroidTest",
                ],
                cwd=ROOT,
                env={**os.environ, "CMAKE_BUILD_PARALLEL_LEVEL": "1"},
                start_new_session=True,
            )
            captures = {}
            try:
                with (OUTPUT / "logcat.txt").open(errors="replace") as stream:
                    pending = ""
                    while test.poll() is None:
                        pending += stream.read()
                        if not captures and "TestRunner: failed:" in pending:
                            captures["first-failure"] = snapshot("first-failure")
                        # Keep a partial line for the next read, not the entire stream.
                        pending = pending.rsplit("\n", 1)[-1]
                        time.sleep(0.1)
                code = test.wait()
                if code != 0:
                    captures["suite-failure"] = snapshot("suite-failure")
            finally:
                stop(test)
            # An unexpected EOF (even exit 0) means the diagnostic stream was lost.
            early_exit = logcat.poll()
        finally:
            stop(logcat)
    status = {"tests": code, "logcatEarlyExit": early_exit, "logcatExit": logcat.returncode, "snapshots": captures}
    (OUTPUT / "status.json").write_text(json.dumps(status, indent=2) + "\n")
    print(json.dumps(status), flush=True)
    diagnostic_failed = early_exit is not None or logcat.returncode not in (0, -signal.SIGTERM)
    diagnostic_failed |= any(result != 0 for capture in captures.values() for result in capture.values())
    return (128 - code if code < 0 else code) or (74 if diagnostic_failed else 0)


if __name__ == "__main__":

    def cancelled(signum, _frame):
        raise SystemExit(128 + signum)

    signal.signal(signal.SIGTERM, cancelled)
    signal.signal(signal.SIGINT, cancelled)
    raise SystemExit(main())
