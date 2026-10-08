#!/usr/bin/env python3
"""Carry release history into the rolling signed native update manifest."""

import argparse
import json
from pathlib import Path
import re
import subprocess

MAX_MANIFEST_BYTES = 64 * 1024  # Match the updater and API metadata cache.
NO_PRODUCT_CHANGES = "• No user-facing changes in this build."
PRODUCT_ROOTS = (
    "apps/api/src/",
    "apps/api/migrations/",
    "apps/web/src/",
    "apps/web/public/",
    "apps/native/android/app/src/main/",
    "apps/native/apple/Sources/",
    "apps/native/apple/Resources/",
    "apps/native/apple/Configuration/",
    "apps/native/desktop/src/",
    "apps/native/desktop/resources/",
    "apps/native/updater/src/",
    "shared/",
    "assets/",
)
TEST_DIRECTORIES = {"test", "tests", "__tests__", "fixtures", "__fixtures__", "snapshots", "__snapshots__", "benches"}
TEST_FILE = re.compile(r"(?:^|/)(?:test_[^/]+|tests?\.[^/]+)$|\.(?:test|spec)\.[^/]+$", re.IGNORECASE)
# Some maintenance commits reformat runtime files or repair tests embedded in them.
# Do not discard product fixes merely because their title also mentions test coverage.
INTERNAL_TITLE = re.compile(
    r"^(?:Bump\b|(?:build|ci|test|tests|chore|docs|refactor)(?:\([^)]*\))?!?:)"
    r"|\b(?:clippy|tooling)\b",
    re.IGNORECASE,
)


def git(*args):
    return subprocess.check_output(["git", "-c", "core.quotepath=false", *args], text=True).strip()


def is_product_path(path):
    return (
        path.startswith(PRODUCT_ROOTS)
        and not path.endswith((".md", ".py"))
        and not TEST_DIRECTORIES.intersection(part.lower() for part in path.split("/"))
        and not TEST_FILE.search(path)
    )


def commit_notes(*revisions):
    # First-parent diffs include merge changes, root commits and removed files.
    records = git(
        "log",
        "--first-parent",
        "--diff-merges=first-parent",
        "--format=%x00%x00%H%x00%s%x00%an%x00%ae",
        "--name-only",
        *revisions,
    )
    commits = {}
    for record in records.split("\0\0")[1:]:
        header, _, paths = record.partition("\n")
        commit, subject, author, email = header.split("\0")
        visible = (
            not INTERNAL_TITLE.search(subject)
            and "dependabot" not in author.lower()
            and "dependabot" not in email.lower()
            and any(is_product_path(path) for path in paths.splitlines())
        )
        commits[commit] = (subject, visible)
    return commits


def filter_retained_notes(notes, hidden):
    # Only remove known generated commit bullets; preserve custom/legacy prose.
    if not notes:
        return notes
    lines = [line for line in notes.splitlines() if not (line.startswith("• ") and line[2:] in hidden)]
    return "\n".join(lines) or NO_PRODUCT_CHANGES


def encode(manifest):
    return (json.dumps(manifest, ensure_ascii=False, separators=(",", ":")) + "\n").encode()


def with_changelog(latest, previous):
    build = latest["build"]
    commit = latest["commit"]
    if previous:
        if previous["build"] > build:
            raise ValueError("cannot publish a lower build number")
        if previous["build"] == build and previous["commit"] != commit:
            raise ValueError("a published build number cannot change commits")
    commits = commit_notes(commit, *([previous["commit"]] if previous else []))
    hidden = {subject for subject, visible in commits.values() if not visible}
    hidden -= {subject for subject, visible in commits.values() if visible}
    history = []
    oldest = 0
    if previous:
        history = [
            {**entry, "notes": filter_retained_notes(entry["notes"], hidden)} for entry in previous.get("changelog", [])
        ]
        oldest = previous.get("changelog_from_build", previous["build"])
        if previous["build"] == build:
            # A workflow rerun must not replace the original notes with an empty range.
            notes = filter_retained_notes(previous["notes"], hidden)
            return fit({**latest, "notes": notes, "changelog": history, "changelog_from_build": oldest})
        ancestor = subprocess.run(
            ["git", "merge-base", "--is-ancestor", previous["commit"], commit],
            check=False,
        ).returncode
        if ancestor not in (0, 1):
            raise ValueError("could not resolve the previous release commit")
        if ancestor == 1:
            notes = f"• Rebuilt earlier source {commit[:7]}; replaces the previous release."
        else:
            changed = git("rev-list", "--first-parent", f"{previous['commit']}..{commit}").splitlines()
            notes = "\n".join(f"• {commits[sha][0]}" for sha in changed if commits[sha][1])
            if changed and not notes:
                notes = NO_PRODUCT_CHANGES
    else:
        notes = "\n".join(f"• {subject}" for subject, visible in commits.values() if visible) or NO_PRODUCT_CHANGES
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
