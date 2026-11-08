"""NEVER MERGE. Two independent diagnostics; neither is product acceptance."""

from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import threading
import time
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parent
SOURCE = "5910cfa98ee038fe53204b333c7065879f29a9ba"
TREE = "8793de8321c788ce41587b97104971f805433000"
GENERATOR = "21ac9944b0ab546a07422dbed86f33dd2ebd76f8"
SIDEBAR = "CaperAppKitViewTests/NavigationSidebarTests/testVoiceCardPreservesFocusAndLiveRowWhenChannelsCollapse"
AX = "AXMenuTests/AXMenuTests/testInitialTextFieldAndFullHierarchy"
TEST_FILE = Path("apps/native/apple/Tests/CaperAppKitViewTests/NavigationSidebarTests.swift")
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


def command(argv, output, timeout, cwd=None, observe=False):
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
    """One sample pair, only while the original query is in flight. No relaunch."""
    report = {"failed": False, "status": "not_attempted", "reason": "query marker not observed before process exit"}
    try:
        while process.poll() is None:
            text = (output / "stdout.txt").read_text(errors="replace")
            match = re.search(r"AX_CONTROL_QUERY_BEGIN pid=(\d+)", text)
            if match:
                if "AX_CONTROL_QUERY_END" in text:
                    report["reason"] = "query completed before sampler observed marker"
                    break
                pid = int(match[1])
                report.update(status="attempted", pid=pid, observedAtUTC=time.time())
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


def crash_logs(output):
    """Run as a separately time-bounded subprocess; preserve original bytes."""
    output.mkdir(parents=True, exist_ok=False)
    records = []
    size = 0

    def walk_error(error):
        raise error

    for base in [Path.home() / "Library/Logs/DiagnosticReports", Path("/Library/Logs/DiagnosticReports")]:
        try:
            base.stat()
        except FileNotFoundError:
            continue
        for directory, _, names in os.walk(base, onerror=walk_error):
            for name in sorted(names):
                path = Path(directory) / name
                if path.suffix not in {".ips", ".crash", ".spin"} or not name.lower().startswith(
                    ("iconservices", "launchd", "axmenu", "caper", "dock", "spotlight")
                ):
                    continue
                length = path.stat().st_size
                size += length
                if length > 20_000_000 or size > 80_000_000 or len(records) >= 64:
                    raise RuntimeError("Crash-log retention bound exceeded; not silently truncating evidence")
                destination = output / f"{len(records):03d}-{path.name}"
                shutil.copy2(path, destination)
                records.append(
                    {"original": str(path), "retained": destination.name, "sha256": sha(destination), "bytes": length}
                )
                record(output / "manifest.json", {"files": records, "complete": False})
    record(
        output / "manifest.json",
        {"files": records, "complete": True, "note": "No report found is not proof that no crash occurred"},
    )


def preflight(output):
    output.mkdir(parents=True, exist_ok=False)
    shutil.copytree(ROOT, output / "source", ignore=shutil.ignore_patterns("__pycache__"))
    workflow = ROOT.parent / "workflows/apple-ax-sidebar-diagnostic.yml"
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
    assert environment["GITHUB_REF"] == "refs/heads/apple-macos26-ax-sidebar-run"
    assert environment["ImageVersion"] == "20260824.0517.1"
    assert environment["DEVELOPER_DIR"] == "/Applications/Xcode_26.6.app/Contents/Developer"
    assert (output / "architecture/stdout.txt").read_text().strip() == "x86_64"
    assert re.search(r"ProductVersion:\s+26\.6\.1\s", (output / "os/stdout.txt").read_text())
    assert re.search(r"BuildVersion:\s+25G76\s", (output / "os/stdout.txt").read_text())
    assert (output / "xcode/stdout.txt").read_text().strip() == "Xcode 26.6\nBuild version 17F113"
    record(output / "gate.json", {"passed": True, "productCommit": SOURCE, "productTree": TREE})


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


