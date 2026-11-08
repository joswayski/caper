"""Pinned, never-merge combined acceptance. Run only after the graphics gate."""

import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import time
import urllib.error
import urllib.request
import xml.etree.ElementTree as ET

from run import MODES, ROOT, command, ok, record


SOURCE = "5910cfa98ee038fe53204b333c7065879f29a9ba"
TREE = "8793de8321c788ce41587b97104971f805433000"
UI_TESTS = (
    "testDraftFormattingCopyRepairUndoMenusAndSend",
    "testForwardPickerCancelLoadRetrySearchAndPartialDeliveryPreserveDrafts",
    "testAccountDialogsProtectProfileAndRestoreComposerFocus",
    "testPendingChannelLeaveRejectsEscapeAndBackdropUntilFailure",
    "testMessageReplyQuoteCancelSendAndOriginalNavigation",
    "testImagePasteEditSpoilerSendAndDecodedViewer",
)
TEST_FILE = Path("apps/native/apple/Tests/CaperAppKitTests/CaperAppKitTests.swift")
CAPTURE_ATTRIBUTES = {
    "systemAttachmentLifetime": "keepAlways",
    "userAttachmentLifetime": "keepAlways",
    "preferredScreenCaptureFormat": "screenRecording",
}


def graphics_passed(directory):
    summary = json.loads((directory / "summary.json").read_text())
    modes = summary["modes"]
    return (
        summary["failed"] is False
        and [item["mode"] for item in modes] == list(MODES)
        and all(ok(item) and item["survivalMarker"] for item in modes)
    )


def retain_scheme(path, evidence):
    shutil.copy2(path, evidence / (path.stem + ".original.xcscheme"))
    tree = ET.parse(path)
    action = tree.getroot().find("TestAction")
    assert action is not None, path
    action.attrib.update(CAPTURE_ATTRIBUTES)
    tree.write(path, encoding="UTF-8", xml_declaration=True)
    shutil.copy2(path, evidence / path.name)


def expected_core_tests():
    files = [Path("apps/native/apple/Tests/CaperCoreTests/MessageFormattingTests.swift")]
    files += sorted(Path("apps/native/apple/Tests/CaperAppKitViewTests").glob("*.swift"))
    expected = set()
    for path in files:
        source = path.read_text()
        classes = re.findall(r"class (\w+): XCTestCase", source)
        assert len(classes) == 1, path
        module = "CaperCoreTests" if path.name == "MessageFormattingTests.swift" else "CaperAppKitViewTests"
        expected.update((module, classes[0], method) for method in re.findall(r"func (test\w+)\(", source))
    assert len(expected) == 37, expected
    return expected


def check_result(directory, expected, require_ui_evidence):
    summary = json.loads((directory / "summary.json").read_text())
    process = json.loads((directory / "result.json").read_text())
    log = (directory / "stdout.txt").read_text() + (directory / "stderr.txt").read_text()
    outcomes = re.findall(r"Test Case '-\[(\w+)\.(\w+) (\w+)\]' (passed|failed|skipped) \(", log)
    starts = re.findall(r"Test Case '-\[(\w+)\.(\w+) (\w+)\]' started\.", log)
    passed = (
        ok(process)
        and len(starts) == len(expected)
        and set(starts) == expected
        and len(outcomes) == len(expected)
        and {(module, cls, method) for module, cls, method, _ in outcomes} == expected
        and all(outcome == "passed" for _, _, _, outcome in outcomes)
        and summary["totalTestCount"] == len(expected)
        and summary["passedTests"] == len(expected)
        and summary["failedTests"] == 0
        and summary["skippedTests"] == 0
        and summary["expectedFailures"] == 0
        and summary["result"] == "Passed"
    )
    evidence = {}
    if require_ui_evidence:
        captures = directory / "captures"
        groups = json.loads((captures / "manifest.json").read_text())
        attachments = [item for group in groups for item in group["attachments"]]
        videos = [
            item["exportedFileName"]
            for item in attachments
            if item["exportedFileName"].lower().endswith((".mp4", ".mov"))
        ]
        ax = [
            item["exportedFileName"]
            for item in attachments
            if item["suggestedHumanReadableName"].startswith("AX AppKit ")
        ]
        evidence = {"recordings": videos, "axTrees": ax, "inspection": "pending human/agent review"}
        passed = (
            passed and bool(videos) and bool(ax) and all((captures / name).stat().st_size > 0 for name in videos + ax)
        )
        passed = passed and all("Element subtree:" in (captures / name).read_text() for name in ax)
    record(directory / "acceptance.json", {"automatedGatePassed": passed, "outcomes": outcomes, "evidence": evidence})
    return passed


