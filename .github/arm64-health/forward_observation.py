"""Inactive forwarding-only observation overlay. Never changes test outcomes."""

import hashlib
import json
from pathlib import Path
import shutil
import tempfile

ROOT = Path(__file__).resolve().parent
SELECTOR = "CaperAppKitTests/CaperAppKitTests/testForwardPickerCancelLoadRetrySearchAndPartialDeliveryPreserveDrafts"
CANARY = "TEST FIXTURE — cancel this note, not the channel draft"
UI = Path("apps/native/apple/Tests/CaperAppKitTests/CaperAppKitTests.swift")
CORE = Path("apps/native/apple/Sources/CaperCore/EmojiAutocomplete.swift")
HASHES = {
    str(UI): (
        "0f0d4665aca9c6848c89900fe1c63f6a3746cff0a37669d1feabb21705e13a5c",
        "c98f01c3be18afd5c1690b193d905f7ffeeb4b29ff486f1cf96e39842db40963",
    ),
    str(CORE): (
        "9d8727c4d16fa528936ee4d1fd9315cfa9e2041629f9a6ea6d3b19b2dd093f94",
        "a4f0339c4a42c9bce77f28cbe9a323ceacec8db9da26a858559b12dfa04e7135",
    ),
}


def install(output, checked, sha, record):
    for path, (before, _) in HASHES.items():
        assert sha(Path(path)) == before, path
    patch = ROOT / "forward-note.patch"
    checked(["git", "apply", "--check", str(patch)], output / "forward-overlay-check", 10)
    checked(["git", "apply", str(patch)], output / "forward-overlay-apply", 10)
    sources = output / "forward-observed-sources"
    sources.mkdir()
    hashes = {}
    for path, (_, after) in HASHES.items():
        assert sha(Path(path)) == after, path
        shutil.copy2(path, sources / Path(path).name)
        hashes[path] = after
    helper = ROOT / "ForwardNoteObservation.swift"
    for directory in (UI.parent, CORE.parent):
        target = directory / helper.name
        assert not target.exists()
        shutil.copy2(helper, target)
        hashes[str(target)] = sha(target)
    shutil.copy2(helper, sources / helper.name)
    record(output / "forward-observation-inputs.json", {"sources": hashes, "patch": sha(patch)})


