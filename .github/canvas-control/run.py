"""Never-merge diagnostic supervisor. No Caper build or test entry points."""

import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import threading
import time


MODES = ("hosting", "hosting-canvas", "hosting-blur")
ROOT = Path(__file__).resolve().parent
FIXTURE_SHA256 = "58c83c884b281ff038bd7e55de529d8159829796738827a7033e7593475061a3"


def record(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def command(argv, directory, timeout, capture=False):
    """Save direct subprocess return codes (negative signals), even on failure."""
    directory.mkdir(parents=True)
    started = time.monotonic()
    result = {"argv": argv, "timeoutSeconds": timeout, "timedOut": False}
    screenshot = {"status": "not_attempted", "reason": "screen-capture preflight not granted"}
    worker = None
    with (directory / "stdout.txt").open("wb") as stdout, (directory / "stderr.txt").open("wb") as stderr:
        try:
            process = subprocess.Popen(argv, stdout=stdout, stderr=stderr, start_new_session=True)
        except OSError as error:
            result.update(
                returnCode=None, signal=None, launchError=str(error), elapsedSeconds=time.monotonic() - started
            )
        else:
            if capture:

                def take_screenshot():
                    time.sleep(0.2)
                    if process.poll() is not None:
                        screenshot.update(status="not_attempted", reason="fixture exited before capture attempt")
                        return
                    screenshot.clear()
                    screenshot.update(
                        status="attempted",
                        fixtureAliveAtStart=True,
                        startOffsetSeconds=time.monotonic() - started,
                        meaning="Desktop capture only; inspect pixels before claiming the fixture window rendered.",
                    )
                    shot = command(
                        ["/usr/sbin/screencapture", "-x", str(directory / "desktop.png")],
                        directory / "capture-command",
                        5,
                    )
                    screenshot.update(
                        endOffsetSeconds=time.monotonic() - started,
                        returnCode=shot["returnCode"],
                        timedOut=shot["timedOut"],
                        filePresent=(directory / "desktop.png").is_file(),
                    )
                    if shot["returnCode"] != 0 or shot["timedOut"] or not screenshot["filePresent"]:
                        screenshot.update(status="missing", reason="capture failed; fixture lifetime was not extended")
                    else:
                        screenshot.update(status="captured_uninspected")

                worker = threading.Thread(target=take_screenshot)
                worker.start()
            try:
                process.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                result["timedOut"] = True
                # Bound the entire short-lived subprocess group, not just its parent.
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait()
            result.update(
                returnCode=process.returncode,
                signal=-process.returncode if process.returncode < 0 else None,
                elapsedSeconds=time.monotonic() - started,
            )
    if worker is not None:
        worker.join()
    if capture or argv[0].endswith("graphics-control"):
        result["capture"] = screenshot
    record(directory / "result.json", result)
    return result


def ok(result):
    return (
        result["returnCode"] == 0
        and not result["timedOut"]
        and result.get("capture", {}).get("status", "not_attempted") in {"not_attempted", "captured_uninspected"}
    )


def main():
    output = Path(sys.argv[1]).resolve()
    output.mkdir(parents=True, exist_ok=False)
    record(
        output / "runner-image.json",
        {
            key: os.environ.get(key)
            for key in (
                "ImageOS",
                "ImageVersion",
                "RUNNER_OS",
                "RUNNER_ARCH",
                "RUNNER_ENVIRONMENT",
                "GITHUB_SHA",
                "GITHUB_REF",
                "GITHUB_RUN_ID",
                "GITHUB_RUN_ATTEMPT",
                "DEVELOPER_DIR",
            )
        },
    )
    record(
        output / "source-hashes.json",
        {
            str(path.relative_to(ROOT.parent.parent)): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in sorted(ROOT.iterdir())
            if path.is_file()
        },
    )
    workflow = ROOT.parent / "workflows/apple-canvas-control.yml"
    shutil.copytree(ROOT, output / "source", ignore=shutil.ignore_patterns("__pycache__"))
    shutil.copy2(workflow, output / "source" / workflow.name)
    (output / "workflow-sha256.txt").write_text(hashlib.sha256(workflow.read_bytes()).hexdigest() + "\n")
    checks = []
    for name, argv, timeout in (
        ("git", ["git", "rev-parse", "HEAD", "HEAD^{tree}"], 10),
        ("clean", ["git", "diff", "--exit-code"], 10),
        ("os", ["sw_vers"], 10),
        ("architecture", ["uname", "-m"], 10),
        ("xcode", ["xcodebuild", "-version"], 30),
        ("swift", ["xcrun", "swiftc", "--version"], 30),
        ("display", ["system_profiler", "SPDisplaysDataType", "-json"], 30),
    ):
        checks.append(command(argv, output / name, timeout))
    # Fail closed rather than silently comparing a different image/toolchain.
    expected_environment = (
        os.environ.get("RUNNER_ENVIRONMENT") == "github-hosted"
        and os.environ.get("GITHUB_RUN_ATTEMPT") == "1"
        and os.environ.get("GITHUB_REF") == "refs/heads/apple-combined-macos26-run"
        and (output / "architecture/stdout.txt").read_text().strip() == "x86_64"
        and "26.6.1" in (output / "os/stdout.txt").read_text()
        and "25G76" in (output / "os/stdout.txt").read_text()
        and os.environ.get("ImageVersion") == "20260824.0517.1"
        and (output / "xcode/stdout.txt").read_text().strip() == "Xcode 26.6\nBuild version 17F113"
        and hashlib.sha256((ROOT / "IntelGraphicsProbe.swift").read_bytes()).hexdigest() == FIXTURE_SHA256
    )
    record(output / "preconditions.json", {"matched": expected_environment, "commandsPassed": all(map(ok, checks))})
    if not expected_environment or not all(map(ok, checks)):
        record(output / "summary.json", {"failed": True, "reason": "provenance/preconditions failed", "modes": []})
        return 1

    binary = output / "graphics-control"
    compile_fixture = command(
        ["xcrun", "swiftc", "-parse-as-library", str(ROOT / "IntelGraphicsProbe.swift"), "-o", str(binary)],
        output / "compile-fixture",
        180,
    )
    metal = output / "metal-provenance"
    compile_metal = command(
        ["xcrun", "swiftc", str(ROOT / "MetalProvenance.swift"), "-o", str(metal)],
        output / "compile-metal",
        180,
    )
    checks.extend([compile_fixture, compile_metal])
    capture = False
    if ok(compile_metal):
        provenance = command([str(metal)], output / "metal", 30)
        checks.append(provenance)
        if ok(provenance):
            try:
                capture = json.loads((output / "metal/stdout.txt").read_text())["screenCapturePreflight"] is True
            except (ValueError, KeyError):
                record(output / "metal-parse-error.json", {"failed": True})
                checks.append({"returnCode": 1, "timedOut": False})
    results = []
    if ok(compile_fixture):
        (output / "binary-sha256.txt").write_text(hashlib.sha256(binary.read_bytes()).hexdigest() + "\n")
        for mode in MODES:
            result = command([str(binary), mode, str(output / "unused.png")], output / mode, 60, capture=capture)
            result["mode"] = mode
            result["survivalMarker"] = "hosting survived" in (output / mode / "stdout.txt").read_text()
            results.append(result)
    failed = (
        not all(map(ok, checks))
        or len(results) != 3
        or any(not ok(result) or not result["survivalMarker"] for result in results)
    )
    record(output / "summary.json", {"failed": failed, "expectedModes": list(MODES), "modes": results})
    print(json.dumps({"failed": failed, "executed": len(results)}))
    return int(failed)


if __name__ == "__main__":
    sys.exit(main())
