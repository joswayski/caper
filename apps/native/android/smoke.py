"""Check sign-in on one fresh, booted emulator. Never requests a login code."""

from pathlib import Path
import re
import subprocess
import time
import xml.etree.ElementTree as ET


DIST = Path(__file__).resolve().parent / "dist"
OUTPUT = DIST / "ui"


def adb(*args: str) -> str:
    return subprocess.check_output(["adb", *args], text=True, timeout=120)


def capture(name: str) -> ET.Element:
    adb("shell", "uiautomator", "dump", "/sdcard/caper-ui.xml")
    path = OUTPUT / f"{name}.xml"
    adb("pull", "/sdcard/caper-ui.xml", str(path))
    with (OUTPUT / f"{name}.png").open("wb") as image:
        subprocess.run(["adb", "exec-out", "screencap", "-p"], stdout=image, check=True, timeout=30)
    return ET.parse(path).getroot()


def send_button(root: ET.Element) -> ET.Element:
    return next(
        node for node in root.iter("node")
        if node.get("clickable") == "true"
        and any(child.get("text") == "Send code" for child in node.iter("node"))
    )


def main() -> None:
    OUTPUT.mkdir(parents=True, exist_ok=True)
    adb("install", str(DIST / "Caper-android-debug.apk"))
    adb("shell", "am", "start", "-W", "-n", "chat.caper.android.debug/chat.caper.android.MainActivity")
    time.sleep(5)
    adb("shell", "pidof", "chat.caper.android.debug")
    empty = capture("caper-android-sign-in")
    assert any(node.get("text") == "A quieter place to talk" for node in empty.iter("node"))
    assert send_button(empty).get("enabled") == "false", "Empty email must not enable sending"

    field = empty.find('.//node[@class="android.widget.EditText"]')
    assert field is not None, "Sign-in email field is missing"
    left, top, right, bottom = map(int, re.findall(r"\d+", field.attrib["bounds"]))
    adb("shell", "input", "tap", str((left + right) // 2), str((top + bottom) // 2))
    time.sleep(1)
    adb("shell", "input", "text", "preview@example.invalid")
    adb("shell", "input", "keyevent", "KEYCODE_BACK")
    time.sleep(1)
    entered = capture("caper-android-email-entered")
    assert any(node.get("text") == "preview@example.invalid" for node in entered.iter("node"))
    assert send_button(entered).get("enabled") == "true", "Entered email should enable sending"
    print("PASS: sign-in renders; empty/entered email disable/enable Send code. No email was sent.")


if __name__ == "__main__":
    main()
