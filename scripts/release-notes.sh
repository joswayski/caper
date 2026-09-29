#!/usr/bin/env bash
# Print the tester-facing note for a release commit: its pull request title,
# else the commit subject. Needs GH_TOKEN and GITHUB_REPOSITORY.
set -euo pipefail
sha="${1:?usage: release-notes.sh SHA}"
notes="$(gh api "repos/$GITHUB_REPOSITORY/commits/$sha/pulls" --jq '.[0].title // empty' 2>/dev/null || true)"
[[ -n "$notes" ]] || notes="$(gh api "repos/$GITHUB_REPOSITORY/commits/$sha" --jq '.commit.message | split("\n")[0]' 2>/dev/null || true)"
printf '%s\n' "${notes:-Latest Caper build from ${sha:0:7}.}"
