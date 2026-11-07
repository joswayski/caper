#!/usr/bin/python3
"""AT-SPI smoke test for caper-linux: the real app on a private D-Bus and
accessibility bus under Xvfb, against the parity fixture
(scripts/native-parity-fixture.mjs).

It signs in with the fixture's account, waits for #general's messages,
checks that each row's toolbar, built on hover, is named for its author
("Reply to Maya"), and sends a message that must show up in the list. On GTK 4.12 or later, message
rows must also be named "author, time, text"; GTK 4.8 can't name them.

Run under `xvfb-run -a dbus-run-session --` with `--binary` pointing at a
built `caper-linux` and `--api-url` at the fixture. Needs python3-gi,
gir1.2-atspi-2.0, xdotool and gnome-keyring (the app keeps its session in
the Secret Service).
"""

import argparse
import os
import re
import subprocess
import sys
import tempfile
import time

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi, GLib  # noqa: E402

SENT = "Smoke check from caper-linux"


def wait(predicate, what, timeout=30):
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            value = predicate()
        except GLib.Error:
            value = None
        if value:
            return value
        time.sleep(0.3)
    raise AssertionError(f"timed out waiting for {what}")


def application(pid):
    desktop = Atspi.get_desktop(0)
    for index in range(desktop.get_child_count()):
        child = desktop.get_child_at_index(index)
        try:
            if child and child.get_process_id() == pid:
                return child
        except GLib.Error:
            continue
    return None


def nodes(root):
    """Every visible node; rows rebinding mid-walk are skipped."""
    stack = [root]
    while stack:
        node = stack.pop()
        try:
            # The application itself has no visibility state.
            if node != root and not node.get_state_set().contains(Atspi.StateType.VISIBLE):
                continue
            yield node
            stack.extend(
                child
                for child in (node.get_child_at_index(i) for i in range(node.get_child_count()))
                if child is not None
            )
        except GLib.Error:
            continue


def find(root, role=None, name=None, contains=None):
    for node in nodes(root):
        if role and node.get_role_name() != role:
            continue
        label = node.get_name() or ""
        if name is not None and label != name:
            continue
        if contains is not None and contains not in label:
            continue
        return node
    return None


def click(node):
    # Window coordinates: GTK 4.14 reports screen ones as 0,0, and with no
    # window manager the window sits at the origin.
    extents = node.get_extents(Atspi.CoordType.WINDOW)
    x, y = extents.x + extents.width // 2, extents.y + extents.height // 2
    subprocess.run(["xdotool", "mousemove", str(x), str(y), "click", "1"], check=True)
    time.sleep(0.3)


def hover(node):
    extents = node.get_extents(Atspi.CoordType.WINDOW)
    x, y = extents.x + extents.width // 2, extents.y + extents.height // 2
    subprocess.run(["xdotool", "mousemove", str(x), str(y)], check=True)


def toolbar(row):
    """The names of `row`'s buttons, once its toolbar is built."""
    buttons = [node.get_name() or "" for node in nodes(row) if node.get_role_name() == "push button"]
    return buttons if any(name.startswith("Reply to ") for name in buttons) else None


def type_text(text, enter=True):
    subprocess.run(["xdotool", "type", "--delay", "20", text], check=True)
    if enter:
        subprocess.run(["xdotool", "key", "Return"], check=True)


def placed(node):
    """The node, once laid out on screen (a stack's hidden page is 0×0)."""
    return node if node and node.get_extents(Atspi.CoordType.WINDOW).width > 0 else None


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True)
    parser.add_argument("--api-url", default="http://127.0.0.1:3911")
    args = parser.parse_args()

    home = tempfile.mkdtemp(prefix="caper-linux-smoke-")
    env = dict(
        os.environ,
        HOME=home,
        XDG_CONFIG_HOME=f"{home}/config",
        XDG_DATA_HOME=f"{home}/data",
        XDG_CACHE_HOME=f"{home}/cache",
        GDK_BACKEND="x11",
        GSK_RENDERER="cairo",
    )
    # An unlocked Secret Service for the session the app saves.
    keyring = subprocess.run(
        ["gnome-keyring-daemon", "--unlock", "--components=secrets"],
        input="smoke",
        capture_output=True,
        text=True,
        check=True,
        env=env,
    )
    for line in keyring.stdout.splitlines():
        key, _, value = line.partition("=")
        if value:
            env[key] = value

    app_process = subprocess.Popen([args.binary, "--api-url", args.api_url], env=env)
    try:
        app = wait(lambda: application(app_process.pid), "caper-linux on the accessibility bus")
        version = app.get_toolkit_version() or "0"
        print(f"GTK {version}")
        major, minor = (int(part) for part in version.split(".")[:2])

        email = wait(lambda: placed(find(app, role="text", name="Email address")), "the email field")
        click(email)
        type_text("fixture@example.test")
        code = wait(lambda: placed(find(app, role="text", name="Sign-in code")), "the code field")
        click(code)
        type_text("ABC234")

        messages = wait(
            lambda: (lambda node: node if node and node.get_child_count() > 0 else None)(find(app, name="Messages")),
            "#general's messages",
        )
        rows = [node for node in nodes(messages) if node.get_role_name() == "list item"]
        print(f"{len(rows)} message rows showing")
        assert rows, "no message rows"
        if (major, minor) >= (4, 12):
            names = [row.get_name() or "" for row in rows]
            print("first row:", names[0])
            unnamed = [name for name in names if not re.search(r", \d{1,2}:\d{2}", name)]
            # The toolbar a row builds on hover or focus, listed for screen readers.
            assert "Reply in thread" in (rows[0].get_description() or ""), rows[0].get_description()
            assert not unnamed, f"rows without 'author, time, text' names: {unnamed[:3]}"
        # A row builds its toolbar when the pointer first enters it. Rows are
        # recycled, so a stale name would be another author's.
        for row in rows:
            hover(row)
            buttons = wait(lambda row=row: toolbar(row), f"the toolbar of {row.get_name()!r}", timeout=5)
            author = next(name for name in buttons if name.startswith("Reply to ")).removeprefix("Reply to ")
            print(f"toolbar for {author}")
            assert f"Message actions for {author}" in buttons, f"toolbar names: {buttons}"
            if (major, minor) >= (4, 12):
                assert (row.get_name() or "").startswith(f"{author}, "), (row.get_name(), author)

        composer = wait(lambda: placed(find(app, role="text", name="Message")), "the composer")
        click(composer)
        type_text(SENT)
        try:
            wait(lambda: find(messages, contains=SENT), "the sent message in the list")
        except AssertionError:
            for node in nodes(messages):
                print(node.get_role_name(), repr(node.get_name()))
            raise
        print("sent message showed")
    finally:
        app_process.terminate()
        try:
            app_process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            app_process.kill()
    print("caper-linux smoke passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