def main():
    output, control = map(lambda value: Path(value).resolve(), sys.argv[1:3])
    output.mkdir(parents=True, exist_ok=False)
    assert graphics_passed(control), "No Caper build unless every standalone process survives"
    assert subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip() == SOURCE
    assert subprocess.check_output(["git", "rev-parse", "HEAD^{tree}"], text=True).strip() == TREE
    subprocess.run(["git", "diff", "--exit-code"], check=True)
    record(output / "source.json", {"commit": SOURCE, "tree": TREE, "uiMethods": list(UI_TESTS)})
    shutil.copy2(TEST_FILE, output / "CaperAppKitTests.original.swift")
    patch = ROOT / "ax-capture.patch"
    subprocess.run(["git", "apply", "--check", str(patch)], check=True)
    subprocess.run(["git", "apply", str(patch)], check=True)
    shutil.copy2(TEST_FILE, output / "CaperAppKitTests.observed.swift")
    record(
        output / "observation-hashes.json",
        {
            path.name: hashlib.sha256(path.read_bytes()).hexdigest()
            for path in [patch, output / "CaperAppKitTests.original.swift", output / "CaperAppKitTests.observed.swift"]
        },
    )
    # A small/absent desktop is a gap, never a reason to change UI or assertions.
    display = command(
        [
            "xcrun",
            "swift",
            "-e",
            "import AppKit; "
            "let f = NSScreen.main!.visibleFrame; print(f); "
            'print("Keyboard navigation: \\(NSApplication.shared.isFullKeyboardAccessEnabled)"); '
            "precondition(f.width >= 1440 && f.height >= 900)",
        ],
        output / "desktop",
        60,
    )
    if not ok(display):
        return 1
    prepare = command(["bash", "apps/native/apple/prepare.sh"], output / "prepare", 2700)
    if not ok(prepare):
        return 1
    generator = "apps/native/apple/.build/xcodegen-21ac9944b0ab546a07422dbed86f33dd2ebd76f8"
    generate = command(
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
    if not ok(generate):
        return 1
    command(["git", "diff", "--binary"], output / "applied-diff", 30)
    schemes = Path("apps/native/apple/CaperNative.xcodeproj/xcshareddata/xcschemes")
    for scheme in ["CaperAppKit", "CaperAppKitTests"]:
        retain_scheme(schemes / (scheme + ".xcscheme"), output)
    base = [
        "xcodebuild",
        "-project",
        "apps/native/apple/CaperNative.xcodeproj",
        "-configuration",
        "Debug",
        "-destination",
        "platform=macOS,arch=x86_64",
        "-derivedDataPath",
        "apps/native/apple/DerivedData-Combined",
    ]
    for scheme in ["CaperAppKit", "CaperAppKitTests"]:
        result = command(base + ["-scheme", scheme, "build-for-testing"], output / ("build-" + scheme), 1800)
        if not ok(result):
            return 1
    products = Path("apps/native/apple/DerivedData-Combined/Build/Products")
    for path in products.glob("*.xctestrun"):
        shutil.copy2(path, output / path.name)
    record(
        output / "binary-hashes.json",
        {
            str(path.relative_to(products)): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in products.glob("**/*.app/Contents/MacOS/*")
            if path.is_file()
        },
    )
    flags = [
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
    selections = [
        (
            "core-and-views",
            "CaperAppKit",
            ["CaperCoreTests/MessageFormattingTests", "CaperAppKitViewTests"],
            expected_core_tests(),
        )
    ]
    selections += [
        (
            method,
            "CaperAppKitTests",
            ["CaperAppKitTests/CaperAppKitTests/" + method],
            {("CaperAppKitTests", "CaperAppKitTests", method)},
        )
        for method in UI_TESTS
    ]
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
            assert ready, "Synthetic fixture did not become ready"
            failures = []
            for name, scheme, selectors, expected in selections:
                directory = output / name
                bundle = directory / "Results.xcresult"
                args = base + ["-scheme", scheme, "-resultBundlePath", str(bundle)] + flags
                args += ["-only-testing:" + selector for selector in selectors] + ["test-without-building"]
                print("Running once:", name, flush=True)
                command(args, directory, 900)
                exports = []
                for kind, extra in (
                    ("summary", ["get", "test-results", "summary"]),
                    ("attachments", ["export", "attachments", "--output-path", str(directory / "captures")]),
                    ("diagnostics", ["export", "diagnostics", "--output-path", str(directory / "diagnostics")]),
                ):
                    exports.append(
                        command(
                            ["xcrun", "xcresulttool"] + extra + ["--path", str(bundle)],
                            directory / ("export-" + kind),
                            180,
                        )
                    )
                shutil.copy2(directory / "export-summary/stdout.txt", directory / "summary.json")
                if not all(map(ok, exports)):
                    failures.append(name)
                    continue
                try:
                    passed = check_result(directory, expected, name != "core-and-views")
                except (OSError, ValueError, KeyError, TypeError) as error:
                    record(directory / "evidence-error.json", {"failed": True, "error": str(error)})
                    passed = False
                if not passed:
                    failures.append(name)
            record(
                output / "acceptance.json",
                {
                    "failed": bool(failures),
                    "failures": failures,
                    "selectors": [name for name, _, _, _ in selections],
                    "visualAndAXInspection": "still required",
                },
            )
            return int(bool(failures))
        finally:
            if fixture.poll() is None:
                os.killpg(fixture.pid, signal.SIGTERM)
                try:
                    fixture.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(fixture.pid, signal.SIGKILL)
                    fixture.wait()


if __name__ == "__main__":
    sys.exit(main())
