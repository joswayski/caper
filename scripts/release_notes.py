#!/usr/bin/env python3
"""Print the tester-facing notes for one platform's store build of a release.

Prefers this release's drafted highlights (whats_new.py --save-answer) that
list the platform. Otherwise uses the commit's pull request title, else its
subject, when the commit ships in that platform's app or in none (server
changes); a change for other apps only gets a generic line. Needs GH_TOKEN and
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


def notes(platform, sha, highlights, subject, paths, limit):
    if highlights:
        # Whole highlights only, so a store's length limit never cuts one off.
        kept = []
        for line in highlights:
            if len("\n".join([*kept, line])) > limit:
                break
            kept.append(line)
        return "\n".join(kept) or highlights[0][:limit]
    platforms = set().union(*(path_platforms(path) for path in paths if is_product_path(path)))
    if subject and (not platforms or platform in platforms):
        return subject[:limit]
    return f"Latest Caper build from {sha[:7]}."


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
    print(notes(args.platform, args.sha, highlights, subject, paths or [], args.limit))


if __name__ == "__main__":
    main()
