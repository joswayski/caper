#!/usr/bin/env bash
# After a release's signed manifest is live: remove patches it no longer lists
# from native-latest, and keep its full archives on the native-update-bases draft
# release so the next releases can patch from this build.
#   native-update-retain.sh latest.json BUILD ARCHIVE...
# Needs GH_TOKEN/GH_REPO with contents write and RELEASE_TAG. A client holding an
# older cached manifest whose patch was just removed downloads the full archive.
set -euo pipefail
manifest="$1"
build="$2"
shift 2
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

listed="$(jq -r '.platforms[].patches[]?.url | sub(".*/"; "")' "$manifest")"
while read -r name; do
  [[ -z "$name" ]] || grep -qxF "$name" <<<"$listed" || gh release delete-asset "$RELEASE_TAG" "$name" -y
done < <(gh release view "$RELEASE_TAG" --json assets --jq '.assets[].name | select(endswith(".caperpatch"))')

if ! gh release view native-update-bases >/dev/null 2>&1; then
  gh release create native-update-bases --draft --title "Native update bases (internal)" \
    --notes "Full desktop archives of recent builds, kept so release.yml can make patch updates from them. Not for download."
fi
retained="$(mktemp -d)"
for archive in "$@"; do
  cp "$archive" "$retained/$build-$(basename "$archive")"
done
gh release upload native-update-bases --clobber "$retained"/*
rm -rf "$retained"
gh release view native-update-bases --json assets --jq '.assets[].name' \
  | python3 "$here/native_update_patches.py" stale \
  | while read -r name; do
    [[ -z "$name" ]] || gh release delete-asset native-update-bases "$name" -y
  done
