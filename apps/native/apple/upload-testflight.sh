#!/usr/bin/env bash
# Archive the iOS app for the App Store and upload it to TestFlight. Signing is
# cloud-managed: xcodebuild uses the App Store Connect API key to create or
# reuse the Apple Distribution certificate and App Store profile, so no
# distribution certificate or profile is persisted between runners.
# The intermediate archive uses a local ad-hoc signature, preserving its
# entitlements without issuing an Apple Development certificate on every run.
#
# Environment: APPLE_TEAM_ID, NOTARY_KEY_PATH (.p8), NOTARY_KEY_ID,
# NOTARY_ISSUER, BUILD_NUMBER (unique and increasing per upload).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
IOS_BUNDLE_ID="${CAPER_IOS_BUNDLE_ID:-chat.caper.ios}"
: "${APPLE_TEAM_ID:?}" "${NOTARY_KEY_PATH:?}" "${NOTARY_KEY_ID:?}" "${NOTARY_ISSUER:?}" "${BUILD_NUMBER:?}"

auth=(
  -allowProvisioningUpdates
  -authenticationKeyPath "$NOTARY_KEY_PATH"
  -authenticationKeyID "$NOTARY_KEY_ID"
  -authenticationKeyIssuerID "$NOTARY_ISSUER"
)
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

"$ROOT/prepare.sh"
xcodebuild -project "$ROOT/CaperApple.xcodeproj" -scheme CaperIOS -configuration Release \
  -destination 'generic/platform=iOS' -archivePath "$work/Caper.xcarchive" \
  -derivedDataPath "$ROOT/DerivedData-TestFlight" \
  DEVELOPMENT_TEAM="$APPLE_TEAM_ID" CAPER_IOS_BUNDLE_ID="$IOS_BUNDLE_ID" \
  CURRENT_PROJECT_VERSION="$BUILD_NUMBER" \
  CODE_SIGN_IDENTITY=- AD_HOC_CODE_SIGNING_ALLOWED=YES archive

# Validate the built bundles before export can upload them (ITMS-90473).
python3 - "$work/Caper.xcarchive" <<'PY'
from pathlib import Path
import plistlib
import sys

bundle = Path(sys.argv[1]) / "Products/Applications/Caper.app"
with (bundle / "Info.plist").open("rb") as source:
    app = plistlib.load(source)
with (bundle / "PlugIns/CaperNotificationService.appex/Info.plist").open("rb") as source:
    extension = plistlib.load(source)
for key in ("CFBundleVersion", "CFBundleShortVersionString"):
    if extension[key] != app[key]:
        sys.exit(f"CaperNotificationService: {key} {extension[key]!r} does not match app {app[key]!r}.")
print(f"Verified iOS app/extension versions: {app['CFBundleShortVersionString']} ({app['CFBundleVersion']}).")
PY

cat >"$work/ExportOptions.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>method</key><string>app-store-connect</string>
  <key>destination</key><string>upload</string>
  <key>teamID</key><string>$APPLE_TEAM_ID</string>
  <key>signingStyle</key><string>automatic</string>
  <key>manageAppVersionAndBuildNumber</key><false/>
  <key>uploadSymbols</key><true/>
</dict></plist>
EOF
xcodebuild -exportArchive -archivePath "$work/Caper.xcarchive" \
  -exportOptionsPlist "$work/ExportOptions.plist" -exportPath "$work/export" "${auth[@]}"
echo "Uploaded $IOS_BUNDLE_ID build $BUILD_NUMBER to App Store Connect; it appears in TestFlight after processing."
