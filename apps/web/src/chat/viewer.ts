/** The media viewer's set, navigation, and zoom and swipe math; MediaViewer renders it. */
import { attachmentView, type ChatAttachment } from "./types.ts";

export interface Size {
  width: number;
  height: number;
}

export interface Point {
  x: number;
  y: number;
}

/** Zoom relative to the fitted size, and how far the media's centre is panned, in pixels. */
export interface ZoomView {
  scale: number;
  x: number;
  y: number;
}

export const FIT: ZoomView = { scale: 1, x: 0, y: 0 };
export const MAX_ZOOM = 4;
/** One-finger drags past these, while not zoomed, change file or close. */
export const SWIPE_DISTANCE = 60;
export const CLOSE_DISTANCE = 100;

/** Ready images and videos (GIF-like ones too) with a URL. Processing, failed and
 * removed files keep their inline cards; audio and other files are not shown. */
export function isViewable(attachment: ChatAttachment) {
  const view = attachmentView(attachment);
  return view === "image" || view === "video" || view === "animated";
}

/** The viewer's set is one message's viewable files in message order; `index` is -1
 * when `id` is no longer one of them. */
export function viewerPosition(attachments: ChatAttachment[], id: string) {
  const items = attachments.filter(isViewable);
  return { items, index: items.findIndex((item) => item.id === id) };
}

/** The viewable file `step` places from `id`, or undefined past either end. Steps
 * from a file removed while open by its place in the message. */
export function viewerStep(attachments: ChatAttachment[], id: string, step: 1 | -1) {
  const start = attachments.findIndex((attachment) => attachment.id === id);
  if (start === -1) return undefined;
  for (let index = start + step; index >= 0 && index < attachments.length; index += step)
    if (isViewable(attachments[index]!)) return attachments[index]!.id;
  return undefined;
}

/** The largest size inside `box` with the media's aspect ratio, never above its
 * own size unless `upscale` (players fill the box; small images stay crisp). */
export function fitSize(media: Size, box: Size, upscale = false): Size {
  const scale = Math.min(box.width / media.width, box.height / media.height, upscale ? Infinity : 1);
  return { width: Math.round(media.width * scale), height: Math.round(media.height * scale) };
}

// No overflow centres the media (and avoids a -0 offset).
const clamp = (value: number, limit: number) => (limit ? Math.min(limit, Math.max(-limit, value)) : 0);

/** Zoom stays within 1–MAX_ZOOM and pans only as far as the media overflows the box. */
export function clampView(view: ZoomView, size: Size, box: Size): ZoomView {
  const scale = Math.min(MAX_ZOOM, Math.max(1, view.scale));
  return {
    scale,
    x: clamp(view.x, Math.max(0, (size.width * scale - box.width) / 2)),
    y: clamp(view.y, Math.max(0, (size.height * scale - box.height) / 2)),
  };
}

/** Zooms to `scale`, keeping the media under `point` (from the box centre) in place. */
export function zoomAt(view: ZoomView, point: Point, scale: number, size: Size, box: Size): ZoomView {
  const next = Math.min(MAX_ZOOM, Math.max(1, scale));
  const ratio = next / view.scale;
  return clampView(
    { scale: next, x: point.x - (point.x - view.x) * ratio, y: point.y - (point.y - view.y) * ratio },
    size,
    box,
  );
}

/** Double-click or double-tap: fit ↔ 2x, centred on the point. */
export function toggleZoom(view: ZoomView, point: Point, size: Size, box: Size) {
  return view.scale > 1 ? FIT : zoomAt(view, point, 2, size, box);
}

/** What a finished one-finger drag means while not zoomed. Swiping left shows the
 * next file, like paging; down closes. */
export function swipeAction(dx: number, dy: number): "next" | "previous" | "close" | undefined {
  if (Math.abs(dx) > Math.abs(dy)) {
    if (dx <= -SWIPE_DISTANCE) return "next";
    if (dx >= SWIPE_DISTANCE) return "previous";
  } else if (dy >= CLOSE_DISTANCE) return "close";
  return undefined;
}
