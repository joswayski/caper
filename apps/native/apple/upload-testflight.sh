#!/usr/bin/env bash
# Archive the iOS app for the App Store and upload it to TestFlight. Signing is
# cloud-managed: xcodebuild uses the App Store Connect API key to create or
# reuse the Apple Distribution certificate and App Store profile, so no
# distribution certificate or profile is persisted between runners.
# The intermediate archive uses a local ad-hoc signature, preserving its
# entitlements without issuing an Apple Development certificate on every run.
#
# Environment: APPLE_TEAM_ID, NOTARY_KEY_PATH (.p8), NOTARY_KEY_ID,
# NOTARY_ISSUER, BUILD_NUMBER (unique and increasing per upload; its run
# number before the dot is the release, so the app shows "0.1.<run>").
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
IOS_BUNDLE_ID="${CAPER_IOS_BUNDLE_ID:-chat.caper.ios}"
# The UIKit client keeps the SwiftUI app's Caper.app identity, extension and entitlements.
IOS_SCHEME=CaperIOSUIKit
: "${APPLE_TEAM_ID:?}" "${NOTARY_KEY_PATH:?}" "${NOTARY_KEY_ID:?}" "${NOTARY_ISSUER:?}" "${BUILD_NUMBER:?}"

auth=(
  -allowProvisioningUpdates
  -authenticationKeyPath "$NOTARY_KEY_PATH"
  -authenticationKeyID "$NOTARY_KEY_ID"
  -authenticationKeyIssuerID "$NOTARY_ISSUER"
)
work="$(mktemp -d)"
system_tmp="$(getconf DARWIN_USER_TEMP_DIR 2>/dev/null || true)"
# xcodebuild -exportArchive leaves ~600 MB XcodeDistPipeline.* folders and app
# analysis zips in the per-user temp folder, whatever TMPDIR says, and they
# filled the shared Mac. Remove this run's, even after a failure. Other jobs on
# that Mac export too, so only items holding Caper.app are ours.
cleanup() {
  set +e
  if [[ -n "$system_tmp" && -e "$work/export-started" ]]; then
    while IFS= read -r -d '' item; do
      if [[ -d "$item" ]]; then
        # Xcode also leaves an empty folder; another job's export may be filling
        # its own, so remove empty ones only while no other xcodebuild runs.
        if [[ -z "$(find "$item" -mindepth 1 -print -quit 2>/dev/null)" ]]; then
          pgrep -x xcodebuild >/dev/null || rmdir "$item"
          continue
        fi
        [[ -n "$(find "$item" -name Caper.app -prune -print -quit 2>/dev/null)" ]] || continue
      else
        unzip -Z1 "$item" 2>/dev/null | grep -c '\(^\|/\)Caper\.app/' >/dev/null || continue
      fi
      rm -rf "$item"
    done < <(find "$system_tmp" -maxdepth 1 -newer "$work/export-started" \
      \( -name 'XcodeDistPipeline.*' -o -name 'DTAppAnalyzerExtractorOutput-*.zip' \) -print0 2>/dev/null)
  fi
  rm -rf "$work"
}
trap cleanup EXIT

"$ROOT/prepare.sh"
source_revision="$(sed -n 's/^CAPER_SOURCE_REVISION = //p' "$ROOT/.build/SourceRevision.xcconfig")"
# Resolve Swift packages first, with retries: a package's git clone (such as
# libwebp's submodule) sometimes fails on the network, and archive gives up
# after one retry of its own.
for attempt in 1 2 3; do
  xcodebuild -project "$ROOT/CaperApple.xcodeproj" -scheme "$IOS_SCHEME" \
    -derivedDataPath "$ROOT/DerivedData-TestFlight" -resolvePackageDependencies && break
  (( attempt < 3 )) || exit 1
  echo "::warning::Swift package resolution failed (attempt $attempt/3); retrying."
  rm -rf "$ROOT/DerivedData-TestFlight/SourcePackages"
  sleep $(( attempt * 15 ))
done
xcodebuild -project "$ROOT/CaperApple.xcodeproj" -scheme "$IOS_SCHEME" -configuration Release \
  -destination 'generic/platform=iOS' -archivePath "$work/Caper.xcarchive" \
  -derivedDataPath "$ROOT/DerivedData-TestFlight" \
  DEVELOPMENT_TEAM="$APPLE_TEAM_ID" CAPER_IOS_BUNDLE_ID="$IOS_BUNDLE_ID" \
  CURRENT_PROJECT_VERSION="$BUILD_NUMBER" MARKETING_VERSION="0.1.${BUILD_NUMBER%%.*}" CAPER_SOURCE_REVISION="$source_revision" \
  CODE_SIGN_IDENTITY=- AD_HOC_CODE_SIGNING_ALLOWED=YES archive

# Validate the built bundles before export can upload them (ITMS-90473).
python3 - "$work/Caper.xcarchive" "$source_revision" <<'PY'
from pathlib import Path
import plistlib
import sys

bundle = Path(sys.argv[1]) / "Products/Applications/Caper.app"
with (bundle / "Info.plist").open("rb") as source:
    app = plistlib.load(source)
if app.get("CaperSourceRevision") != sys.argv[2]:
    sys.exit("CaperSourceRevision does not match the release source SHA.")
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
touch "$work/export-started"
# Apple's service sometimes fails the export before anything is uploaded with
# "Error Downloading App Information"; only that failure is retried, so a
# failed or partial upload is never sent twice.
for attempt in 1 2 3; do
  status=0
  xcodebuild -exportArchive -archivePath "$work/Caper.xcarchive" \
    -exportOptionsPlist "$work/ExportOptions.plist" -exportPath "$work/export" "${auth[@]}" 2>&1 |
    tee "$work/export.log" || status=$?
  ((status == 0)) && break
  if [[ $attempt == 3 ]] || ! grep -q "Error Downloading App Information" "$work/export.log"; then
    exit "$status"
  fi
  echo "App Store Connect did not return the app information; retrying the export in 60 seconds."
  rm -rf "$work/export"
  sleep 60
done
echo "Uploaded $IOS_BUNDLE_ID build $BUILD_NUMBER to App Store Connect; it appears in TestFlight after processing."
