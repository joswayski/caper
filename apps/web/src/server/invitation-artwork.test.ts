import { test } from "vitest";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

test("native invitation artwork and attribution match the canonical web asset", () => {
  const root = new URL("../../../../", import.meta.url);
  const artwork = readFileSync(new URL("apps/web/public/images/invitation/1f4e8.png", root));
  const notice = readFileSync(new URL("apps/web/public/images/invitation/ATTRIBUTION.txt", root));
  for (const path of [
    "apps/native/android/app/src/main/res/drawable-nodpi/incoming_envelope.png",
    "apps/native/apple/Sources/CaperCore/InvitationAssets.xcassets/IncomingEnvelope.imageset/1f4e8.png",
  ]) assert.deepEqual(readFileSync(new URL(path, root)), artwork, path);
  for (const path of [
    "apps/native/android/third_party/NOTICE-Invitation.txt",
    "apps/native/apple/Sources/CaperCore/InvitationAssets/ATTRIBUTION.txt",
  ]) assert.deepEqual(readFileSync(new URL(path, root)), notice, path);
  assert.equal(artwork.readUInt32BE(16), 64);
  assert.equal(artwork.readUInt32BE(20), 64);
  assert.equal(artwork[25], 6, "PNG uses RGBA, not an opaque RGB image");
});
