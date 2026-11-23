#!/usr/bin/env python3
"""Print the tester-facing notes for one platform's store build of a release.

Prefers this release's drafted highlights (whats_new.py --save-answer) that
list the platform. Otherwise uses the commit's pull request title, else its
subject, when the commit ships in that platform's app or in none (server
changes). A change for other apps only gets the platform's most recent What's
New highlights under a "nothing new" heading, else a generic line. Needs GH_TOKEN and
GITHUB_REPOSITORY.
"""

import argparse
import json
import os
from pathlib import Path
import subprocess

try:
    from .native_changelog import PLATFORMS, is_product_path, path_platforms
except ImportError:
    from native_changelog import PLATFORMS, is_product_path, path_platforms


def drafted(answer, sha, platform):
    """This release's highlights for `platform`, or [] without a usable draft."""
    try:
        saved = json.loads(Path(answer).read_bytes())
        highlights = saved["result"]["highlights"] if saved["revision"] == sha else []
        return [
            f"{item['title']}: {item['body']}"
            for item in highlights
            if platform in item["platforms"] and isinstance(item["title"], str) and isinstance(item["body"], str)
        ]
    except (OSError, ValueError, TypeError, KeyError):
        return []


def recent(feed, sha):
    """The five newest highlights of the platform's earlier What's New releases."""
    try:
        releases = json.loads(feed)["releases"]
        return [
            f"{item['title']}: {item['body']}"
            for release in reversed(releases)
            if release["revision"] != sha
            for item in release["highlights"]
        ][:5]
    except (ValueError, TypeError, KeyError):
        return []


def fit(lines, limit):
    """Whole lines only, so a store's length limit never cuts one off."""
    kept = []
    for line in lines:
        if len("\n".join([*kept, line])) > limit:
            break
        kept.append(line)
    return "\n".join(kept) or lines[0][:limit]


def notes(platform, sha, highlights, subject, paths, limit, earlier=()):
    if highlights:
        return fit(highlights, limit)
    platforms = set().union(*(path_platforms(path) for path in paths if is_product_path(path)))
    if subject and (not platforms or platform in platforms):
        return subject[:limit]
    heading = f"Nothing new for {NAMES.get(platform, platform)} in this build. Recent changes:"
    if earlier and len(heading) < limit:
        return f"{heading}\n{fit(earlier, limit - len(heading) - 1)}"
    return f"Latest Caper build from {sha[:7]}."


NAMES = {"ios": "iPhone", "android": "Android"}


def gh(*args):
    result = subprocess.run(["gh", "api", *args], capture_output=True, text=True)
    return result.stdout.strip() if result.returncode == 0 else ""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("sha")
    parser.add_argument("platform", choices=PLATFORMS)
    parser.add_argument("--answer", type=Path, help="The release's drafted highlights")
    parser.add_argument("--limit", type=int, default=4000)
    args = parser.parse_args()
    repo = os.environ["GITHUB_REPOSITORY"]
    highlights = drafted(args.answer, args.sha, args.platform) if args.answer else []
    subject = paths = None
    if not highlights:
        subject = gh(f"repos/{repo}/commits/{args.sha}/pulls", "--jq", ".[0].title // empty") or gh(
            f"repos/{repo}/commits/{args.sha}", "--jq", '.commit.message | split("\\n")[0]'
        )
        paths = gh(f"repos/{repo}/commits/{args.sha}", "--paginate", "--jq", ".files[].filename").splitlines()
    earlier = []
    if not highlights:
        feed = subprocess.run(
            ["gh", "release", "download", f"whats-new-{args.platform}", "--repo", repo, "-p", "latest.json", "-O", "-"],
            capture_output=True,
        )
        earlier = recent(feed.stdout, args.sha) if feed.returncode == 0 else []
    print(notes(args.platform, args.sha, highlights, subject, paths or [], args.limit, earlier))


if __name__ == "__main__":
    main()
