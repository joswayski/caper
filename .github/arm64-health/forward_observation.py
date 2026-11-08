"""Inactive forwarding-only observation overlay. Never changes test outcomes."""

import hashlib
import json
import math
import os
from pathlib import Path
import re
import shutil
import stat
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
        "23a34c75f84cbcc6a6b44c01872547ab8456f94733e251e41763d973fc70eb78",
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
        "mouseDown.enter",
        "mouseDown.exit",
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
    for key in (
        "writeSucceeded",
        "readbackMatches",
        "readbackPresent",
        "exactCanaryMatch",
        "typePolicyMatches",
        "countStable",
    ):
        assert type(begin.get(key)) is bool, key
    assert type(begin.get("changeCount")) is int and begin["changeCount"] >= 0, "changeCount"
    assert type(begin.get("changeCountAfter")) is int and begin["changeCountAfter"] >= 0, "changeCountAfter"
    assert begin["countStable"] == (begin["changeCount"] == begin["changeCountAfter"]), "Count stability"
    assert not begin["readbackPresent"] or begin["writeSucceeded"], "Readback skipped after failed write"
    assert not begin["exactCanaryMatch"] or begin["readbackPresent"], "Match requires presence"
    assert begin["readbackMatches"] == (
        begin["exactCanaryMatch"] and begin["typePolicyMatches"] and begin["countStable"]
    ), "Compound readback"
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
    states = [
        e
        for e in events
        if e["event"]
        in {"app.mouseDown.beforeRouting", "app.keyDown.beforeRouting", "mouseDown.enter", "mouseDown.exit"}
    ]
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
    routing = assess_mouse_routing(events)
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
        "mouseRouting": routing,
        "viewIdentities": sorted({e["view"] for e in events if "view" in e}),
        "classification": "observations only; inspect ordering and primary XCTest result",
    }


