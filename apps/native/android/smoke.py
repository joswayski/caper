"""Fixture-driven Android UI parity smoke test for one fresh API 36 emulator."""

from __future__ import annotations

import json
import os
from pathlib import Path
import re
import subprocess
import time
import traceback
import urllib.request
import xml.etree.ElementTree as ET


ROOT = Path(__file__).resolve().parent
DIST = ROOT / "dist"
OUTPUT = DIST / "ui"
APK = Path(os.environ.get("CAPER_ANDROID_FIXTURE_APK", DIST / "Caper-android-fixture-debug.apk"))
PACKAGE = "chat.caper.android.debug"
ACTIVITY = f"{PACKAGE}/chat.caper.android.MainActivity"


def adb(*args: str, timeout: int = 120) -> str:
    return subprocess.check_output(["adb", *args], text=True, timeout=timeout).strip()


def fixture(body: dict) -> None:
    request = urllib.request.Request(
        "http://127.0.0.1:3001/__fixture/control",
        data=json.dumps(body).encode(),
        headers={"content-type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=5) as response:
        assert response.status == 200


def hierarchy() -> ET.Element:
    """Read a fresh hierarchy, tolerating transient uiautomator/animation failures.

    `uiautomator dump` writes diagnostics to stdout and can leave an empty or
    stale destination file when the window is changing. Never parse that
    command output as XML, and preserve enough evidence to debug a final CI
    failure rather than surfacing an opaque ElementTree error.
    """
    attempts: list[str] = []
    raw = b""
    for attempt in range(1, 5):
        subprocess.run(
            ["adb", "shell", "rm", "-f", "/sdcard/caper-ui.xml"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=30,
            check=False,
        )
        dumped = subprocess.run(
            ["adb", "shell", "uiautomator", "dump", "--compressed", "/sdcard/caper-ui.xml"],
            capture_output=True,
            timeout=30,
            check=False,
        )
        fetched = subprocess.run(
            ["adb", "exec-out", "cat", "/sdcard/caper-ui.xml"],
            capture_output=True,
            timeout=30,
            check=False,
        )
        raw = fetched.stdout
        start = raw.find(b"<hierarchy")
        end = raw.rfind(b"</hierarchy>")
        attempts.append(
            f"attempt {attempt}: dump={dumped.returncode} cat={fetched.returncode} "
            f"bytes={len(raw)} stdout={dumped.stdout.decode(errors='replace').strip()!r} "
            f"stderr={dumped.stderr.decode(errors='replace').strip()!r}"
        )
        if dumped.returncode == 0 and fetched.returncode == 0 and start >= 0 and end >= start:
            try:
                return ET.fromstring(raw[start : end + len(b"</hierarchy>")])
            except ET.ParseError as error:
                attempts[-1] += f" parse={error}"
        time.sleep(0.5 * attempt)

    OUTPUT.mkdir(parents=True, exist_ok=True)
    (OUTPUT / "hierarchy-failure.txt").write_text("\n".join(attempts) + "\n")
    (OUTPUT / "hierarchy-failure.xml").write_bytes(raw)
    with (OUTPUT / "hierarchy-failure.png").open("wb") as image:
        subprocess.run(["adb", "exec-out", "screencap", "-p"], stdout=image, check=False, timeout=30)
    raise AssertionError(
        f"Unable to acquire Android UI hierarchy after 4 attempts; diagnostics written to {OUTPUT}"
    )


def nodes(root: ET.Element):
    return root.iter("node")


def find(root: ET.Element, *, text: str | None = None, description: str | None = None, contains: str | None = None) -> ET.Element | None:
    for node in nodes(root):
        if text is not None and node.get("text") == text:
            return node
        if description is not None and node.get("content-desc") == description:
            return node
        if contains is not None and contains in (node.get("text", "") + node.get("content-desc", "")):
            return node
    return None


def wait_for(*, text: str | None = None, description: str | None = None, contains: str | None = None, seconds: int = 20) -> ET.Element:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        root = hierarchy()
        match = find(root, text=text, description=description, contains=contains)
        if match is not None:
            return root
        time.sleep(0.5)
    raise AssertionError(f"UI did not show text={text!r}, description={description!r}, contains={contains!r}")


def center(node: ET.Element) -> tuple[int, int]:
    left, top, right, bottom = map(int, re.findall(r"\d+", node.attrib["bounds"]))
    return (left + right) // 2, (top + bottom) // 2


def tap(*, text: str | None = None, description: str | None = None) -> None:
    root = wait_for(text=text, description=description)
    node = find(root, text=text, description=description)
    assert node is not None
    x, y = center(node)
    adb("shell", "input", "tap", str(x), str(y))


def enter_first_field(value: str) -> None:
    root = hierarchy()
    field = next((node for node in nodes(root) if node.get("class") == "android.widget.EditText"), None)
    assert field is not None, "Expected an editable field"
    x, y = center(field)
    adb("shell", "input", "tap", str(x), str(y))
    deadline = time.monotonic() + 10
    focused: ET.Element | None = None
    while time.monotonic() < deadline:
        current = hierarchy()
        focused = next(
            (
                node for node in nodes(current)
                if node.get("class") == "android.widget.EditText" and node.get("focused") == "true"
            ),
            None,
        )
        if focused is not None:
            break
        time.sleep(0.25)
    assert focused is not None, "Editable field did not receive focus"

    # API 36's input command supports key combinations. Explicitly clear the
    # focused editor and wait briefly for the IME connection before typing;
    # otherwise its startup can consume the first keystrokes on CI emulators.
    adb("shell", "input", "keycombination", "KEYCODE_CTRL_LEFT", "KEYCODE_A")
    adb("shell", "input", "keyevent", "KEYCODE_DEL")
    time.sleep(0.5)
    # adb input uses %s as its space escape. Arguments are passed without a
    # shell, so characters such as @ must remain literal.
    adb("shell", "input", "text", value.replace(" ", "%s"))

    observed = ""
    deadline = time.monotonic() + 8
    while time.monotonic() < deadline:
        current = hierarchy()
        editor = next((node for node in nodes(current) if node.get("class") == "android.widget.EditText" and node.get("focused") == "true"), None)
        observed = editor.get("text", "") if editor is not None else ""
        if observed == value:
            break
        time.sleep(0.25)
    assert observed == value, f"Editable field value mismatch: expected {value!r}, observed {observed!r}"
    adb("shell", "input", "keyevent", "KEYCODE_BACK")


def capture(name: str, expected: str) -> ET.Element:
    root = wait_for(contains=expected)
    OUTPUT.mkdir(parents=True, exist_ok=True)
    ET.ElementTree(root).write(OUTPUT / f"{name}.xml", encoding="unicode")
    with (OUTPUT / f"{name}.png").open("wb") as image:
        subprocess.run(["adb", "exec-out", "screencap", "-p"], stdout=image, check=True, timeout=30)
    return root


def viewport(width_dp: int, height_dp: int) -> None:
    # DPR 2, matching the supplied Chromium references.
    adb("shell", "wm", "size", f"{width_dp * 2}x{height_dp * 2}")
    adb("shell", "wm", "density", "320")


def launch() -> None:
    adb("shell", "am", "force-stop", PACKAGE)
    adb("shell", "am", "start", "-W", "-n", ACTIVITY)


def preserve_failure_artifacts(error: BaseException) -> None:
    """Best-effort bounded evidence for failures before or between named captures."""
    OUTPUT.mkdir(parents=True, exist_ok=True)
    (OUTPUT / "final-failure.txt").write_text(
        "".join(traceback.format_exception(type(error), error, error.__traceback__)),
    )
    with (OUTPUT / "final-failure.png").open("wb") as image:
        subprocess.run(
            ["adb", "exec-out", "screencap", "-p"],
            stdout=image,
            stderr=subprocess.DEVNULL,
            timeout=30,
            check=False,
        )
    try:
        ET.ElementTree(hierarchy()).write(OUTPUT / "final-failure.xml", encoding="unicode")
    except Exception as hierarchy_error:
        with (OUTPUT / "final-failure-hierarchy.txt").open("a") as details:
            details.write(f"{type(hierarchy_error).__name__}: {hierarchy_error}\n")

    pid = subprocess.run(
        ["adb", "shell", "pidof", "-s", PACKAGE],
        capture_output=True,
        text=True,
        timeout=15,
        check=False,
    ).stdout.strip()
    if pid.isdigit():
        logcat = subprocess.run(
            ["adb", "logcat", "-d", "-t", "400", "--pid", pid],
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )
        (OUTPUT / "final-failure-logcat.txt").write_text(
            logcat.stdout[-256_000:] + ("\nSTDERR:\n" + logcat.stderr[-16_000:] if logcat.stderr else ""),
        )
    else:
        (OUTPUT / "final-failure-logcat.txt").write_text("App process was not running; no app-scoped logcat available.\n")


def main() -> None:
    if not APK.is_file():
        raise SystemExit(f"Missing fixture APK: {APK}. Run ./fixture-build.sh first.")
    urllib.request.urlopen("http://127.0.0.1:3001/health", timeout=5).close()
    fixture({"reset": True})
    adb("reverse", "tcp:3001", "tcp:3001")
    viewport(1440, 900)
    adb("install", "-r", str(APK))
    adb("shell", "pm", "clear", PACKAGE)
    launch()

    guest = capture("caper-android-guest-populated-desktop", "TEST FIXTURE")
    assert find(guest, contains="The same conversation") is not None
    tap(text="Guest")
    login = capture("caper-android-login", "Come on in.")
    for required in ("WELCOME TO CAPER", "Email address", "Email me a code", "Join general as a guest."):
        assert find(login, text=required) is not None, f"Login is missing {required!r}"

    fixture({"failure": {"path": "/api/auth/email/request", "method": "POST", "status": 503}})
    enter_first_field("fixture@example.test")
    tap(text="Email me a code")
    error = capture("caper-android-login-error", "temporarily unavailable")
    assert find(error, text="fixture@example.test") is not None
    tap(text="Email me a code")
    wait_for(text="Check your email.")
    enter_first_field("ABC234")
    tap(text="Continue")
    wait_for(description="Fixture Studio")
    tap(description="Fixture Studio")
    wait_for(text="design")
    desktop = capture("caper-android-populated-desktop", "Fixture Studio")
    for required in ("CHANNELS", "general", "design", "planning", "Members", "Maya"):
        assert find(desktop, contains=required) is not None, f"Populated shell is missing {required!r}"

    tap(description="Manage planning")
    overview = capture("caper-android-channel-settings", "Overview")
    for required in ("Private channel", "Only you and the people you add can view or join.", "Delete channel"):
        assert find(overview, contains=required) is not None, f"Channel overview is missing {required!r}"
    tap(description="Close")

    viewport(390, 844)
    launch()
    narrow = capture("caper-android-narrow", "Browse")
    assert find(narrow, contains="Message #general") is not None
    tap(text="Browse")
    tap(description="Fixture Studio")
    wait_for(text="Fixture Studio")
    browse = capture("caper-android-browse", "Fixture Studio")
    assert find(browse, text="CHANNELS") is not None

    # Run after parity captures so the stable seeded reference conversation is
    # unchanged. This crosses the real Compose input -> HTTP send -> gateway UI
    # path and independently checks fixture persistence.
    tap(text="general")
    wait_for(contains="Message #general")
    sent_text = "Android fixture send check"
    enter_first_field(sent_text)
    tap(description="Send")
    delivered = wait_for(text=sent_text)
    assert sum(1 for node in nodes(delivered) if node.get("text") == sent_text) == 1, "Sent message rendered more than once"
    composer = next((node for node in nodes(delivered) if node.get("class") == "android.widget.EditText"), None)
    assert composer is not None and composer.get("text", "") == "", "Composer did not clear after confirmed send"

    history_request = urllib.request.Request(
        "http://127.0.0.1:3001/api/chat/channels/chan00000001/messages",
        headers={"Authorization": "Bearer fixture-owner-token"},
    )
    with urllib.request.urlopen(history_request, timeout=5) as response:
        history = json.load(response)
    matching = [message for message in history["messages"] if message["content"]["text"] == sent_text]
    assert len(matching) == 1, "Fixture history did not contain exactly one sent message"
    assert matching[0]["author"] == {"id": "owner0000001", "name": "Fixture Owner", "isGuest": False}
    assert matching[0]["channelId"] == "chan00000001" and matching[0]["content"]["version"] == 1
    print(f"PASS: fixture parity captures and interactions written to {OUTPUT}")


if __name__ == "__main__":
    try:
        main()
    except BaseException as error:
        try:
            preserve_failure_artifacts(error)
        except Exception as artifact_error:
            print(f"WARNING: could not preserve all failure artifacts: {artifact_error}")
        raise
