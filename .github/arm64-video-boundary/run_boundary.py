"""Inactive, never-merge: three distinct timestamps, each once in a fresh process."""

import json
import math
import os
from pathlib import Path
import re
import shutil
import sys

ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT.parent / "arm64-health"))
from run import command, ok, record, sha  # noqa: E402

MEDIA = ROOT.parent / "arm64-health/video-replay/failed.mp4"
MEDIA_SHA = "449df2b898b9b5b907a9989aeae75bf002035c69f18456912dd47a1faf8cd42c"
ENVIRONMENT = {
    "RUNNER_ENVIRONMENT": "github-hosted",
    "GITHUB_RUN_ATTEMPT": "1",
    "GITHUB_REF": "refs/heads/apple-video-boundary-run",
    "ImageVersion": "20261102.0473.1",
    "DEVELOPER_DIR": "/Applications/Xcode_26.6.app/Contents/Developer",
}


def media_matches():
    return not MEDIA.is_symlink() and MEDIA.is_file() and MEDIA.stat().st_size == 51285 and sha(MEDIA) == MEDIA_SHA


def read_case(directory, tick, process, image):
    """Completeness is separate from image success; negative observations stay negative."""
    result = json.loads((directory / "stdout.txt").read_text())
    lines = (directory / "stderr.txt").read_text().splitlines()
    events = [json.loads(line[15:]) for line in lines if line.startswith("BOUNDARY_VIDEO ")]
    assert 2 <= len(events) <= 10 and events == result["events"], "missing or inconsistent phase stream"
    assert [e["sequence"] for e in events] == list(range(1, len(events) + 1)), "sequence gap"
    assert all(int(b["uptimeNanoseconds"]) > int(a["uptimeNanoseconds"]) for a, b in zip(events, events[1:])), "order"
    requested = {"value": tick, "timescale": 600, "epoch": 0, "flags": 1}
    first = events[0]
    assert first["stage"] == "asset.create" and type(first["pid"]) is int and first["pid"] > 0, "process identity"
    assert all(first["requestedTime"][k] == v for k, v in requested.items()), "wrong timestamp"
    assert result["contentAcceptance"] == "not-evaluated", "unexpected content gate"
    generated = result["imageGenerationSucceeded"]
    assert type(generated) is bool and type(result["imageOutputComplete"]) is bool, "missing outcome"
    prefix = [
        "asset.create",
        "duration.load.start",
        "duration.load.result",
        "duration.validate",
        "image.request",
        "image.result",
        "png.encode",
        "png.write",
        "output.success",
    ]
    stages = [e["stage"] for e in events]
    if result["status"] == "complete":
        assert stages == prefix and generated and result["imageOutputComplete"] and ok(process), "inconsistent success"
        assert image.is_file() and image.read_bytes()[:8] == b"\x89PNG\r\n\x1a\n", "missing PNG"
    else:
        assert result["status"] == "error" and stages[-1] == "error", "missing terminal outcome"
        assert stages[:-1] == prefix[: len(stages) - 1], "incomplete failure prefix"
        assert result["failedStage"] == events[-1]["failedStage"] == stages[-2], "failure stage"
        assert process["returnCode"] == 1 and not process["timedOut"] and process["signal"] is None, "process mismatch"
        assert generated == ("image.result" in stages) and not result["imageOutputComplete"], "image/result conflation"
        chain = events[-1]["errorChain"]
        assert chain["end"] == "no-underlying-error" and 1 <= len(chain["entries"]) <= 16, "incomplete error chain"
        for entry in chain["entries"]:
            assert type(entry["code"]) is int and type(entry["domain"]) is str and type(entry["message"]) is str
            assert entry["textTruncated"] is False, "truncated error text"
    for event in events:
        required = {"duration.load.result": "duration", "image.request": "requestedTime", "image.result": "actualTime"}
        if event["stage"] in required:
            assert required[event["stage"]] in event, "missing time metadata"
        if event["stage"] == "image.result":
            assert all(type(event[k]) is int and event[k] > 0 for k in ("width", "height")), "missing image dimensions"
        for field in ("duration", "requestedTime", "actualTime"):
            if field in event:
                time = event[field]
                assert all(type(time[k]) is int for k in ("value", "timescale", "epoch", "flags")), "invalid CMTime"
                assert time["seconds"] is None or (
                    type(time["seconds"]) in (int, float) and math.isfinite(time["seconds"])
                ), "invalid seconds"
                if field != "duration":
                    assert time["timescale"] > 0 and time["seconds"] is not None, "non-numeric image time"
        if event["stage"] == "image.request":
            assert all(event["requestedTime"][k] == v for k, v in requested.items()), "request mismatch"
    return {
        "observationComplete": True,
        "imageGenerationSucceeded": generated,
        "imageOutputComplete": result["imageOutputComplete"],
        "contentAcceptance": "not-evaluated",
    }


