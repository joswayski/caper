#!/usr/bin/env bash
# Re-sign dist/Caper.app with a Developer ID identity and the hardened runtime,
# notarize it with an App Store Connect API key, staple the ticket, and write
# dist/Caper-macOS-<label>.zip. Run after `build.sh macos`.
#
# Environment: APPLE_SIGNING_IDENTITY (a Developer ID Application identity in an
# unlocked keychain), NOTARY_KEY_PATH (.p8), NOTARY_KEY_ID, NOTARY_ISSUER.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
LABEL="${1:?Usage: $0 Apple-Silicon|Intel}"
APP="$ROOT/dist/Caper.app"
ENTITLEMENTS="$ROOT/Configuration/macOS-Release.entitlements"
: "${APPLE_SIGNING_IDENTITY:?}" "${NOTARY_KEY_PATH:?}" "${NOTARY_KEY_ID:?}" "${NOTARY_ISSUER:?}"
[[ "$APPLE_SIGNING_IDENTITY" == "Developer ID Application:"* ]] || {
  echo "APPLE_SIGNING_IDENTITY must be a Developer ID Application identity." >&2
  exit 1
}
test -d "$APP"

sign() {
  codesign --force --timestamp --options runtime --sign "$APPLE_SIGNING_IDENTITY" "$@"
}

# Inside out: nested dylibs and frameworks first, deepest paths first, then the app.
while IFS= read -r item; do
  sign "$item"
done < <(find "$APP/Contents/Frameworks" \( -name '*.dylib' -o -name '*.framework' \) -print \
  | awk '{ print length($0) "\t" $0 }' | sort -rn | cut -f2-)
sign --entitlements "$ENTITLEMENTS" "$APP"

codesign --verify --deep --strict --verbose=2 "$APP"
codesign -d --entitlements - "$APP" 2>/dev/null | grep -q 'com.apple.security.device.audio-input'

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
ditto -c -k --keepParent "$APP" "$work/Caper.zip"
xcrun notarytool submit "$work/Caper.zip" \
  --key "$NOTARY_KEY_PATH" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER" \
  --wait --timeout 30m --output-format json >"$work/notary.json"
status="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["status"])' "$work/notary.json")"
if [[ "$status" != "Accepted" ]]; then
  submission="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["id"])' "$work/notary.json")"
  xcrun notarytool log "$submission" \
    --key "$NOTARY_KEY_PATH" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER" || true
  echo "Notarization finished with status $status." >&2
  exit 1
fi

xcrun stapler staple "$APP"
xcrun stapler validate "$APP"
# What a friend's Mac checks when they open the download.
spctl --assess --type execute --verbose=2 "$APP" 2>&1 | tee "$work/spctl.txt"
grep -q 'source=Notarized Developer ID' "$work/spctl.txt"

out="$ROOT/dist/Caper-macOS-$LABEL.zip"
rm -f "$out"
ditto -c -k --keepParent "$APP" "$out"
echo "$out"
