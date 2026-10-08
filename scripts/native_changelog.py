#!/usr/bin/env python3
"""Carry release history into the rolling signed native update manifest."""

import argparse
import json
from pathlib import Path
import subprocess

MAX_MANIFEST_BYTES = 64 * 1024  # Match the updater and API metadata cache.


def git(*args):
    return subprocess.check_output(["git", *args], text=True).strip()


def encode(manifest):
    return (json.dumps(manifest, ensure_ascii=False, separators=(",", ":")) + "\n").encode()


def with_changelog(latest, previous):
    build = latest["build"]
    commit = latest["commit"]
    history = []
    oldest = 0
    if previous:
        if previous["build"] > build:
            raise ValueError("cannot publish a lower build number")
        history = previous.get("changelog", [])
        oldest = previous.get("changelog_from_build", previous["build"])
        if previous["build"] == build:
            if previous["commit"] != commit:
                raise ValueError("a published build number cannot change commits")
            # A workflow rerun must not replace the original notes with an empty range.
            return fit({**latest, "notes": previous["notes"], "changelog": history,
                        "changelog_from_build": oldest})
        ancestor = subprocess.run(
            ["git", "merge-base", "--is-ancestor", previous["commit"], commit],
            check=False,
        ).returncode
        if ancestor not in (0, 1):
            raise ValueError("could not resolve the previous release commit")
        if ancestor == 1:
            notes = f"• Rebuilt earlier source {commit[:7]}; replaces the previous release."
        else:
            subjects = git("log", "--first-parent", "--format=%s", f'{previous["commit"]}..{commit}')
            notes = "\n".join(f"• {subject}" for subject in subjects.splitlines())
    else:
        subjects = git("log", "--first-parent", "--format=%s", commit)
        notes = "\n".join(f"• {subject}" for subject in subjects.splitlines())
    if not notes:
        notes = "• Rebuilt the same source; no new code changes."
    entry = {"build": build, "version": latest["version"], "notes": notes}
    history = [entry, *sorted(history, key=lambda entry: entry["build"], reverse=True)]
    return fit({**latest, "notes": notes, "changelog": history, "changelog_from_build": oldest})


def fit(manifest):
    # Never cut a release's notes midway or silently imply the retained list is complete.
    while len(encode(manifest)) > MAX_MANIFEST_BYTES:
        if len(manifest["changelog"]) <= 1:
            raise ValueError("latest release notes exceed the signed manifest size limit")
        removed = manifest["changelog"].pop()
        manifest["changelog_from_build"] = max(manifest["changelog_from_build"], removed["build"])
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--previous", type=Path)
    args = parser.parse_args()
    latest = json.loads(args.manifest.read_bytes())
    previous = json.loads(args.previous.read_bytes()) if args.previous else None
    args.manifest.write_bytes(encode(with_changelog(latest, previous)))


if __name__ == "__main__":
    main()
