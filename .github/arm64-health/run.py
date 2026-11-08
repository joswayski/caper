"""NEVER MERGE. Same-VM health controls gate the pinned 37+6 AppKit selection."""

from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import math
import os
from pathlib import Path
import plistlib
import re
import shutil
import signal
import stat
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
import xml.etree.ElementTree as ET

import forward_observation

ROOT = Path(__file__).resolve().parent
SOURCE = "9fca8b93f3629390e837794b9468c5cdc18d7617"
TREE = "1fdd75c82099d48f375e275ed105c93cfcd6c316"
GENERATOR = "21ac9944b0ab546a07422dbed86f33dd2ebd76f8"
SIDEBAR = "CaperAppKitViewTests/NavigationSidebarTests/testVoiceCardPreservesFocusAndLiveRowWhenChannelsCollapse"
AX = "AXMenuTests/AXMenuTests/testInitialTextFieldAndFullHierarchy"
TEST_FILE = Path("apps/native/apple/Tests/CaperAppKitViewTests/NavigationSidebarTests.swift")
QUERY_HEALTH_SECONDS = 15
MODES = ("hosting", "hosting-canvas", "hosting-blur")
FLAGS = [
    "-parallel-testing-enabled",
    "NO",
    "-test-timeouts-enabled",
    "YES",
    "-default-test-execution-time-allowance",
    "300",
    "-maximum-test-execution-time-allowance",
    "600",
    "-collect-test-diagnostics",
    "on-failure",
]