def main(output):
    output.mkdir(parents=True, exist_ok=False)
    report = {"failed": True, "productAcceptance": False, "runs": [], "stage": "environment"}
    try:
        environment = {key: os.environ.get(key) for key in (*ENVIRONMENT, "GITHUB_SHA", "GITHUB_RUN_ID")}
        record(output / "environment.json", environment)
        assert all(environment[key] == expected for key, expected in ENVIRONMENT.items()), "environment mismatch"
        assert re.fullmatch(r"[0-9a-f]{40}", environment["GITHUB_SHA"] or ""), "invalid head"
        for name, argv, expected, timeout in (
            ("architecture", ["uname", "-m"], "arm64", 10),
            ("os-version", ["sw_vers", "-productVersion"], "26.7.2", 10),
            ("os-build", ["sw_vers", "-buildVersion"], "25G332", 10),
            ("xcode", ["xcodebuild", "-version"], "Xcode 26.6\nBuild version 17F113", 30),
            ("head", ["git", "rev-parse", "HEAD"], environment["GITHUB_SHA"], 10),
            ("clean", ["git", "status", "--porcelain", "--untracked-files=all"], "", 10),
        ):
            result = command(argv, output / name, timeout)
            assert ok(result) and (output / name / "stdout.txt").read_text().strip() == expected, name + " mismatch"
        report["stage"] = "inputs"
        assert ok(command(["shasum", "-a", "256", "-c", str(ROOT / "inputs.sha256")], output / "hashes", 30)), (
            "hash mismatch"
        )
        assert media_matches(), "media mismatch"
        record(output / "media-before.json", {"sha256": sha(MEDIA), "size": MEDIA.stat().st_size})
        for line in (ROOT / "inputs.sha256").read_text().splitlines():
            _, relative = line.split("  ", 1)
            source = ROOT.parent.parent / relative
            target = output / "source" / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, target)
        shutil.copyfile(ROOT / "inputs.sha256", output / "inputs.sha256")
        report["stage"] = "compile"
        binary = output / "boundary"
        result = command(
            ["xcrun", "swiftc", "-parse-as-library", str(ROOT / "Boundary.swift"), "-o", str(binary)],
            output / "compile",
            180,
        )
        assert ok(result), "compile failed; no timestamp case executed"
        record(output / "binary.json", {"sha256": sha(binary)})
        report["stage"] = "cases"

        def one_case(tick):
            assert media_matches(), "input changed; remaining cases not executed"
            directory = output / str(tick)
            image = output / (str(tick) + ".png")
            process = command([str(binary), str(MEDIA), str(tick), str(image)], directory, 60)
            entry = {"tick": tick, "timescale": 600, "process": process, "observationComplete": False}
            report["runs"].append(entry)
            try:
                entry.update(read_case(directory, tick, process, image))
            except (OSError, ValueError, KeyError, TypeError, AssertionError, IndexError) as error:
                entry["evidenceError"] = f"{type(error).__name__}: {error}"
            entry["mediaUnchanged"] = media_matches()
            if image.is_file():
                entry["image"] = {"sha256": sha(image), "size": image.stat().st_size}
            record(output / "report.json", report)
            assert entry["mediaUnchanged"], "input changed; remaining cases not executed"

        # Independent cases, not retries. Each command creates a fresh OS process/generator.
        one_case(369)
        one_case(370)
        one_case(1720)
        report["stage"] = "complete"
        report["observationsComplete"] = all(r["observationComplete"] for r in report["runs"])
        report["failed"] = not all(ok(r["process"]) and r["observationComplete"] for r in report["runs"])
    except Exception as error:
        report["error"] = f"{type(error).__name__}: {error}"
    record(output / "report.json", report)
    return int(report["failed"])


if __name__ == "__main__":
    sys.exit(main(Path(sys.argv[1])))