def assess_mouse_routing(events):
    """Validate observation shape; prediction and actual handler calls stay separate."""

    def numbers(data, keys):
        assert type(data) is dict, "Missing coordinate metadata"
        for key in keys:
            assert type(data.get(key)) in (int, float) and math.isfinite(data[key]), f"Invalid coordinate: {key}"
        return data

    def mouse(event):
        data = event.get("mouseEvent")
        assert type(data) is dict, "Missing mouse event metadata"
        assert type(data.get("windowID")) is str and data["windowID"], "Mouse window identity"
        for key in ("windowNumber", "eventNumber"):
            assert type(data.get(key)) is int, key
        numbers(data, ("timestampSeconds",))
        numbers(data.get("windowPoint"), ("x", "y"))
        return data

    clicks = [e for e in events if e["event"] == "app.mouseDown.beforeRouting"]
    assert clicks, "No predictive mouse observation"
    for index, event in enumerate(clicks):
        mouse(event)
        data = event.get("routing")
        assert type(data) is dict, "Missing routing metadata"
        if index:
            assert data == {"status": "already-observed", "queryPerformed": False}, "Repeated hitTest query"
    first = clicks[0]
    data = first["routing"]
    assert data.get("status") == "observed" and data.get("queryPerformed") is True, (
        f"Routing unavailable: {data.get('reason')}"
    )
    assert data.get("predictionOnly") is True, "hitTest must remain a prediction"
    assert data.get("coordinateSpace") == "event-window-base", "Geometry coordinate space"
    assert first["mouseEvent"]["windowNumber"] == first["windowNumber"], "Different event/note windows"
    for key in ("noteID", "scrollID", "clipID", "rootID", "hostID", "recipientID", "querySpaceID"):
        assert type(data.get(key)) is str and data[key], key
    for key in ("noteID", "scrollID", "clipID", "rootID"):
        assert data[key] != "nil", key
    assert data.get("hostIdentification") == "nearest-NSHostingView-class-prefix", "Host heuristic provenance"
    for key in ("hostSearchTruncated", "ancestryTruncated"):
        assert type(data.get(key)) is bool, key
    for key in ("noteVisibleWindow", "clipVisibleWindow"):
        bounds = numbers(data.get(key), ("x", "y", "width", "height"))
        assert bounds["width"] >= 0 and bounds["height"] >= 0, "Negative visible size"
    for key in ("noteLocalPoint", "queryPoint", "notePointRoundTripWindow", "queryPointRoundTripWindow"):
        point = numbers(data.get(key), ("x", "y"))
        if "RoundTrip" in key:
            assert all(
                abs(point[axis] - first["mouseEvent"]["windowPoint"][axis]) <= 0.000001 for axis in ("x", "y")
            ), "Coordinate round-trip mismatch"
    chain = data.get("ancestry")
    assert type(chain) is list and len(chain) <= 16, "Missing/oversized ancestry"
    assert (len(chain) == 0) == (data["recipientID"] == "nil"), "Nil recipient ancestry"
    assert not chain or chain[0].get("id") == data["recipientID"], "Recipient/ancestry mismatch"
    assert not data["ancestryTruncated"] or len(chain) == 16, "Invalid ancestry truncation"
    identities = []
    for node in chain:
        for key in ("id", "class"):
            assert type(node.get(key)) is str and node[key], key
        identities.append(node["id"])
        for role, identity in (
            ("Note", "noteID"),
            ("Scroll", "scrollID"),
            ("Clip", "clipID"),
            ("Host", "hostID"),
            ("Root", "rootID"),
        ):
            assert type(node.get("is" + role)) is bool and node["is" + role] == (node["id"] == data[identity]), (
                "Inconsistent ancestry role"
            )
    assert len(set(identities)) == len(identities), "Duplicate ancestry identity"
    entries = [e for e in events if e["event"] == "mouseDown.enter"]
    exits = [e for e in events if e["event"] == "mouseDown.exit"]
    assert len(entries) == len(exits), "Incomplete actual mouseDown handler"
    for entry, exit_event in zip(entries, exits):
        received = mouse(entry)
        assert mouse(exit_event) == received, "Handler event changed"
        assert entry["view"] == exit_event["view"], "Handler view changed"
        matching = [e for e in clicks if e["mouseEvent"] == received and e["view"] == entry["view"]]
        assert len(matching) == 1, "Handler missing unique pre-routing observation"
        assert matching[0]["sequence"] < entry["sequence"] < exit_event["sequence"], "Handler order"
        assert entry["stream"] == exit_event["stream"] == matching[0]["stream"], "Handler process/stream mismatch"
    return {
        "predictionOnly": True,
        "predictedRecipient": data["recipientID"],
        "ancestryTruncated": data["ancestryTruncated"],
        "actualHandlerEntries": len(entries),
        "actualHandlerExits": len(exits),
    }


UUID = r"[0-9A-Fa-f]{8}(?:-[0-9A-Fa-f]{4}){3}-[0-9A-Fa-f]{12}"


def receipt_location(raw):
    """Only the reported UUID directory in our temp root or exact XCTest container."""
    assert type(raw) is str and raw.startswith("/"), "Receipt must be an absolute path"
    source = Path(raw)
    assert str(source) == raw and ".." not in source.parts, f"Noncanonical receipt path: {raw}"
    assert re.fullmatch("caper-forward-observation-" + UUID, source.name), f"Invalid receipt directory: {raw}"
    roots = {
        Path(tempfile.gettempdir()).resolve(): "supervisor-temporary-root",
        Path.home()
        / "Library/Containers/chat.caper.appkit-development.tests.xctrunner/Data/tmp": "xctest-container-temporary-root",
    }
    assert source.parent in roots, f"Receipt parent outside allowed roots: {raw}"
    return source, roots[source.parent]