def record(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def ok(result):
    return result["returnCode"] == 0 and not result["timedOut"]


def command(argv, output, timeout, cwd=None, observe=False, capture=None):
    output.mkdir(parents=True, exist_ok=False)
    started = time.monotonic()
    result = {"argv": list(map(str, argv)), "cwd": str(cwd or Path.cwd()), "timeoutSeconds": timeout, "timedOut": False}
    worker = None
    with (output / "stdout.txt").open("wb") as stdout, (output / "stderr.txt").open("wb") as stderr:
        try:
            process = subprocess.Popen(argv, cwd=cwd, stdout=stdout, stderr=stderr, start_new_session=True)
        except OSError as error:
            result.update(returnCode=None, signal=None, launchError=str(error))
        else:
            if observe:
                worker = threading.Thread(target=sample_query, args=(process, output))
                worker.start()
            elif capture:
                worker = threading.Thread(target=capture_window, args=(process, output, *capture))
                worker.start()
            try:
                process.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                result["timedOut"] = True
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait()
            result.update(returnCode=process.returncode, signal=-process.returncode if process.returncode < 0 else None)
            result["processElapsedSeconds"] = time.monotonic() - started
    if worker is not None:
        worker.join()
    result["elapsedSeconds"] = time.monotonic() - started
    record(output / "result.json", result)
    return result


def checked(argv, output, timeout, cwd=None):
    result = command(argv, output, timeout, cwd)
    if not ok(result):
        raise RuntimeError(f"{output.name} failed; see retained result/stdout/stderr")
    return result


def sample_query(process, output):
    """One sample pair only AFTER the health deadline; never perturb a healthy query."""
    report = {"failed": False, "status": "not_attempted", "reason": "query marker not observed before process exit"}
    observed = None
    try:
        while process.poll() is None:
            text = (output / "stdout.txt").read_text(errors="replace")
            match = re.search(r"AX_CONTROL_QUERY_BEGIN pid=(\d+)", text)
            if match:
                if "AX_CONTROL_QUERY_END" in text:
                    report["reason"] = "query completed before sampling; health uses its reported elapsed time"
                    break
                if observed is None:
                    observed = time.monotonic()
                if time.monotonic() - observed < QUERY_HEALTH_SECONDS:
                    time.sleep(0.2)
                    continue
                pid = int(match[1])
                report.update(
                    status="attempted",
                    reason="query still in flight after health deadline",
                    pid=pid,
                    observedAtUTC=time.time(),
                )
                with ThreadPoolExecutor(max_workers=2) as pool:
                    calls = [
                        pool.submit(
                            command,
                            ["/usr/bin/sample", str(pid), "5", "10", "-file", str(output / "app.sample.txt")],
                            output / "sample-app",
                            15,
                        ),
                        pool.submit(
                            command,
                            [
                                "sudo",
                                "-n",
                                "/usr/bin/sample",
                                "1",
                                "5",
                                "10",
                                "-file",
                                str(output / "launchd.sample.txt"),
                            ],
                            output / "sample-launchd",
                            15,
                        ),
                    ]
                    results = [call.result() for call in calls]
                report["failed"] = not all(map(ok, results)) or not all(
                    (output / name).is_file() and (output / name).stat().st_size > 0
                    for name in ["app.sample.txt", "launchd.sample.txt"]
                )
                break
            time.sleep(0.2)
    except Exception as error:
        report.update(failed=True, error=str(error))
    record(output / "sampling.json", report)


def capture_window(process, output, pixels, mode):
    """One PID-matched window capture; unchanged one-second fixture, no retries."""
    report = {"failed": True, "status": "not_attempted"}
    started = time.monotonic()
    try:
        if process.poll() is not None:
            raise RuntimeError("fixture exited before capture; lifetime not extended")
        report.update(status="attempted", pid=process.pid, fixtureAliveAtStart=True)
        discovery = command(
            [str(pixels), "window", str(process.pid), str(output / "window-observations.json")],
            output / "window-info",
            2,
        )
        report["discovery"] = discovery
        text = (output / "window-info/stdout.txt").read_text()
        if text.strip():
            report["window"] = json.loads(text)
        if not ok(discovery):
            raise RuntimeError("window readiness failed; see retained result/observations/stdout/stderr")
        window = report["window"]
        if window.get("passed") is not True:
            raise RuntimeError("window readiness did not pass; no capture attempted")
        if process.poll() is not None:
            raise RuntimeError("fixture exited after discovery; no capture attempted")
        report["fixtureAliveBeforeCapture"] = True
        checked(
            ["/usr/sbin/screencapture", "-x", "-o", "-l" + str(window["windowID"]), str(output / "window.png")],
            output / "capture-command",
            5,
        )
        checked([str(pixels), mode, str(output / "window.png")], output / "pixel-check", 60)
        report.update(
            failed=False,
            status="captured_content_checked",
            meaning="Automated content checks only; human inspection still required",
        )
    except Exception as error:
        report["error"] = f"{type(error).__name__}: {error}"
    report["elapsedSeconds"] = time.monotonic() - started
    record(output / "capture.json", report)


def graphics(output):
    binaries = {}
    for filename, name in [
        ("IntelGraphicsProbe.swift", "graphics-control"),
        ("MetalProvenance.swift", "metal-provenance"),
    ]:
        binary = output / name
        flags = ["-parse-as-library"] if name == "graphics-control" else []
        checked(["xcrun", "swiftc", *flags, str(ROOT / filename), "-o", str(binary)], output / ("compile-" + name), 180)
        binaries[name] = binary
    record(output / "binary-hashes.json", {name: sha(path) for name, path in binaries.items()})
    checked([str(binaries["metal-provenance"])], output / "metal", 30)
    permission = json.loads((output / "metal/stdout.txt").read_text())["screenCapturePreflight"] is True
    results = []
    for mode in MODES:
        result = command(
            [str(binaries["graphics-control"]), mode, str(output / "unused.png")],
            output / mode,
            60,
            capture=(output.parent / "preflight/pixels", mode) if permission else None,
        )
        if not permission:
            record(
                output / mode / "capture.json",
                {
                    "failed": True,
                    "status": "not_attempted",
                    "reason": "read-only permission preflight false; no prompt or grant",
                },
            )
        if not (output / mode / "capture.json").exists():
            record(
                output / mode / "capture.json",
                {
                    "failed": True,
                    "status": "not_attempted",
                    "reason": "fixture launch failed or capture worker produced no report",
                },
            )
        log = (output / mode / "stdout.txt").read_text()
        result.update(
            mode=mode,
            survivalMarker=log.count("hosting survived") == 1,
            startMarker=log.count("begin: " + mode + "\n") == 1,
            capture=json.loads((output / mode / "capture.json").read_text()),
        )
        results.append(result)
    passed = len(results) == 3 and all(
        ok(r) and r["survivalMarker"] and r["startMarker"] and not r["capture"]["failed"] for r in results
    )
    record(output / "modes.json", {"passed": passed, "modes": results})
    return passed


def preflight(output, provenance_only=False):
    output.mkdir(parents=True, exist_ok=False)
    shutil.copytree(ROOT, output / "source", ignore=shutil.ignore_patterns("__pycache__"))
    workflow = ROOT.parent / "workflows/apple-arm64-health-sidebar.yml"
    shutil.copy2(workflow, output / "source" / workflow.name)
    record(output / "input-hashes.json", {p.name: sha(p) for p in (output / "source").iterdir() if p.is_file()})
    environment = {
        name: os.environ.get(name)
        for name in [
            "ImageVersion",
            "RUNNER_ENVIRONMENT",
            "GITHUB_REF",
            "GITHUB_SHA",
            "GITHUB_RUN_ID",
            "GITHUB_RUN_ATTEMPT",
            "DEVELOPER_DIR",
        ]
    }
    record(output / "environment.json", environment)
    for name, argv, timeout in [
        ("os", ["sw_vers"], 10),
        ("architecture", ["uname", "-m"], 10),
        ("xcode", ["xcodebuild", "-version"], 30),
        ("display", ["system_profiler", "SPDisplaysDataType", "-json"], 30),
        ("harness-pin", ["git", "rev-parse", "HEAD", "HEAD^{tree}"], 10),
        ("clean", ["git", "diff", "--exit-code"], 10),
    ]:
        checked(argv, output / name, timeout)
    assert environment["RUNNER_ENVIRONMENT"] == "github-hosted"
    assert environment["GITHUB_RUN_ATTEMPT"] == "1"
    assert environment["GITHUB_REF"] == "refs/heads/apple-arm64-health-sidebar-run"
    assert environment["ImageVersion"] == "20261102.0473.1"
    assert environment["DEVELOPER_DIR"] == "/Applications/Xcode_26.6.app/Contents/Developer"
    assert (output / "harness-pin/stdout.txt").read_text().splitlines()[0] == environment["GITHUB_SHA"]
    assert (output / "architecture/stdout.txt").read_text().strip() == "arm64"
    assert re.search(r"ProductVersion:\s+26\.7\.2\s", (output / "os/stdout.txt").read_text())
    assert re.search(r"BuildVersion:\s+25G332\s", (output / "os/stdout.txt").read_text())
    assert (output / "xcode/stdout.txt").read_text().strip() == "Xcode 26.6\nBuild version 17F113"
    if provenance_only:
        return  # Observation mode must not create the controls' admission gate.
    checked(
        [sys.executable, str(ROOT / "crashes.py"), "baseline", str(output / "crash-baseline")],
        output / "inventory-baseline",
        30,
    )
    checked(
        ["xcrun", "swiftc", "-parse-as-library", str(ROOT / "Pixels.swift"), "-o", str(output / "pixels")],
        output / "compile-pixels",
        180,
    )
    record(output / "pixel-binary-hash.json", {"sha256": sha(output / "pixels")})
    record(output / "gate.json", {"passed": True, "productCommit": SOURCE, "productTree": TREE})


def display_inventory(output):
    geometry = command(["xcrun", "swift", str(ROOT / "DisplayInventory.swift")], output / "geometry", 60)
    # Inspect receipt metadata and the known guest utility; never execute it,
    # even with guessed --help/--version switches or an empty argument list.
    package = "com.veertu.anka.guestaddons.pkg"
    receipt = command(["/usr/sbin/pkgutil", "--pkg-info-plist", package], output / "guest-receipt", 10)
    report = {"packageID": package, "receiptProcess": receipt, "complete": False, "utilityExecuted": False}
    record(output / "guest.json", report)
    try:
        if ok(receipt):
            metadata = plistlib.loads((output / "guest-receipt/stdout.txt").read_bytes())
            report["receiptVersion"] = metadata.get("pkg-version")
            report["receiptPackageID"] = metadata.get("pkgid")
        else:
            report["receiptVersion"] = None  # Nonzero does not establish an installed version.
        directory = Path("/Library/Application Support/Veertu/Anka/addons")
        report["directory"] = str(directory)
        entries = []
        report["entries"] = entries  # Preserve partial inventory on bounds/permission failure.
        try:
            if directory.is_symlink():
                raise RuntimeError("Addons directory is a symlink; not followed")
            with os.scandir(directory) as scan:
                for index, entry in enumerate(scan):
                    if index == 128:
                        raise RuntimeError("Addons directory exceeds 128-entry inventory bound")
                    entries.append(entry.name)
            report["directoryState"] = "present"
        except FileNotFoundError:
            report["directoryState"] = "absent"
        report["entries"] = sorted(entries)
        utility = directory / "change_res"
        report["utility"] = {"path": str(utility), "present": False}
        try:
            info = utility.lstat()
        except FileNotFoundError:
            pass
        else:
            report["utility"].update(present=True, mode=oct(info.st_mode), size=info.st_size)
            if stat.S_ISLNK(info.st_mode):
                report["utility"]["symlinkTarget"] = os.readlink(utility)
            elif stat.S_ISREG(info.st_mode) and info.st_size <= 64 * 1024 * 1024:
                report["utility"]["sha256"] = sha(utility)
                description = command(["/usr/bin/file", "-b", str(utility)], output / "guest-utility-file", 10)
                report["utility"]["fileProcess"] = description
                assert ok(description), "file metadata command failed"
            else:
                raise RuntimeError("Utility is not a bounded regular file; not read")
        report["complete"] = True
    except Exception as error:
        report["error"] = f"{type(error).__name__}: {error}"
    record(output / "guest.json", report)
    # A complete observation is not geometry acceptance. Missing receipt version
    # remains explicitly unknown; no guest utility is presumed available or safe.
    return ok(geometry) and report["complete"] and ok(receipt)


def display_snapshot(binary, output, timeout=5):
    checked([str(binary)], output, timeout)
    data = json.loads((output / "stdout.txt").read_text())
    assert data["complete"] is True and data["errors"] == [], "Incomplete display observation"
    assert data["onlineCountBefore"] == data["onlineCountReturned"] == data["onlineCountAfter"] == 1
    assert len(data["screens"]) == len(data["displays"]) == 1, "Only one display is supported"
    screen, display = data["screens"][0], data["displays"][0]
    assert screen["isNSScreenMain"] is True and display["isCGMain"] is True
    assert screen["displayID"] == display["displayID"]
    assert display["active"] is True and display["online"] is True and display["asleep"] is False
    assert display["mirrorsDisplayID"] == 0
    return data


def check_session_display(output, setup, restore=False, attempts=1):
    state = json.loads((setup / "original-session.json").read_text())
    assert all(os.environ.get(key) == value for key, value in state["identity"].items()), "Different hosted run"
    original = state["original"]
    expected = original["displays"][0]["currentMode"] if restore else state["selected"]
    binary = setup / "display-inventory"
    hashes = json.loads((setup / "display-binary-hashes.json").read_text())
    assert sha(binary) == hashes[binary.name]
    deadline = time.monotonic() + 10
    observations = []
    try:
        for index in range(attempts):
            remaining = deadline - time.monotonic()
            assert remaining > 0, "Display settling observation budget exhausted"
            data = display_snapshot(binary, output / f"sample-{index + 1}", min(2, remaining))
            screen, display = data["screens"][0], data["displays"][0]
            frame, visible = screen["framePoints"], screen["visibleFramePoints"]
            matches = (
                display["displayID"] == original["displays"][0]["displayID"] and display["currentMode"] == expected
            )
            if restore:
                matches = matches and frame == original["screens"][0]["framePoints"]
                matches = matches and screen["backingScaleFactor"] == original["screens"][0]["backingScaleFactor"]
            else:
                matches = (
                    matches and screen["backingScaleFactor"] == 1 and frame["width"] == 1600 and frame["height"] == 1200
                )
                matches = matches and visible["width"] >= 1440 and visible["height"] >= 900
            observations.append({"sample": index + 1, "matches": matches})
            if matches:
                return
            if index + 1 < attempts:
                time.sleep(min(0.2, max(0, deadline - time.monotonic())))
        raise RuntimeError("Fresh display mode/usable geometry did not match; no repeated switch")
    finally:
        record(output / "observations.json", {"restore": restore, "observations": observations})


def session_display(output, restore=False):
    setup = output.parent / "display-setup" if restore else output
    state = setup / "original-session.json"
    if restore and not state.exists():
        record(
            output / "restoration.json", {"status": "not_needed", "reason": "No saved state; setup could not mutate"}
        )
        return True
    if not restore:
        for source, name in [
            ("DisplayInventory.swift", "display-inventory"),
            ("SessionDisplay.swift", "session-display"),
        ]:
            checked(["xcrun", "swiftc", str(ROOT / source), "-o", str(setup / name)], output / ("compile-" + name), 60)
        record(
            output / "display-binary-hashes.json",
            {name: sha(setup / name) for name in ["display-inventory", "session-display"]},
        )
    else:
        saved = json.loads(state.read_text())
        assert all(os.environ.get(key) == value for key, value in saved["identity"].items()), "Different hosted run"
        hashes = json.loads((setup / "display-binary-hashes.json").read_text())
        assert all(sha(setup / name) == digest for name, digest in hashes.items()), "Display binary changed"
    display_snapshot(setup / "display-inventory", output / "before")
    action = "restore" if restore else "setup"
    result = command(
        [
            str(setup / "session-display"),
            action,
            str(state),
            str(output / "before/stdout.txt"),
            str(output / "api.json"),
        ],
        output / "transaction",
        10,
    )
    if not ok(result):
        # Preserve post-failure geometry without retrying a transaction or admitting controls.
        command([str(setup / "display-inventory")], output / "after-failed-transaction", 5)
        raise RuntimeError(f"Session display {action} failed; inspect raw API/exit results")
    assert json.loads((output / "api.json").read_text())["passed"] is True
    check_session_display(output / "settling", setup, restore=restore, attempts=5)
    return True


def retain_scheme(path, output):
    shutil.copy2(path, output / (path.stem + ".original.xcscheme"))
    tree = ET.parse(path)
    action = tree.getroot().find("TestAction")
    assert action is not None
    action.attrib.update(
        systemAttachmentLifetime="keepAlways",
        userAttachmentLifetime="keepAlways",
        preferredScreenCaptureFormat="screenRecording",
    )
    tree.write(path, encoding="UTF-8", xml_declaration=True)
    shutil.copy2(path, output / path.name)


def run_test(base, selector, output, standalone, expected=None):
    selectors = [selector] if isinstance(selector, str) else selector
    expected = {tuple(selector.split("/"))} if expected is None else expected
    test = output / "test"
    result = command(
        base
        + ["-resultBundlePath", str(test / "Results.xcresult")]
        + FLAGS
        + ["-only-testing:" + value for value in selectors]
        + ["test-without-building"],
        test,
        900,
        observe=standalone,
    )
    exports = []
    for name, args in [
        ("summary", ["get", "test-results", "summary"]),
        ("attachments", ["export", "attachments", "--output-path", str(test / "captures")]),
        ("diagnostics", ["export", "diagnostics", "--output-path", str(test / "diagnostics")]),
    ]:
        exports.append(
            command(
                ["xcrun", "xcresulttool"] + args + ["--path", str(test / "Results.xcresult")],
                test / ("export-" + name),
                180,
            )
        )
    shutil.copy2(test / "export-summary/stdout.txt", test / "summary.json")
    log = (test / "stdout.txt").read_text() + (test / "stderr.txt").read_text()
    starts = re.findall(r"Test Case '-\[(\w+)\.(\w+) (\w+)\]' started\.", log)
    ends = re.findall(r"Test Case '-\[(\w+)\.(\w+) (\w+)\]' (passed|failed|skipped) \(", log)
    report = {
        "selector": selector,
        "starts": starts,
        "outcomes": ends,
        "process": result,
        "failed": True,
        "productAcceptance": False,
    }
    record(test / "counts.json", report)
    summary = json.loads((test / "summary.json").read_text())
    passed = (
        ok(result)
        and len(starts) == len(expected)
        and set(starts) == expected
        and len(ends) == len(expected)
        and set(ends) == {item + ("passed",) for item in expected}
    )
    passed = passed and all(
        summary[key] == value
        for key, value in {
            "totalTestCount": len(expected),
            "passedTests": len(expected),
            "failedTests": 0,
            "skippedTests": 0,
            "expectedFailures": 0,
            "result": "Passed",
        }.items()
    )
    report.update(xctestPassed=passed, exportsPassed=all(map(ok, exports)), summary=summary)
    record(test / "counts.json", report)
    passed = passed and report["exportsPassed"]
    if standalone:
        begins = re.findall(r"^AX_CONTROL_QUERY_BEGIN pid=(\d+)$", log, re.M)
        elapsed = re.findall(r"^AX_CONTROL_QUERY_END elapsed=([^\r\n]+)$", log, re.M)
        try:
            seconds = float(elapsed[0]) if len(elapsed) == 1 else None
        except ValueError:
            seconds = None
        if seconds is not None and not math.isfinite(seconds):
            seconds = None
        timely = (
            len(begins) == 1 and seconds is not None and math.isfinite(seconds) and 0 <= seconds <= QUERY_HEALTH_SECONDS
        )
        record(
            test / "query-health.json",
            {
                "passed": timely,
                "boundSeconds": QUERY_HEALTH_SECONDS,
                "elapsedSeconds": seconds,
                "beginMarkers": begins,
                "endMarkers": elapsed,
                "meaning": "Includes initial existence query and complete hierarchy; slow recovery is unhealthy",
            },
        )
        passed = passed and timely
        passed = passed and not json.loads((test / "sampling.json").read_text())["failed"]
        attachments = [
            item for group in json.loads((test / "captures/manifest.json").read_text()) for item in group["attachments"]
        ]
        matches = [
            item
            for item in attachments
            if item["suggestedHumanReadableName"].startswith("Standalone full AX hierarchy")
        ]
        assert len(matches) == 1, ("Full hierarchy missing or duplicated", matches)
        hierarchy = (test / "captures" / matches[0]["exportedFileName"]).read_text()
        assert all(
            token in hierarchy for token in ["Element subtree:", "standalone-text-field", "MenuBar", "File", "Edit"]
        )
        recordings = [
            test / "captures" / item["exportedFileName"]
            for item in attachments
            if item["exportedFileName"].lower().endswith((".mp4", ".mov"))
        ]
        assert len(recordings) == 1 and recordings[0].stat().st_size > 0
        checked(
            [str(output.parent / "preflight/pixels"), "video", str(recordings[0]), str(test / "usable-frame.png")],
            test / "pixel-check",
            60,
        )
    report.update(failed=not passed, summary=summary, evidenceInspection="required; not product acceptance")
    record(test / "counts.json", report)
    return passed


def base_command(project, scheme, derived):
    return [
        "xcodebuild",
        "-project",
        str(project),
        "-scheme",
        scheme,
        "-configuration",
        "Debug",
        "-destination",
        "platform=macOS,arch=arm64",
        "-derivedDataPath",
        str(derived),
    ]


def standalone(output):
    work = output / "work"
    work.mkdir()
    for name in ["AXMenuControl.swift", "AXMenuTests.swift", "project.yml"]:
        shutil.copy2(ROOT / name, work / name)
    generator = work / "XcodeGen"
    checked(
        ["git", "clone", "--no-checkout", "https://github.com/yonaskolb/XcodeGen.git", str(generator)],
        output / "fetch-generator",
        180,
    )
    checked(["git", "checkout", "--detach", GENERATOR], output / "pin-generator", 30, generator)
    checked(["git", "rev-parse", "HEAD"], output / "generator-pin", 10, generator)
    assert (output / "generator-pin/stdout.txt").read_text().strip() == GENERATOR
    checked(
        [
            "swift",
            "run",
            "--package-path",
            str(generator),
            "xcodegen",
            "--spec",
            str(work / "project.yml"),
            "--project",
            str(work),
        ],
        output / "generate",
        600,
    )
    project = work / "AXMenuDiagnostic.xcodeproj"
    retain_scheme(project / "xcshareddata/xcschemes/AXMenuDiagnostic.xcscheme", output)
    base = base_command(project, "AXMenuDiagnostic", work / "DerivedData")
    checked(base + ["build-for-testing"], output / "build", 600)
    for path in (work / "DerivedData/Build/Products").glob("*.xctestrun"):
        shutil.copy2(path, output / path.name)
    binary = work / "DerivedData/Build/Products/Debug/AXMenuControl.app/Contents/MacOS/AXMenuControl"
    checked(["otool", "-L", str(binary)], output / "linkage", 30)
    # Xcode's debug executable may carry the actual links in its sibling dylib.
    debug = binary.parent / "AXMenuControl.debug.dylib"
    if debug.exists():
        checked(["otool", "-L", str(debug)], output / "debug-linkage", 30)
    for log in output.glob("*linkage/stdout.txt"):
        assert not any(
            token in log.read_text()
            for token in ["CaperCore", "CaperMobile", "CaperRTC", "WebRTC", "SwiftUI.framework"]
        )
    record(
        output / "binary-hashes.json",
        {str(p.relative_to(work)): sha(p) for p in binary.parent.iterdir() if p.is_file()},
    )
    return run_test(base, AX, output, True)


def resolve_sidebar_packages(base, project, derived, output):
    # Retry only the observed SwiftPM binary-transfer diagnostic, never a build.
    transport = (
        "failed downloading 'https://github.com/stasel/WebRTC/releases/download/153.0.0/"
        "WebRTC-M153.xcframework.zip' which is required by binary target 'WebRTC': "
        'downloadError("Could not connect to the server.")'
    )
    expected = {"webrtc": "153.0.0", "libwebp-xcode": "1.6.0"}
    resolved = project / "project.xcworkspace/xcshareddata/swiftpm/Package.resolved"
    packages = derived / "SourcePackages"
    started = time.monotonic()
    deadline = started + 370
    for attempt in range(1, 4):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise RuntimeError("package resolution exhausted its 370s budget")
        directory = output / f"attempt-{attempt}"
        result = command(base + ["-resolvePackageDependencies"], directory, min(120, remaining))
        metadata = {}
        for path in [resolved, packages / "workspace-state.json", *sorted(packages.glob("checkouts/*/Package.swift"))]:
            name = f"{path.parent.name}-Package.swift" if path.name == "Package.swift" else path.name
            metadata[str(path)] = {"present": path.is_file()}
            if path.is_file():
                shutil.copy2(path, directory / name)
                metadata[str(path)].update(copy=name, sha256=sha(directory / name))
        record(directory / "metadata.json", metadata)
        # Missing metadata is permitted only on failed resolution, not on success.
        if resolved.is_file():
            pins = json.loads((directory / "Package.resolved").read_text())["pins"]
            assert len(pins) == len(expected) and {p["identity"]: p["state"]["version"] for p in pins} == expected
        stdout = (directory / "stdout.txt").read_text(errors="replace")
        stderr = (directory / "stderr.txt").read_text(errors="replace")
        remainder = []
        for text in [stdout, stderr]:
            text = text.replace(transport + "fatalError", "").replace(transport, "")
            text = text.replace("xcodebuild: error: Could not resolve package dependencies:", "")
            text = re.sub(r"(?m)^\s*fatalError\s*$", "", text)
            text = re.sub(
                r"(?m)^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d+ xcodebuild\[\d+:\d+\] Writing error result bundle to [^\n]+\.xcresult$",
                "",
                text,
            )
            remainder.append(text.strip())
        progress = {"Command line invocation:", "Resolve Package Graph"}
        for owner, name, version in [("stasel", "WebRTC", "153.0.0"), ("SDWebImage", "libwebp-Xcode", "1.6.0")]:
            progress.update(
                {
                    f"Fetching from https://github.com/{owner}/{name}.git",
                    f"Updating from https://github.com/{owner}/{name}.git",
                    f"Creating working copy of package ‘{name}’",
                    f"Checking out {version} of package ‘{name}’",
                }
            )
        transport_only = (
            result["returnCode"] == 74
            and result["signal"] is None
            and not result["timedOut"]
            and transport in stdout + stderr
            and not remainder[1]
            and all(
                line.strip() in progress
                or line.strip().startswith("/Applications/Xcode_26.6.app/Contents/Developer/usr/bin/xcodebuild ")
                for line in remainder[0].splitlines()
                if line.strip()
            )
            and not re.search(
                r"\b(error|failed|failure|fatalerror|checksum|integrity|signature|fingerprint|corrupt|invalid|version|conflict|mismatch)\b",
                "\n".join(remainder),
                re.I,
            )
        )
        elapsed = time.monotonic() - started
        passed = ok(result) and result["signal"] is None and resolved.is_file() and elapsed <= 370
        retry = transport_only and attempt < 3 and deadline - time.monotonic() > 5
        record(
            directory / "decision.json",
            {
                "passed": passed,
                "identifiedTransportFailure": transport_only,
                "retry": retry,
                "elapsedSeconds": elapsed,
                "overallBoundSeconds": 370,
                "attemptLimit": 3,
                "delaySeconds": 5 if retry else 0,
            },
        )
        if passed:
            return
        if not retry:
            raise RuntimeError(f"package resolution stopped at attempt {attempt}; see retained evidence")
        time.sleep(5)


def ui_evidence(output, checkpoints):
    captures = output / "test/captures"
    attachments = [
        item for group in json.loads((captures / "manifest.json").read_text()) for item in group["attachments"]
    ]
    recordings = [item for item in attachments if item["exportedFileName"].lower().endswith((".mp4", ".mov"))]
    assert recordings and all((captures / item["exportedFileName"]).stat().st_size > 0 for item in recordings)
    retained = []
    for checkpoint in checkpoints:
        for prefix in ["AppKit ", "AX AppKit "]:
            name = prefix + checkpoint + " — local fixture"
            matches = [item for item in attachments if item["suggestedHumanReadableName"].startswith(name)]
            assert len(matches) == 1, (name, matches)
            path = captures / matches[0]["exportedFileName"]
            assert path.stat().st_size > 0, path
            if prefix == "AX AppKit ":
                assert "Element subtree:" in path.read_text(), path
            retained.append(matches[0])
    record(output / "evidence.json", {"checkpoints": retained, "recordings": recordings, "inspection": "required"})


def cumulative(output, forward_only=False):
    assert json.loads((output.parent / "health/report.json").read_text())["failed"] is False
    checked(["git", "rev-parse", "HEAD", "HEAD^{tree}"], output / "product-pin", 10)
    assert (output / "product-pin/stdout.txt").read_text().splitlines() == [SOURCE, TREE]
    checked(["git", "diff", "--exit-code"], output / "clean", 10)
    assert sha(TEST_FILE) == "38658ea7cdd3ea9a8625d54f589ef8f663ed4df2a111883409e7c7ced948a832"
    shutil.copy2(TEST_FILE, output / "NavigationSidebarTests.swift")
    record(output / "test-source-hash.json", {"sha256": sha(TEST_FILE), "overlay": False})
    selection = json.loads((ROOT / "selections.json").read_text())
    assert selection["product"] == SOURCE
    expected = {tuple(value.split("/")) for value in selection["coreAndViews"]}
    assert len(expected) == len(selection["coreAndViews"]) == 37
    assert sum(item[0] == "CaperCoreTests" for item in expected) == 14
    assert sum(item[0] == "CaperAppKitViewTests" for item in expected) == 23
    assert SIDEBAR in selection["coreAndViews"]
    assert len(selection["ui"]) == len({item["selector"] for item in selection["ui"]}) == 6
    shutil.copy2(ROOT / "selections.json", output / "selections.json")
    # Preserve the original combined desktop requirement; do not resize the host.
    checked(
        [
            "xcrun",
            "swift",
            "-e",
            "import AppKit; import Foundation; "
            'guard let f = NSScreen.main?.visibleFrame else { print("No visible desktop"); exit(1) }; '
            'print(f); print("Keyboard navigation: \\(NSApplication.shared.isFullKeyboardAccessEnabled)"); '
            "guard f.width >= 1440 && f.height >= 900 else { exit(1) }",
        ],
        output / "desktop",
        60,
    )
    # The historical eight-line capture-only overlay; test methods stay byte-identical.
    ui_source = Path("apps/native/apple/Tests/CaperAppKitTests/CaperAppKitTests.swift")
    assert sha(ui_source) == "42785b0654aa6934519ce9bf08e72c9d2e88de84c29cdffd0d8b13470e19e8c8"
    shutil.copy2(ui_source, output / "CaperAppKitTests.original.swift")
    patch = ROOT / "ax-capture.patch"
    checked(["git", "apply", "--check", str(patch)], output / "overlay-check", 10)
    checked(["git", "apply", str(patch)], output / "overlay-apply", 10)
    assert sha(ui_source) == "0f0d4665aca9c6848c89900fe1c63f6a3746cff0a37669d1feabb21705e13a5c"
    if forward_only:
        forward_observation.install(output, checked, sha, record)
    shutil.copy2(ui_source, output / "CaperAppKitTests.observed.swift")
    checked(["git", "diff", "--binary"], output / "observation-diff", 10)
    record(output / "observation-hashes.json", {"patch": sha(patch), "observedUI": sha(ui_source)})
    # Preserve the product's curl transfer retries; Cargo and preparation are not retried.
    checked(
        [
            "bash",
            "-c",
            "export CARGO_NET_RETRY=0; bash apps/native/apple/prepare.sh",
        ],
        output / "prepare",
        2700,
    )
    generator = "apps/native/apple/.build/xcodegen-" + GENERATOR
    checked(
        [
            "swift",
            "run",
            "--package-path",
            generator,
            "xcodegen",
            "--spec",
            "apps/native/apple/native-project.yml",
            "--project",
            "apps/native/apple",
        ],
        output / "generate",
        300,
    )
    project = Path("apps/native/apple/CaperNative.xcodeproj")
    for scheme in ["CaperAppKit", "CaperAppKitTests"]:
        retain_scheme(project / f"xcshareddata/xcschemes/{scheme}.xcscheme", output)
    derived = Path("apps/native/apple/DerivedData-SidebarDiagnostic")
    base = base_command(project, "CaperAppKit", derived)
    resolve_sidebar_packages(base, project, derived, output / "resolve-packages")
    base += ["-disableAutomaticPackageResolution"]
    ui_base = base_command(project, "CaperAppKitTests", derived) + ["-disableAutomaticPackageResolution"]
    for scheme, args in [("CaperAppKit", base), ("CaperAppKitTests", ui_base)]:
        directory = output / ("build-" + scheme)
        checked(args + ["-resultBundlePath", str(directory / "Results.xcresult"), "build-for-testing"], directory, 1800)
    products = derived / "Build/Products"
    for path in products.glob("*.xctestrun"):
        shutil.copy2(path, output / path.name)
    record(
        output / "binary-hashes.json",
        {str(path.relative_to(products)): sha(path) for path in products.glob("**/Contents/MacOS/*") if path.is_file()},
    )
    selections = [
        ("core-and-views", base, ["CaperCoreTests/MessageFormattingTests", "CaperAppKitViewTests"], expected, [])
    ]
    selections += [
        (
            item["selector"].split("/")[-1],
            ui_base,
            item["selector"],
            {tuple(item["selector"].split("/"))},
            item["captures"],
        )
        for item in selection["ui"]
    ]
    if forward_only:
        selections = [item for item in selections if item[2] == forward_observation.SELECTOR]
        assert len(selections) == 1
    record(output / "execution-selection.json", {"forwardOnly": forward_only, "selectors": [s[2] for s in selections]})
    check_session_display(output / "display-before-tests", output.parent / "display-setup")
    with (output / "fixture.log").open("wb") as log:
        fixture = subprocess.Popen(
            ["node", "scripts/native-parity-fixture.mjs"], stdout=log, stderr=log, start_new_session=True
        )
        try:
            ready = False
            for _ in range(100):
                if fixture.poll() is not None:
                    break
                try:
                    with urllib.request.urlopen("http://127.0.0.1:3001/health", timeout=1) as response:
                        ready = response.status == 200
                    if ready:
                        break
                except (urllib.error.URLError, TimeoutError):
                    pass
                time.sleep(0.2)
            assert ready and fixture.poll() is None, "Synthetic fixture did not become ready"
            results = []
            for name, args, selectors, identities, checkpoints in selections:
                directory = output / name
                result = {"name": name, "failed": True}
                try:
                    assert fixture.poll() is None, "Synthetic fixture exited"
                    try:
                        passed = run_test(args, selectors, directory, False, identities)
                    finally:
                        if forward_only:
                            try:
                                result["observationComplete"] = forward_observation.collect(directory, record)
                            except Exception as error:
                                result["observationComplete"] = False
                                result["observationError"] = f"{type(error).__name__}: {error}"
                    result["testPassed"] = passed
                    if checkpoints:
                        ui_evidence(directory, checkpoints)
                    result["failed"] = not passed or not result.get("observationComplete", True)
                except Exception as error:
                    result["error"] = f"{type(error).__name__}: {error}"
                results.append(result)
                record(directory / "report.json", result)
                record(
                    output / "selections-report.json", {"results": results, "complete": len(results) == len(selections)}
                )
            return not any(result["failed"] for result in results)
        finally:
            if fixture.poll() is None:
                os.killpg(fixture.pid, signal.SIGTERM)
                try:
                    fixture.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(fixture.pid, signal.SIGKILL)
                    fixture.wait()


def main():
    mode, output = sys.argv[1], Path(sys.argv[2]).resolve()
    if mode in {"health", "finalize"}:
        reports = {}
        root = output.parent if mode == "health" else output
        names = (
            ["display-setup", "preflight", "graphics", "standalone"]
            if mode == "health"
            else ["display-setup", "preflight", "graphics", "standalone", "health", "cumulative", "display-restore"]
        )
        for name in names:
            path = root / name / "report.json"
            reports[name] = (
                json.loads(path.read_text())
                if path.exists()
                else {"failed": True, "reason": "not completed; inspect workflow step outcomes"}
            )
        record(
            output / ("report.json" if mode == "health" else "summary.json"),
            {"checks": reports, "failed": any(r["failed"] for r in reports.values()), "productAcceptance": False},
        )
        return int(any(r["failed"] for r in reports.values()))
    report = {"failed": True, "mode": mode, "productAcceptance": False}
    try:
        if mode in {"preflight", "display-inventory", "display-setup"}:
            if mode == "preflight":
                assert json.loads((output.parent / "display-setup/report.json").read_text())["failed"] is False
            preflight(output, provenance_only=mode != "preflight")
            if mode == "preflight":
                report["failed"] = False
            else:
                report["failed"] = not {"display-inventory": display_inventory, "display-setup": session_display}[mode](
                    output
                )
        elif mode == "display-restore":
            output.mkdir(parents=True, exist_ok=False)
            report["failed"] = not session_display(output, restore=True)
        else:
            assert mode in {"graphics", "standalone", "cumulative", "forwarding"}
            assert json.loads((output.parent / "preflight/gate.json").read_text())["passed"]
            output.mkdir(parents=True, exist_ok=False)
            try:
                passed = {
                    "graphics": graphics,
                    "standalone": standalone,
                    "cumulative": cumulative,
                    "forwarding": lambda out: cumulative(out, forward_only=True),
                }[mode](output)
                report["checkPassed"] = passed
            finally:
                after = command(
                    [
                        sys.executable,
                        str(ROOT / "crashes.py"),
                        "capture",
                        str(output / "crashes-after"),
                        str(output.parent / "preflight/crash-baseline/manifest.json"),
                    ],
                    output / "collect-crashes-after",
                    30,
                )
                report["crashCollectionPassed"] = ok(after)
                inventory = output / "crashes-after/manifest.json"
                report["noNewOrChangedReports"] = (
                    ok(after) and inventory.is_file() and json.loads(inventory.read_text())["capturedFiles"] == 0
                )
            report["failed"] = not (passed and ok(after) and report["noNewOrChangedReports"])
    except Exception as error:
        report["error"] = f"{type(error).__name__}: {error}"
    record(output / "report.json", report)
    return int(report["failed"])


if __name__ == "__main__":
    sys.exit(main())
