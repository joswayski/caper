"""Read-only bounded inventory, then new/changed crash capture. Never prune reports."""

import hashlib
import json
import os
from pathlib import Path
import sys
import time

ROOTS = (Path.home() / "Library/Logs/DiagnosticReports", Path("/Library/Logs/DiagnosticReports"))
PREFIXES = (
    "iconservices",
    "launchd",
    "axmenu",
    "caper",
    "dock",
    "spotlight",
    "graphics-control",
    "intelgraphics",
    "pixels",
    "metal-provenance",
    "xctest",
    "screencapture",
)
MAX_ENTRIES = 20_000
MAX_REPORTS = 2048
MAX_FILE_BYTES = 20_000_000
MAX_READ_BYTES = 256_000_000
MAX_CAPTURE_FILES = 64
MAX_CAPTURE_BYTES = 80_000_000


def save(path, data):
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")
    temporary.replace(path)


def fingerprint(stat):
    return {"bytes": stat.st_size, "mtimeNS": stat.st_mtime_ns, "ctimeNS": stat.st_ctime_ns, "inode": stat.st_ino}


def collect(mode, output, baseline_path=None, roots=ROOTS):
    output.mkdir(parents=True, exist_ok=False)
    manifest = {
        "mode": mode,
        "complete": False,
        "startedAtUTC": time.time(),
        "records": [],
        "missingSinceBaseline": [],
        "roots": [],
        "entriesVisited": 0,
        "readBytes": 0,
        "capturedFiles": 0,
        "capturedBytes": 0,
        "note": "Unchanged baseline reports are explicitly excluded, not deleted. An empty delta is not proof of no crash.",
    }
    path = output / "manifest.json"
    save(path, manifest)

    def walk_error(error):
        raise error

    try:
        assert mode in {"baseline", "capture"}
        baseline = {}
        if mode == "capture":
            data = Path(baseline_path).read_bytes()
            original = json.loads(data)
            assert original["complete"] is True and original["mode"] == "baseline"
            baseline = {r["original"]: r for r in original["records"]}
            manifest.update(
                baselinePath=str(baseline_path),
                baselineSHA256=hashlib.sha256(data).hexdigest(),
                baselineStartedAtUTC=original["startedAtUTC"],
            )
        seen = set()
        for root in roots:
            root_record = {"path": str(root), "status": "inspecting"}
            manifest["roots"].append(root_record)
            save(path, manifest)
            try:
                root.stat()
            except FileNotFoundError:
                root_record["status"] = "absent"
                save(path, manifest)
                continue
            for directory, directories, files in os.walk(root, onerror=walk_error):
                directories.sort()
                manifest["entriesVisited"] += 1 + len(directories) + len(files)
                if manifest["entriesVisited"] > MAX_ENTRIES:
                    raise RuntimeError("Directory entry inventory bound exceeded")
                for name in sorted(files):
                    source = Path(directory) / name
                    if source.suffix not in {".ips", ".crash", ".spin"} or not name.lower().startswith(PREFIXES):
                        continue
                    if len(manifest["records"]) >= MAX_REPORTS:
                        raise RuntimeError("Report inventory bound exceeded")
                    before = fingerprint(source.stat())
                    if before["bytes"] > MAX_FILE_BYTES or manifest["readBytes"] + before["bytes"] > MAX_READ_BYTES:
                        raise RuntimeError(f"Read byte bound exceeded at {source}")
                    # Bound even a report that grows after stat; a moving report fails this snapshot.
                    with source.open("rb") as stream:
                        raw = stream.read(before["bytes"] + 1)
                    manifest["readBytes"] += len(raw)
                    if len(raw) != before["bytes"] or fingerprint(source.stat()) != before:
                        raise RuntimeError(f"Report changed during read: {source}")
                    digest = hashlib.sha256(raw).hexdigest()
                    record = {"original": str(source), **before, "sha256": digest}
                    seen.add(str(source))
                    previous = baseline.get(str(source))
                    unchanged = previous is not None and all(previous[k] == record[k] for k in (*before, "sha256"))
                    if mode == "baseline":
                        record["classification"] = "baseline_inventory_only"
                    elif unchanged:
                        record["classification"] = "historical_unchanged_excluded"
                    else:
                        record["classification"] = "changed_since_baseline" if previous else "new_since_baseline"
                        if (
                            manifest["capturedFiles"] >= MAX_CAPTURE_FILES
                            or manifest["capturedBytes"] + len(raw) > MAX_CAPTURE_BYTES
                        ):
                            record["retentionFailure"] = "current-report capture bound exceeded"
                            manifest["records"].append(record)
                            raise RuntimeError(f"Current-report capture bound exceeded at {source}")
                        destination = f"{manifest['capturedFiles']:03d}-{source.name}"
                        (output / destination).write_bytes(raw)
                        record["retained"] = destination
                        manifest["capturedFiles"] += 1
                        manifest["capturedBytes"] += len(raw)
                    manifest["records"].append(record)
                    save(path, manifest)
            root_record["status"] = "inventoried"
            save(path, manifest)
        manifest["missingSinceBaseline"] = sorted(set(baseline) - seen)
        if manifest["missingSinceBaseline"]:
            raise RuntimeError("Baseline paths disappeared; inventory is incomplete")
        manifest["complete"] = True
    except Exception as error:
        manifest["error"] = f"{type(error).__name__}: {error}"
    save(path, manifest)
    return 0 if manifest["complete"] else 1


if __name__ == "__main__":
    sys.exit(collect(sys.argv[1], Path(sys.argv[2]).resolve(), sys.argv[3] if len(sys.argv) == 4 else None))