def run_test(base, selector, output, standalone):
    test = output / "test"
    result = command(
        base
        + ["-resultBundlePath", str(test / "Results.xcresult")]
        + FLAGS
        + ["-only-testing:" + selector, "test-without-building"],
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
    expected = tuple(selector.split("/"))
    passed = ok(result) and all(map(ok, exports)) and starts == [expected] and ends == [expected + ("passed",)]
    passed = passed and all(
        summary[key] == value
        for key, value in {
            "totalTestCount": 1,
            "passedTests": 1,
            "failedTests": 0,
            "skippedTests": 0,
            "expectedFailures": 0,
            "result": "Passed",
        }.items()
    )
    attachments = [
        item for group in json.loads((test / "captures/manifest.json").read_text()) for item in group["attachments"]
    ]
    names = (
        ["Standalone full AX hierarchy"]
        if standalone
        else ["Sidebar geometry at outer width 344", "Sidebar geometry at outer width 300"]
    )
    for name in names:
        matches = [item for item in attachments if item["suggestedHumanReadableName"].startswith(name)]
        assert len(matches) == 1, (name, matches)
        path = test / "captures" / matches[0]["exportedFileName"]
        if standalone:
            assert "standalone-text-field" in path.read_text() and "Element subtree:" in path.read_text()
        else:
            snapshot = json.loads(path.read_text())
            assert "tree" in snapshot and "preferredScrollerStyle" in snapshot
    if standalone:
        passed = passed and not json.loads((test / "sampling.json").read_text())["failed"]
        assert len(re.findall(r"AX_CONTROL_QUERY_BEGIN pid=\d+", log)) == 1
        assert len(re.findall(r"AX_CONTROL_QUERY_END elapsed=", log)) == 1
        recordings = [
            test / "captures" / item["exportedFileName"]
            for item in attachments
            if item["exportedFileName"].lower().endswith((".mp4", ".mov"))
        ]
        assert recordings and all(path.is_file() and path.stat().st_size > 0 for path in recordings)
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
        "platform=macOS,arch=x86_64",
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


def sidebar(output):
    checked(["git", "rev-parse", "HEAD", "HEAD^{tree}"], output / "product-pin", 10)
    assert (output / "product-pin/stdout.txt").read_text().splitlines() == [SOURCE, TREE]
    checked(["git", "diff", "--exit-code"], output / "clean", 10)
    original = TEST_FILE.read_bytes()
    (output / "NavigationSidebarTests.original.swift").write_bytes(original)
    checked(["git", "apply", "--check", str(ROOT / "sidebar-geometry.patch")], output / "overlay-check", 10)
    checked(["git", "apply", str(ROOT / "sidebar-geometry.patch")], output / "overlay-apply", 10)
    shutil.copy2(TEST_FILE, output / "NavigationSidebarTests.observed.swift")
    record(
        output / "overlay-hashes.json",
        {
            "original": hashlib.sha256(original).hexdigest(),
            "observed": sha(TEST_FILE),
            "patch": sha(ROOT / "sidebar-geometry.patch"),
        },
    )
    checked(["git", "diff", "--binary"], output / "applied-diff", 10)
    # Keep source untouched, but do not retry its dependency downloads in this probe.
    checked(
        [
            "bash",
            "-c",
            'curl() { command curl "$@" --retry 0; }; export -f curl; '
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
    retain_scheme(project / "xcshareddata/xcschemes/CaperAppKit.xcscheme", output)
    base = base_command(project, "CaperAppKit", "apps/native/apple/DerivedData-SidebarDiagnostic")
    checked(base + ["build-for-testing"], output / "build", 1800)
    products = Path("apps/native/apple/DerivedData-SidebarDiagnostic/Build/Products")
    for path in products.glob("*.xctestrun"):
        shutil.copy2(path, output / path.name)
    record(
        output / "binary-hashes.json",
        {str(path.relative_to(products)): sha(path) for path in products.glob("**/Contents/MacOS/*") if path.is_file()},
    )
    return run_test(base, SIDEBAR, output, False)


def main():
    mode, output = sys.argv[1], Path(sys.argv[2]).resolve()
    if mode == "crash-logs":
        crash_logs(output)
        return 0
    if mode == "finalize":
        reports = {}
        for name in ["preflight", "standalone", "sidebar"]:
            path = output / name / "report.json"
            reports[name] = (
                json.loads(path.read_text())
                if path.exists()
                else {"failed": True, "reason": "not completed; inspect workflow step outcomes"}
            )
        record(
            output / "summary.json",
            {"checks": reports, "failed": any(r["failed"] for r in reports.values()), "productAcceptance": False},
        )
        return int(any(r["failed"] for r in reports.values()))
    report = {"failed": True, "mode": mode, "productAcceptance": False}
    try:
        if mode == "preflight":
            preflight(output)
            report["failed"] = False
        else:
            assert mode in {"standalone", "sidebar"}
            assert json.loads((output.parent / "preflight/gate.json").read_text())["passed"]
            output.mkdir(parents=True, exist_ok=False)
            before = command(
                [sys.executable, str(ROOT / "run.py"), "crash-logs", str(output / "crashes-before")],
                output / "collect-crashes-before",
                30,
            )
            try:
                passed = (standalone if mode == "standalone" else sidebar)(output)
            finally:
                after = command(
                    [sys.executable, str(ROOT / "run.py"), "crash-logs", str(output / "crashes-after")],
                    output / "collect-crashes-after",
                    30,
                )
            report["failed"] = not (passed and ok(before) and ok(after))
    except Exception as error:
        report["error"] = f"{type(error).__name__}: {error}"
    record(output / "report.json", report)
    return int(report["failed"])


if __name__ == "__main__":
    sys.exit(main())