def open_directory(path):
    """Open each component without following symlinks, retaining the final directory FD."""
    assert path.is_absolute() and ".." not in path.parts, f"Invalid directory path: {path}"
    fd = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
    try:
        for component in path.parts[1:]:
            child = os.open(component, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            os.close(fd)
            fd = child
        return fd
    except BaseException:
        os.close(fd)
        raise


def copy_receipt(source, destination):
    fd = open_directory(source)
    try:
        files = sorted(name for name in os.listdir(fd) if name.startswith("events-"))
        assert 1 <= len(files) <= 2, f"Expected one or two event files at {source}; got {len(files)}"
        for name in files:
            assert re.fullmatch("events-" + UUID + r"\.jsonl", name), f"Invalid event filename at {source}: {name}"
            item = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=fd)
            with os.fdopen(item, "rb") as stream:
                info = os.fstat(stream.fileno())
                assert stat.S_ISREG(info.st_mode) and info.st_nlink == 1, (
                    f"Not a single-link regular file: {source / name}"
                )
                assert info.st_size <= 1_048_576, f"Oversized event file: {source / name}"
                data = stream.read(1_048_577)
                assert len(data) <= 1_048_576, f"Event file grew beyond bound: {source / name}"
            (destination / name).write_bytes(data)
    finally:
        os.close(fd)


def recover_console(paths, destination, record):
    """Explicit offline fallback; never rewrites the live collector's original outcome."""
    destination.mkdir()
    report = {"complete": False, "productAcceptance": False, "provenance": "retained-console-offline", "sources": []}
    events = []
    try:
        assert 1 <= len(paths) <= 2, "Expected explicit test and/or app console paths"
        for path in paths:
            report["check"] = "read bounded explicit console"
            report["path"] = str(path)
            parent = open_directory(path.parent)
            try:
                fd = os.open(path.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
            finally:
                os.close(parent)
            with os.fdopen(fd, "rb") as stream:
                assert stat.S_ISREG(os.fstat(stream.fileno()).st_mode), "Not a regular console"
                data = stream.read(16_777_217)
                assert len(data) <= 16_777_216, "Console exceeds 16 MiB bound"
            report["sources"].append({"path": str(path), "sha256": hashlib.sha256(data).hexdigest()})
            for number, line in enumerate(data.decode().splitlines(), 1):
                if "CAPER_FORWARD_OBSERVATION " in line:
                    assert len(events) < 514, "Console event bound exceeded"
                    events.append(
                        {
                            "source": str(path),
                            "line": number,
                            "event": json.loads(line.split("CAPER_FORWARD_OBSERVATION ", 1)[1]),
                        }
                    )
        report["check"] = "assess current metadata schema (legacy fields are not inferred)"
        report.update(assess([item["event"] for item in events]))
    except Exception as error:
        report["error"] = f"{type(error).__name__}: {error}"
    record(destination / "events.json", events)
    record(destination / "report.json", report)
    return report


def collect(output, record):
    report = {"complete": False, "productAcceptance": False, "check": "read test console receipt"}
    destination = output / "observations"
    destination.mkdir()
    try:
        test = output / "test"
        log = (test / "stdout.txt").read_text() + (test / "stderr.txt").read_text()
        assert "CAPER_FORWARD_OBSERVATION_IO_ERROR" not in log, "Observation writer reported an I/O error"
        records = []
        for line in log.splitlines():
            if "CAPER_FORWARD_OBSERVATION " in line:
                records.append(json.loads(line.split("CAPER_FORWARD_OBSERVATION ", 1)[1]))
        prepared = [e for e in records if e["event"] == "test.prepared"]
        assert len(prepared) == 1, "Missing/duplicate test observation directory"
        report["check"] = "validate reported receipt location"
        report["path"] = prepared[0]["directory"]
        source, provenance = receipt_location(prepared[0]["directory"])
        report["provenance"] = provenance
        report["check"] = "copy bounded no-symlink receipt files"
        copy_receipt(source, destination)
        events = []
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
        report["check"] = "assess receipt metadata"
        report.update(assess(events))
    except Exception as error:
        report["error"] = f"{type(error).__name__}: {error}"
    record(destination / "report.json", report)
    return report["complete"]