def assess(events):
    """Report evidence completeness, not a product/root-cause classification."""
    allowed = {
        "test.prepared",
        "test.pasteboard",
        "test.end",
        "app.ready",
        "app.mouseDown.beforeRouting",
        "app.keyDown.beforeRouting",
        "paste.enter",
        "paste.exit",
        "focus.become",
        "focus.resign",
        "update.enter",
        "update.exit",
        "textDidChange.enter",
        "textDidChange.exit",
    }
    assert all(e["event"] in allowed for e in events), "Unknown/error/limit event"
    streams = {}
    for event in events:
        streams.setdefault(event["stream"], []).append(event)
    assert len(streams) == 2, "Expected one test stream and one app stream"
    for items in streams.values():
        assert len(items) <= 257
        assert [e["sequence"] for e in items] == list(range(1, len(items) + 1)), "Missing/duplicate sequence"
        times = [int(e["uptimeNanoseconds"]) for e in items]
        assert times == sorted(times), "Non-monotonic stream"
        assert len({e["pid"] for e in items}) == 1
    names = [e["event"] for e in events]
    assert not any("error" in n.lower() or n in {"observation.limit", "app.expired"} for n in names)
    for name in ("test.prepared", "test.pasteboard", "test.end", "app.ready"):
        assert names.count(name) == 1, name
    begin = next(e for e in events if e["event"] == "test.pasteboard")
    end = next(e for e in events if e["event"] == "test.end")
    assert begin["stream"] == end["stream"]
    for key in ("writeSucceeded", "readbackMatches"):
        assert type(begin.get(key)) is bool, key
    assert type(begin.get("changeCount")) is int and begin["changeCount"] >= 0, "changeCount"
    assert not begin["readbackMatches"] or begin["writeSucceeded"], "Readback cannot match after a failed write"

    def canary_metadata(data, matching):
        if matching:
            assert data.get("sha256") == hashlib.sha256(CANARY.encode()).hexdigest(), "Canary digest"
            assert type(data.get("utf16Length")) is int and data["utf16Length"] == len(CANARY.encode("utf-16-le")) // 2
            assert data.get("types") == ["public.utf8-plain-text"], "Canary types"
        else:
            assert not {"sha256", "utf16Length", "types"}.intersection(data), "Unexpected clipboard metadata"

    canary_metadata(begin, begin["readbackMatches"])
    for event in events:
        if event["stream"] != begin["stream"] and event["event"] != "app.ready":
            assert int(begin["uptimeNanoseconds"]) <= int(event["uptimeNanoseconds"]) <= int(end["uptimeNanoseconds"])
    assert next(e for e in events if e["event"] == "app.ready")["monitorInstalled"] is True
    states = [e for e in events if e["event"] in {"app.mouseDown.beforeRouting", "app.keyDown.beforeRouting"}]
    assert states, "No independent state observation during replacement; paste absence is inconclusive"
    for state in states:
        for key in ("view", "firstResponderClass"):
            assert type(state.get(key)) is str and state[key], key
        for key in (
            "isEditable",
            "isSelectable",
            "appActive",
            "windowIsKey",
            "firstResponderIsNote",
            "swiftUIEditable",
            "swiftUIEnabled",
        ):
            assert type(state.get(key)) is bool, key
    for event in (e for e in events if e["event"] == "paste.enter"):
        data = event.get("pasteboard")
        assert type(data) is dict, "Missing paste-entry clipboard metadata"
        for key in ("markerChangeCount", "changeCount", "changeCountAfter"):
            assert type(data.get(key)) is int and data[key] >= 0, key
        for key in ("provenanceSafe", "contentInspected"):
            assert type(data.get(key)) is bool, key
        assert data["markerChangeCount"] == begin["changeCount"], "Marker count changed"
        safe = begin["writeSucceeded"] and begin["readbackMatches"]
        assert data["provenanceSafe"] is safe, "Marker provenance differs from original observation"
        changed = data["changeCount"] != begin["changeCount"] or data["changeCountAfter"] != begin["changeCount"]
        assert data.get("changeStatus") == ("changed" if changed else "unchanged"), "Count status"
        may_inspect = safe and data["changeCount"] == begin["changeCount"]
        assert not data["contentInspected"] or may_inspect, "Unsafe content inspection"
        status = data.get("readbackStatus")
        if not may_inspect:
            assert status == "skipped-unsafe-provenance", "Unsafe provenance must skip inspection"
        else:
            assert status in {"matching-canary", "readback-unavailable"}, "Readback status"
        if status == "matching-canary":
            assert not changed and data["contentInspected"], "Match requires inspected, unchanged content"
        canary_metadata(data, status == "matching-canary")
    assert names.count("paste.enter") == names.count("paste.exit"), "Incomplete paste callback"
    return {
        "complete": True,
        "pasteCallbacks": names.count("paste.enter"),
        "viewIdentities": sorted({e["view"] for e in events if "view" in e}),
        "classification": "observations only; inspect ordering and primary XCTest result",
    }


def collect(output, record):
    report = {"complete": False, "productAcceptance": False}
    destination = output / "observations"
    destination.mkdir()
    try:
        test = output / "test"
        log = (test / "stdout.txt").read_text() + (test / "stderr.txt").read_text()
        assert "CAPER_FORWARD_OBSERVATION_IO_ERROR" not in log
        records = []
        for line in log.splitlines():
            if "CAPER_FORWARD_OBSERVATION " in line:
                records.append(json.loads(line.split("CAPER_FORWARD_OBSERVATION ", 1)[1]))
        prepared = [e for e in records if e["event"] == "test.prepared"]
        assert len(prepared) == 1, "Missing/duplicate test observation directory"
        source = Path(prepared[0]["directory"])
        assert not source.is_symlink() and source.parent.resolve() == Path(tempfile.gettempdir()).resolve()
        assert source.name.startswith("caper-forward-observation-")
        files = list(source.glob("events-*.jsonl"))
        assert 1 <= len(files) <= 2
        events = []
        for path in sorted(files):
            assert not path.is_symlink() and path.stat().st_size <= 1_048_576
            shutil.copy2(path, destination / path.name)
        record(
            destination / "hashes.json",
            {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in destination.glob("*.jsonl")},
        )
        for path in sorted(destination.glob("*.jsonl")):
            events.extend(json.loads(line) for line in path.read_text().splitlines())
        record(
            destination / "ordered.json",
            sorted(events, key=lambda e: (int(e["uptimeNanoseconds"]), e["stream"], e["sequence"])),
        )
        report.update(assess(events))
    except Exception as error:
        report["error"] = f"{type(error).__name__}: {error}"
    record(destination / "report.json", report)
    return report["complete"]
