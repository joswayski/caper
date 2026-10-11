import assert from "node:assert/strict";
import { test } from "vitest";
import type { ChatAttachment } from "../chat/types.ts";
import {
  FIT,
  MAX_ZOOM,
  clampView,
  fitSize,
  isViewable,
  swipeAction,
  toggleZoom,
  viewerPosition,
  viewerStep,
  zoomAt,
} from "../chat/viewer.ts";

const url = (id: string) => `https://cdn.test/original/${id}?exp=1&sig=s`;
const file = (id: string, overrides: Partial<ChatAttachment> = {}): ChatAttachment => ({
  id,
  kind: "image",
  contentType: "image/webp",
  name: `${id}.webp`,
  size: 10,
  url: url(id),
  ...overrides,
});

// One message's files, in message order.
const attachments = [
  file("photo"),
  file("voice", { kind: "audio", contentType: "audio/mp4" }),
  file("encoding", { kind: "video", status: "processing", url: undefined }),
  file("clip", { kind: "video", contentType: "video/mp4", status: "ready" }),
  file("notes", { kind: "file", contentType: "application/pdf" }),
  file("deleted", { unavailable: true }),
  file("gif", { kind: "video", contentType: "video/mp4", animated: true }),
  file("broken", { status: "failed", url: undefined }),
  file("unsigned", { url: undefined }),
];

test("the set is the message's ready images and videos, GIF-like ones included, in message order", () => {
  assert.deepEqual(
    attachments.filter(isViewable).map((item) => item.id),
    ["photo", "clip", "gif"],
  );
  const { items, index } = viewerPosition(attachments, "clip");
  assert.deepEqual(
    items.map((item) => item.id),
    ["photo", "clip", "gif"],
  );
  assert.equal(index, 1, "opens at the clicked file: counter 2 / 3");
  assert.equal(viewerPosition(attachments, "photo").index, 0);
  assert.equal(viewerPosition(attachments, "voice").index, -1, "audio plays inline");
  assert.equal(viewerPosition(attachments, "deleted").index, -1, "removed files show their placeholder");
  assert.equal(viewerPosition(attachments, "missing").index, -1);
  assert.deepEqual(viewerPosition([file("only")], "only"), { items: [file("only")], index: 0 });
});

test("previous and next stop at either end and skip files the viewer can't show", () => {
  assert.equal(viewerStep(attachments, "photo", -1), undefined, "no wrap before the first");
  assert.equal(viewerStep(attachments, "photo", 1), "clip", "skips audio and a processing video");
  assert.equal(viewerStep(attachments, "clip", 1), "gif", "skips other files and removed ones");
  assert.equal(viewerStep(attachments, "gif", 1), undefined, "no wrap after the last");
  assert.equal(viewerStep(attachments, "gif", -1), "clip");
  assert.equal(viewerStep([file("only")], "only", 1), undefined);
  assert.equal(viewerStep(attachments, "missing", 1), undefined);
});

test("a file removed while open keeps its place, so navigation continues around it", () => {
  const removed = attachments.map((item) => (item.id === "clip" ? { ...item, unavailable: true } : item));
  assert.equal(viewerPosition(removed, "clip").index, -1);
  assert.deepEqual(
    viewerPosition(removed, "clip").items.map((item) => item.id),
    ["photo", "gif"],
  );
  assert.equal(viewerStep(removed, "clip", -1), "photo");
  assert.equal(viewerStep(removed, "clip", 1), "gif");
});

test("media fits the box with its aspect ratio; images never upscale and videos fill like a player", () => {
  const box = { width: 1000, height: 800 };
  assert.deepEqual(fitSize({ width: 4000, height: 2000 }, box), { width: 1000, height: 500 });
  assert.deepEqual(fitSize({ width: 1000, height: 4000 }, box), { width: 200, height: 800 });
  assert.deepEqual(fitSize({ width: 64, height: 32 }, box), { width: 64, height: 32 });
  assert.deepEqual(fitSize({ width: 640, height: 360 }, box, true), { width: 1000, height: 563 });
});

test("double-click zooms to 2x around the point and back to fit", () => {
  const size = { width: 800, height: 600 };
  const box = { width: 800, height: 600 };
  const zoomed = toggleZoom(FIT, { x: 100, y: -50 }, size, box);
  // The point under the cursor stays put: x' = p - (p - x) * 2.
  assert.deepEqual(zoomed, { scale: 2, x: -100, y: 50 });
  assert.deepEqual(toggleZoom(zoomed, { x: 0, y: 0 }, size, box), FIT);
  assert.deepEqual(
    toggleZoom(FIT, { x: 400, y: 300 }, size, box),
    { scale: 2, x: -400, y: -300 },
    "a corner zooms to the corner, at the edge of the pan range",
  );
});

test("zoom stays between fit and the maximum, and pans only as far as the media overflows", () => {
  const size = { width: 400, height: 300 };
  const box = { width: 1000, height: 800 };
  assert.deepEqual(zoomAt(FIT, { x: 200, y: 0 }, 0.5, size, box), FIT, "never smaller than fit");
  assert.equal(zoomAt(FIT, { x: 0, y: 0 }, 100, size, box).scale, MAX_ZOOM);
  // 2x is 800×600 inside 1000×800: no overflow, so it stays centred.
  assert.deepEqual(zoomAt(FIT, { x: 150, y: 100 }, 2, size, box), { scale: 2, x: 0, y: 0 });
  // 4x is 1600×1200: up to 300px and 200px either way.
  assert.deepEqual(clampView({ scale: 4, x: 900, y: -900 }, size, box), { scale: 4, x: 300, y: -200 });
  assert.deepEqual(clampView({ scale: 4, x: -120, y: 50 }, size, box), { scale: 4, x: -120, y: 50 });
});

test("swipes change file left/right and close downward; short or diagonal drags do nothing", () => {
  assert.equal(swipeAction(-80, 10), "next", "swiping left pages forward");
  assert.equal(swipeAction(80, -10), "previous");
  assert.equal(swipeAction(140, 0), "previous");
  assert.equal(swipeAction(0, 120), "close");
  assert.equal(swipeAction(30, 0), undefined, "too short");
  assert.equal(swipeAction(0, 60), undefined, "too short to close");
  assert.equal(swipeAction(0, -200), undefined, "swiping up doesn't close");
  assert.equal(swipeAction(70, 90), undefined, "mostly vertical, not far enough to close");
});
