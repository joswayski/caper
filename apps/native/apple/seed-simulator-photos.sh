#!/usr/bin/env bash
# Gives the UI tests' iOS simulator something in its photo library. New
# simulators on the Xcode 27 images start with an empty library, so the attach
# drawer's grid has no tiles to pick. Adds three plain test pictures once per
# device (a marker in the device's data directory), then leaves it booted.
#
#   ./apps/native/apple/seed-simulator-photos.sh "<xcodebuild -destination>"
set -euo pipefail

destination="$1"
if [[ "$destination" =~ (^|,)id=([^,]+) ]]; then
  udid="${BASH_REMATCH[2]}"
else
  name="iPhone 16"
  [[ "$destination" =~ (^|,)name=([^,]+) ]] && name="${BASH_REMATCH[2]}"
  # The newest available runtime's device, as OS=latest resolves it.
  udid=$(xcrun simctl list devices available -j | python3 -I -c '
import json, re, sys
name = sys.argv[1]
devices = json.load(sys.stdin)["devices"]
def version(runtime):
    return [int(part) for part in re.findall(r"\d+", runtime.rsplit(".", 1)[-1])]
for runtime in sorted((r for r in devices if ".iOS-" in r), key=version, reverse=True):
    for device in devices[runtime]:
        if device["name"] == name:
            print(device["udid"])
            sys.exit()
' "$name")
fi
if [[ -z "$udid" ]]; then
  echo "seed-simulator-photos: no simulator for $destination" >&2
  exit 1
fi

marker="$HOME/Library/Developer/CoreSimulator/Devices/$udid/data/.caper-test-photos"
[[ -f "$marker" ]] && exit 0
xcrun simctl boot "$udid" 2>/dev/null || true
xcrun simctl bootstatus "$udid" -b >/dev/null

pictures="$(mktemp -d)"
trap 'rm -rf "$pictures"' EXIT
# Solid 600x400 PNGs in Caper's terracotta, green and eggshell.
python3 -I - "$pictures" <<'PY'
import struct, sys, zlib
def png(path, rgb, width=600, height=400):
    row = b"\x00" + bytes(rgb) * width
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    with open(path, "wb") as out:
        out.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
                  + chunk(b"IDAT", zlib.compress(row * height)) + chunk(b"IEND", b""))
for index, rgb in enumerate([(182, 77, 50), (99, 122, 67), (241, 237, 228)]):
    png(f"{sys.argv[1]}/caper-test-photo-{index + 1}.png", rgb)
PY
xcrun simctl addmedia "$udid" "$pictures"/*.png
touch "$marker"
