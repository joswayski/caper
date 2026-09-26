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


def find(root: ET.Element, *, text: str | None = None, description: str | None = None, contains: str | None = None, resource_id: str | None = None) -> ET.Element | None:
    for node in nodes(root):
        if text is not None and node.get("text") == text:
            return node
        if description is not None and node.get("content-desc") == description:
            return node
        if contains is not None and contains in (node.get("text", "") + node.get("content-desc", "")):
            return node
        if resource_id is not None and node.get("resource-id") == resource_id:
            return node
    return None


def wait_for(*, text: str | None = None, description: str | None = None, contains: str | None = None, resource_id: str | None = None, seconds: int = 20) -> ET.Element:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        root = hierarchy()
        match = find(root, text=text, description=description, contains=contains, resource_id=resource_id)
        if match is not None:
            return root
        time.sleep(0.5)
    raise AssertionError(f"UI did not show text={text!r}, description={description!r}, contains={contains!r}, resource_id={resource_id!r}")


def center(node: ET.Element) -> tuple[int, int]:
    left, top, right, bottom = map(int, re.findall(r"\d+", node.attrib["bounds"]))
    return (left + right) // 2, (top + bottom) // 2


def tap(*, text: str | None = None, description: str | None = None, resource_id: str | None = None) -> None:
    root = wait_for(text=text, description=description, resource_id=resource_id)
    node = find(root, text=text, description=description, resource_id=resource_id)
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
    assert find(guest, description="Channel options") is None
    assert find(guest, description="Create channel") is None
    tap(description="Collapse channels")
    guest_collapsed = wait_for(description="Expand channels")
    assert find(guest_collapsed, text="general") is None
    assert find(guest_collapsed, contains="Message #general") is not None
    tap(description="Expand channels")
    wait_for(text="general")
    tap(text="Guest")
    login = capture("caper-android-login", "Come on in.")
    for required in ("WELCOME TO CAPER", "Email address", "Email me a code", "Join #general as a guest."):
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
    for required in ("Channels", "general", "design", "planning", "Members", "Maya"):
        assert find(desktop, contains=required) is not None, f"Populated shell is missing {required!r}"
    assert find(desktop, text="caper") is None, "The workspace must not have a web-style branding header"

    # Hide the list while reading a non-default channel: collapsing must not
    # silently select General, disconnect chat, or expose hidden row actions.
    tap(text="design")
    wait_for(contains="Message #design")
    tap(description="Channel options")
    options = capture("caper-android-channel-options", "Collapse channels")
    assert find(options, text="Create channel") is not None
    tap(text="Collapse channels")
    collapsed = capture("caper-android-channels-collapsed", "Expand channels")
    for hidden in ("general", "design", "planning"):
        assert find(collapsed, text=hidden) is None
    assert find(collapsed, description="Manage design") is None
    assert find(collapsed, contains="Message #design") is not None
    assert find(collapsed, text="Create channel") is None, "The action must close its menu"
    tap(description="Channel options")
    tap(text="Expand channels")
    wait_for(text="planning")
    tap(description="Collapse channels")
    wait_for(description="Expand channels")
    tap(description="Expand channels")
    wait_for(description="Manage design")
    tap(text="general")
    wait_for(contains="Message #general")

    tap(description="Create space")
    create_space = capture("caper-android-create-space", "Space name")
    assert find(create_space, contains="A space keeps") is None
    tap(description="Close")
    tap(description="Create channel")
    create_channel = capture("caper-android-create-channel", "Channel name")
    assert find(create_channel, contains="Add a conversation") is None
    assert find(create_channel, text="Anyone in Fixture Studio can view or join this channel.") is not None
    tap(description="Close")
    tap(description="Channel options")
    tap(text="Create channel")
    wait_for(text="Channel name")
    tap(description="Close")
    assert find(hierarchy(), text="Collapse channels") is None, "Opening a dialog must close the menu"

    # Inspect prejoin audio controls without starting capture or playback.
    tap(description="Audio and account settings")
    settings = capture("caper-android-audio-settings", "Audio test")
    for label in ("Caper sound effects", "Audio test", "Log out"):
        assert find(settings, text=label) is not None, f"Missing audio settings entry: {label}"
    assert find(settings, text="Connection details") is None, "Connection details are only offered while connected"
    tap(text="Audio test")
    audio = capture("caper-android-audio-prejoin", "Only you can hear these tests.")
    for label in ("Test microphone volume", "Voice processing", "Test speaker volume"):
        assert find(audio, description=label) is not None, f"Missing labeled slider: {label}"
    for label in ("Microphone volume", "Speaker volume", "Voice enhancement", "Test speakers", "Test microphone", "Try your microphone", "Input level"):
        assert find(audio, text=label) is not None, f"Missing audio test copy: {label}"
    for removed in ("DPDFNet", "RNNoise", "voice EQ", "Android system settings", "communication routes", "Join voice", "Signed in as", "Mic Test"):
        assert find(audio, contains=removed) is None, f"Unexpected explanatory copy: {removed}"
    assert find(audio, text="Choose speakers in system settings.") is not None, "Do not show a routing section without routes"
    assert find(audio, text="Connection details") is None
    assert find(audio, text="25%") is not None
    output = find(audio, description="Test speaker volume")
    left, top, right, bottom = map(int, re.findall(r"\d+", output.attrib["bounds"]))
    # Compose's accessibility bounds include padding outside the touch track.
    # Drag the current 100% thumb past the minimum instead of tapping padding;
    # sampled motion events may stop short of the final pointer-up coordinate.
    adb("shell", "input", "swipe", str((left + right) // 2), str((top + bottom) // 2),
        str(left - (right - left) // 4), str((top + bottom) // 2), "500")
    wait_for(text="0%")
    tap(description="Close")
    tap(description="Audio and account settings")
    tap(text="Audio test")
    zero = capture("caper-android-audio-prejoin-zero-output", "0%")
    assert find(zero, text="Test microphone") is not None
    tap(description="Close")

    tap(description="Edit profile")
    enter_first_field("ab")
    invalid = hierarchy()
    # Compose exposes the label separately; disabled belongs to its action node.
    parents = {child: parent for parent in invalid.iter() for child in parent}
    submit = find(invalid, text="Save profile")
    while submit is not None and submit.get("clickable") != "true":
        submit = parents.get(submit)
    assert submit is not None and submit.get("enabled") == "false"
    enter_first_field("fixture_owner")
    fixture({"failure": {"path": "/api/account/profile", "method": "POST", "status": 503,
                         "error": "TEST FIXTURE: profile save temporarily unavailable."}})
    tap(text="Save profile")
    profile_error = capture("caper-android-profile-error", "Your profile could not be saved. Please try again.")
    assert find(profile_error, text="fixture_owner") is not None, "Rejected save must retain the edit"
    assert find(profile_error, text="Save profile") is not None, "Rejected save must keep the form open"
    assert sum(1 for node in nodes(profile_error) if node.get("text") == "Edit profile") == 1
    tap(text="Save profile")
    wait_for(description="Manage planning")

    tap(description="Manage planning")
    overview = capture("caper-android-channel-settings", "Overview")
    for required in ("Private channel", "Only you and the people you add can view or join.", "Members", "Exact username", "Delete channel"):
        assert find(overview, contains=required) is not None, f"Channel overview is missing {required!r}"
    assert find(overview, text="Add") is not None
    for removed in ("Private channel access", "Existing username", "Grant"):
        assert find(overview, contains=removed) is None, f"Unexpected member-management copy: {removed}"
    tap(description="Close")

    viewport(390, 844)
    launch()
    wait_for(text="Fixture Owner")
    narrow = capture("caper-android-narrow", "Browse")
    assert find(narrow, text="caper") is None
    assert find(narrow, contains="Message #general") is not None
    menu = find(narrow, text="Browse")
    channel = find(narrow, text="# general")
    assert menu is not None and channel is not None, "Narrow navigation shows the web's Menu + Browse toggle"
    assert center(menu)[0] < center(channel)[0], "Browse must precede the channel title, as on the web"
    send = find(narrow, description="Send")
    assert send is not None and send.get("enabled") == "false", "Empty composer must not send"
    tap(text="Browse")
    tap(description="Fixture Studio")
    wait_for(text="Fixture Studio")
    browse = capture("caper-android-browse", "Fixture Studio")
    assert find(browse, contains="Channels") is not None
    tap(description="Collapse channels")
    wait_for(description="Expand channels")
    tap(description="Close navigation")
    wait_for(contains="Message #general")
    tap(text="Browse")
    collapsed_browse = capture("caper-android-browse-collapsed", "Expand channels")
    assert find(collapsed_browse, text="planning") is None, "Browse must retain the collapsed state"
    tap(description="Channel options")
    capture("caper-android-channel-options-narrow", "Expand channels")
    adb("shell", "input", "keyevent", "KEYCODE_BACK")
    assert find(hierarchy(), text="Expand channels") is None, "Back must dismiss the options menu"
    tap(description="Expand channels")
    wait_for(text="planning")
    tap(description="Audio and account settings")
    tap(text="Audio test")
    audio_narrow = capture("caper-android-audio-prejoin-narrow", "Only you can hear these tests.")
    assert find(audio_narrow, description="Test microphone volume") is not None
    tap(description="Close")

    # Spectator snapshots are fixture-only; the media join endpoint remains 503.
    # Read #general while #design has occupants, without granting microphone access.
    fixture({"media": {"channelId": "chan00000002", "participants": [
        {"id": f"spectator00{index}", "name": f"Fixture Voice {index}", "muted": False, "deafened": False}
        for index in range(1, 5)
    ]}})
    wait_for(description="Close navigation")  # Audio preferences closed back into Browse.
    roster = capture("caper-android-voice-roster-expanded", "4 in voice in design. Hide who is in voice")
    assert find(roster, text="Fixture Voice 1") is not None
    assert find(roster, text="Fixture Voice 4") is not None
    assert find(roster, text="+1") is not None, "Avatar stack must cap at three faces"
    assert find(roster, text="4 in voice in design") is None, "Voice count belongs in accessibility, not visible copy"
    assert find(roster, description="Audio controls for Fixture Voice 1") is None, "Spectators must not get local audio controls"
    assert find(roster, text="User volume") is None
    assert find(roster, text="Mute") is None
    assert find(roster, contains="Message #general") is None  # conversation is behind narrow navigation
    tap(description="4 in voice in design. Hide who is in voice")
    collapsed = capture("caper-android-voice-roster-collapsed", "4 in voice in design. Show who is in voice")
    assert find(collapsed, text="Fixture Voice 1") is None, "Collapsed roster still exposed occupants"
    tap(description="4 in voice in design. Show who is in voice")
    wait_for(text="Fixture Voice 1")
    fixture({"mediaAccessDenied": {"channelId": "chan00000002"}})
    try:
        gone = hierarchy()
        for _ in range(25):
            gone = hierarchy()
            if find(gone, text="Fixture Voice 1") is None:
                break
            time.sleep(0.2)
        assert find(gone, text="Fixture Voice 1") is None, "Revoked spectator remained visible"
    finally:
        fixture({"mediaAccessDenied": {"channelId": "chan00000002", "denied": False}})
    tap(description="Close navigation")
    assert find(hierarchy(), contains="Message #general") is not None, "Spectator updates changed selected chat"
    tap(text="Browse")

    # Run after parity captures so the stable seeded reference conversation is
    # unchanged. This crosses the real Compose input -> HTTP send -> gateway UI
    # path and independently checks fixture persistence.
    tap(text="general")
    wait_for(contains="Message #general")
    sent_text = "Android fixture send check"
    enter_first_field(sent_text)
    ready = capture("caper-android-send-ready", sent_text)
    send = find(ready, description="Send")
    assert send is not None and send.get("enabled") == "true", "Draft must enable the send button"
    composer = next(node for node in nodes(ready) if node.get("class") == "android.widget.EditText")
    composer_right = list(map(int, re.findall(r"\d+", composer.attrib["bounds"])))[2]
    send_left = list(map(int, re.findall(r"\d+", send.attrib["bounds"])))[0]
    assert send_left > composer_right, "Send must be a separate button to the right of the composer"
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

    # Exercise the actual default voice entry and Android permission controller.
    # Only notifications are pre-granted; microphone denial and approval happen
    # through the system UI. The loopback fixture rejects join, never fakes audio.
    # Web joins voice from the channel list only; the chat header has no Join.
    assert find(delivered, text="Join") is None, "Chat header must not duplicate the channel-list Join"
    tap(text="Browse")
    join = "Join voice in #general"
    voice_ready = capture("caper-android-voice-ready", join)
    assert find(voice_ready, description=join) is not None
    adb("shell", "pm", "grant", PACKAGE, "android.permission.POST_NOTIFICATIONS")
    tap(description=join)
    deny = "com.android.permissioncontroller:id/permission_deny_button"
    wait_for(resource_id=deny)
    capture("caper-android-voice-permission", "Caper")
    tap(resource_id=deny)
    denied = capture("caper-android-voice-permission-denied", "Microphone permission is required")
    assert find(denied, description=join) is not None and find(denied, description="Leave voice") is None
    tap(description=join)
    tap(resource_id="com.android.permissioncontroller:id/permission_allow_foreground_only_button")
    failed = capture("caper-android-voice-fixture-error", "TEST FIXTURE: no real media engine or SFU is connected.")
    assert find(failed, description=join) is not None and find(failed, description="Leave voice") is None
    assert find(failed, contains="Microphone permission is required") is None
    assert find(failed, description="Dismiss voice error") is not None, "Web shows voice errors in the dock"
    tap(description="Close navigation")
    chat = wait_for(contains="Message #general")
    assert find(chat, contains="TEST FIXTURE: no real media engine or SFU is connected.") is not None, \
        "Phones keep the voice error visible outside Browse"
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
