/**
 * Verification files that let the native apps open `caper.chat/invite/<code>`
 * directly instead of the browser. Android checks `assetlinks.json` against the
 * installed app's signing certificate; iOS checks the associated-domains file.
 * The Mac (Developer ID) and Windows/Linux apps use the `caper://` scheme
 * instead, from the invite page's "Open in the Caper app".
 */

export const APPLE_TEAM_ID = "NAUU43LATL";
export const IOS_BUNDLE_ID = "chat.caper.ios";
export const ANDROID_PACKAGE = "chat.caper.android";

/**
 * SHA-256 fingerprints of certificates that sign `chat.caper.android`.
 * The upload key signs the GitHub APK. Google Play re-signs Play installs
 * with its app-signing key (Play Console → Test and release → App integrity →
 * App signing): add that fingerprint here so Play installs verify too.
 */
export const ANDROID_CERT_FINGERPRINTS = [
  // Upload key (GitHub release APK).
  "88:0E:35:78:F0:A3:49:08:6D:1D:C7:FF:4F:9E:56:3C:75:5E:82:37:AB:6E:3B:A3:E8:86:D3:1D:EB:D9:1F:E4",
];

export function appleAppSiteAssociation(): Response {
  return Response.json({
    applinks: {
      details: [
        {
          appIDs: [`${APPLE_TEAM_ID}.${IOS_BUNDLE_ID}`],
          components: [{ "/": "/invite/*", comment: "Space invite links" }],
        },
      ],
    },
  });
}

export function androidAssetLinks(): Response {
  return Response.json([
    {
      relation: ["delegate_permission/common.handle_all_urls"],
      target: {
        namespace: "android_app",
        package_name: ANDROID_PACKAGE,
        sha256_cert_fingerprints: ANDROID_CERT_FINGERPRINTS,
      },
    },
  ]);
}
