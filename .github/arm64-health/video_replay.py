"""Inactive, never-merge proposal: two pinned files, one pixel invocation each, no Caper."""

import json
import os
from pathlib import Path
import re
import shutil
import sys

from run import ROOT, command, ok, record, sha

VIDEOS = (
    ("failed", "449df2b898b9b5b907a9989aeae75bf002035c69f18456912dd47a1faf8cd42c", 51285),
    ("passing", "4d490dea2913e03f82e8de57ca93d952b1a8919f00d461b1ee9aba8194ede597", 102694),
)
ENVIRONMENT = {
    "RUNNER_ENVIRONMENT": "github-hosted",
    "GITHUB_RUN_ATTEMPT": "1",
    "GITHUB_REF": "refs/heads/apple-video-phase-replay-run",
    "ImageVersion": "20261102.0473.1",
    "DEVELOPER_DIR": "/Applications/Xcode_26.6.app/Contents/Developer",
}


def main(output):
    output.mkdir(parents=True, exist_ok=False)
    report = {"failed": True, "productAcceptance": False, "runs": [], "stage": "environment"}
    try:
        environment = {key: os.environ.get(key) for key in (*ENVIRONMENT, "GITHUB_SHA", "GITHUB_RUN_ID")}
        record(output / "environment.json", environment)
        assert all(environment[key] == expected for key, expected in ENVIRONMENT.items()), "environment pin mismatch"
        assert re.fullmatch(r"[0-9a-f]{40}", environment["GITHUB_SHA"] or ""), "invalid head"
        report["stage"] = "identity"
        for name, argv, expected, timeout in (
            ("architecture", ["uname", "-m"], "arm64", 10),
            ("os-version", ["sw_vers", "-productVersion"], "26.7.2", 10),
            ("os-build", ["sw_vers", "-buildVersion"], "25G332", 10),
            ("xcode", ["xcodebuild", "-version"], "Xcode 26.6\nBuild version 17F113", 30),
            ("head", ["git", "rev-parse", "HEAD"], environment["GITHUB_SHA"], 10),
            ("clean", ["git", "status", "--porcelain", "--untracked-files=all"], "", 10),
        ):
            result = command(argv, output / name, timeout)
            assert ok(result), name + " command failed"
            assert (output / name / "stdout.txt").read_text().strip() == expected, name + " pin mismatch"
        report["stage"] = "input-hashes"
        result = command(["shasum", "-a", "256", "-c", str(ROOT / "inputs.sha256")], output / "hashes", 30)
        assert ok(result), "input hash mismatch"
        # Fixed local files only. No archive download, container sweep, capture or producer process.
        inputs = {}
        for name, expected, size in VIDEOS:
            path = ROOT / "video-replay" / (name + ".mp4")
            assert not path.is_symlink() and path.is_file(), name + " is not a regular input"
            assert path.stat().st_size == size and sha(path) == expected, name + " media mismatch"
            inputs[name] = {"sha256": expected, "size": size}
        record(output / "media-before.json", inputs)
        shutil.copytree(ROOT, output / "source", ignore=shutil.ignore_patterns("__pycache__"))
        report["stage"] = "compile"
        binary = output / "pixels"
        result = command(
            ["xcrun", "swiftc", "-parse-as-library", str(ROOT / "Pixels.swift"), "-o", str(binary)],
            output / "compile",
            180,
        )
        assert ok(result), "compile failed; neither video executed"
        record(output / "binary.json", {"sha256": sha(binary)})
        report["stage"] = "replay"
        for name, expected, size in VIDEOS:
            video = ROOT / "video-replay" / (name + ".mp4")
            result = command([str(binary), "video", str(video), str(output / (name + ".png"))], output / name, 60)
            # A failed invocation stays failed; the second file is an independent control, not a retry.
            entry = {"name": name, "process": result, "resultPassed": False}
            report["runs"].append(entry)
            try:
                entry["resultPassed"] = json.loads((output / name / "stdout.txt").read_text())["passed"] is True
            except (OSError, ValueError, KeyError, TypeError) as error:
                entry["resultReadError"] = f"{type(error).__name__}: {error}"
            entry["mediaUnchanged"] = video.stat().st_size == size and sha(video) == expected
            record(output / "report.json", report)
            assert entry["mediaUnchanged"], name + " input mutated"
        report["stage"] = "complete"
        report["failed"] = not all(ok(r["process"]) and r["resultPassed"] for r in report["runs"])
    except Exception as error:
        report["error"] = f"{type(error).__name__}: {error}"
    record(output / "report.json", report)
    return int(report["failed"])


if __name__ == "__main__":
    sys.exit(main(Path(sys.argv[1])))
