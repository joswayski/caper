"""Fixture-driven Android UI parity smoke test for one fresh API 36 emulator."""

from __future__ import annotations

import json
import os
from pathlib import Path
import re
import subprocess
import time
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
    adb("shell", "uiautomator", "dump", "/sdcard/caper-ui.xml", timeout=30)
    xml = adb("exec-out", "cat", "/sdcard/caper-ui.xml", timeout=30)
    return ET.fromstring(xml)


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
    adb("shell", "input", "text", value)
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
    desktop = capture("caper-android-populated-desktop", "Fixture Studio")
    for required in ("CHANNELS", "general", "design", "planning", "MEMBERS — 3"):
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
    print(f"PASS: fixture parity captures and interactions written to {OUTPUT}")


if __name__ == "__main__":
    main()
