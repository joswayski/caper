// Edits to an attachment before it is sent: crop, quarter turns, drawing and
// text (docs/media.md, "Editing attachments before sending"). The draft keeps
// the original file and this description, so reopening the editor can change
// any part of it; preparing the upload applies it to the original. Geometry
// is shared with every client through shared/messages/edit-cases.json.

/** Clockwise quarter turns applied to the upright original. */
export type EditRotation = 0 | 90 | 180 | 270;

/** Fractions of the rotated, uncropped image's width and height. */
export interface EditRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** A freehand line. `width` is a fraction of the rotated image's longer edge;
 * `points` are flat x, y pairs in the image's fractions. */
export interface StrokeMark {
  kind: "stroke";
  color: string;
  width: number;
  points: number[];
}

/** One line of text centred on x, y; `size` is its font size as a fraction of
 * the rotated image's longer edge. Text stays upright when the image turns. */
export interface TextMark {
  kind: "text";
  color: string;
  size: number;
  x: number;
  y: number;
  text: string;
}

export type EditMark = StrokeMark | TextMark;

/** The editor's tools; the pencil opens Draw, and the preview can open any of
 * them. Draw's eraser is a mode of Draw, not a tool of its own. */
export type EditTool = "crop" | "draw" | "text";

export interface AttachmentEdit {
  rotation: EditRotation;
  crop: EditRect;
  /** Painted in order, over the image. */
  marks: EditMark[];
}

export interface PixelRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

export const FULL_CROP: EditRect = { x: 0, y: 0, width: 1, height: 1 };
export const NO_EDIT: AttachmentEdit = { rotation: 0, crop: FULL_CROP, marks: [] };

/** Markup colours offered before the custom picker; the first is the default. */
export const EDIT_COLORS = ["#E5484D", "#F5C542", "#4CAF50", "#3B82F6", "#FFFFFF", "#000000"] as const;
/** Stroke widths and text sizes in on-screen pixels, converted to fractions as they are drawn. */
export const STROKE_SIZES = { min: 2, max: 40, initial: 6 } as const;
export const TEXT_SIZES = { min: 14, max: 96, initial: 32 } as const;
/** Satoshi Bold, like the app; the fallbacks match `shared/design.css`. */
export const EDIT_FONT = `"Satoshi", "Avenir Next", "Segoe UI", sans-serif`;
export const EDIT_FONT_WEIGHT = 700;

const EPSILON = 1e-6;

/** Edits that could not be applied. Uploads fail rather than send the unedited
 * original: people crop to hide things. */
export class EditFailed extends Error {
  constructor() {
    super("Couldn’t apply your edits.");
  }
}

export function isEdited(edit: AttachmentEdit | undefined): edit is AttachmentEdit {
  if (!edit) return false;
  const { x, y, width, height } = edit.crop;
  return (
    edit.rotation !== 0 ||
    edit.marks.length > 0 ||
    Math.abs(x) > EPSILON ||
    Math.abs(y) > EPSILON ||
    Math.abs(width - 1) > EPSILON ||
    Math.abs(height - 1) > EPSILON
  );
}

export function rotatedSize(width: number, height: number, rotation: EditRotation) {
  return rotation === 90 || rotation === 270 ? { width: height, height: width } : { width, height };
}

/** Turns the picture a quarter anticlockwise, carrying the crop and marks with it. */
export function rotateLeft(edit: AttachmentEdit): AttachmentEdit {
  const { x, y, width, height } = edit.crop;
  return {
    rotation: ((edit.rotation + 270) % 360) as EditRotation,
    crop: { x: y, y: 1 - (x + width), width: height, height: width },
    marks: edit.marks.map((mark) =>
      mark.kind === "stroke"
        ? {
            ...mark,
            points: mark.points.map((_, index) =>
              index % 2 === 0 ? mark.points[index + 1] : 1 - mark.points[index - 1],
            ),
          }
        : { ...mark, x: mark.y, y: 1 - mark.x },
    ),
  };
}

/** The crop in pixels of the rotated image. Video crops start and measure on
 * even pixels (4:2:0 chroma); every crop keeps at least one pixel (two for video). */
export function cropPixels(edit: AttachmentEdit, width: number, height: number, even = false): PixelRect {
  const size = rotatedSize(width, height, edit.rotation);
  const { crop } = edit;
  const left = Math.min(Math.max(Math.round(crop.x * size.width), 0), size.width - 1);
  const top = Math.min(Math.max(Math.round(crop.y * size.height), 0), size.height - 1);
  const right = Math.max(left + 1, Math.min(size.width, Math.round((crop.x + crop.width) * size.width)));
  const bottom = Math.max(top + 1, Math.min(size.height, Math.round((crop.y + crop.height) * size.height)));
  if (!even) return { left, top, width: right - left, height: bottom - top };
  const evenLeft = left - (left % 2);
  const evenTop = top - (top % 2);
  const evenWidth = Math.max(2, right - evenLeft - ((right - evenLeft) % 2));
  const evenHeight = Math.max(2, bottom - evenTop - ((bottom - evenTop) % 2));
  return { left: evenLeft, top: evenTop, width: evenWidth, height: evenHeight };
}

/** Keeps a crop inside the image and at least `min` (a fraction) on each side. */
export function clampCrop(crop: EditRect, min = 0.02): EditRect {
  const width = Math.min(1, Math.max(min, crop.width));
  const height = Math.min(1, Math.max(min, crop.height));
  return {
    x: Math.min(1 - width, Math.max(0, crop.x)),
    y: Math.min(1 - height, Math.max(0, crop.y)),
    width,
    height,
  };
}

type Context = CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;

export function editFont(pixels: number) {
  return `${EDIT_FONT_WEIGHT} ${pixels}px ${EDIT_FONT}`;
}

/** Draws the upright original turned by `rotation` into the rotated image's pixel space. */
function drawRotated(
  context: Context,
  source: CanvasImageSource,
  width: number,
  height: number,
  rotation: EditRotation,
) {
  context.save();
  if (rotation === 90) {
    context.translate(height, 0);
    context.rotate(Math.PI / 2);
  } else if (rotation === 180) {
    context.translate(width, height);
    context.rotate(Math.PI);
  } else if (rotation === 270) {
    context.translate(0, width);
    context.rotate(-Math.PI / 2);
  }
  context.drawImage(source, 0, 0, width, height);
  context.restore();
}

/** Paints marks in the rotated image's pixel space (`width` × `height`). */
export function paintMarks(context: Context, marks: EditMark[], width: number, height: number) {
  const edge = Math.max(width, height);
  for (const mark of marks) {
    context.fillStyle = mark.color;
    if (mark.kind === "text") {
      if (!mark.text) continue;
      context.font = editFont(mark.size * edge);
      context.textAlign = "center";
      // Centred on the middle of the font's ascent and descent, like the
      // native clients; the em box's middle where browsers lack the metrics.
      context.textBaseline = "alphabetic";
      const { fontBoundingBoxAscent: ascent, fontBoundingBoxDescent: descent } = context.measureText(mark.text);
      if (Number.isFinite(ascent) && Number.isFinite(descent))
        context.fillText(mark.text, mark.x * width, mark.y * height + (ascent - descent) / 2);
      else {
        context.textBaseline = "middle";
        context.fillText(mark.text, mark.x * width, mark.y * height);
      }
      continue;
    }
    const lineWidth = mark.width * edge;
    if (mark.points.length < 4) {
      context.beginPath();
      context.arc(mark.points[0] * width, mark.points[1] * height, lineWidth / 2, 0, Math.PI * 2);
      context.fill();
      continue;
    }
    context.strokeStyle = mark.color;
    context.lineWidth = lineWidth;
    context.lineCap = "round";
    context.lineJoin = "round";
    context.beginPath();
    context.moveTo(mark.points[0] * width, mark.points[1] * height);
    for (let index = 2; index + 1 < mark.points.length; index += 2)
      context.lineTo(mark.points[index] * width, mark.points[index + 1] * height);
    context.stroke();
  }
}

/**
 * Paints the edited picture's `region` (pixels of the rotated image) at
 * `scale`, from the upright original of `width` × `height` pixels. Without a
 * source only the marks are painted, as a video's overlay.
 */
export function paintEdit(
  context: Context,
  source: CanvasImageSource | undefined,
  width: number,
  height: number,
  edit: AttachmentEdit,
  region: PixelRect,
  scale: number,
) {
  const rotated = rotatedSize(width, height, edit.rotation);
  context.save();
  context.scale(scale, scale);
  context.translate(-region.left, -region.top);
  if (source) drawRotated(context, source, width, height, edit.rotation);
  paintMarks(context, edit.marks, rotated.width, rotated.height);
  context.restore();
}

/** The fonts text marks use must be loaded before a canvas paints them. */
export async function loadEditFont(edit: AttachmentEdit) {
  if (typeof document === "undefined" || !edit.marks.some((mark) => mark.kind === "text")) return;
  await document.fonts?.load(editFont(32)).catch(() => undefined);
}

/** Still formats an editor can open where the browser decodes them. Callers
 * also rule out animation: GIFs and animated WebP/PNG would keep one frame. */
export function editableImageType(contentType: string) {
  return [
    "image/jpeg",
    "image/png",
    "image/webp",
    "image/avif",
    "image/heic",
    "image/heif",
    "image/bmp",
    "image/tiff",
  ].includes(contentType);
}

export type CropHandle = "move" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w" | "nw";

/** A crop dragged by `dx`, `dy` (fractions of the image) from `start` by one of
 * its handles, kept inside the image and at least `min` on each side. */
export function dragCrop(
  start: EditRect,
  handle: CropHandle,
  dx: number,
  dy: number,
  min: { width: number; height: number },
): EditRect {
  const clamp = (value: number, low: number, high: number) => Math.min(Math.max(value, low), Math.max(low, high));
  if (handle === "move")
    return {
      ...start,
      x: clamp(start.x + dx, 0, 1 - start.width),
      y: clamp(start.y + dy, 0, 1 - start.height),
    };
  let left = start.x,
    top = start.y,
    right = start.x + start.width,
    bottom = start.y + start.height;
  if (handle.includes("w")) left = clamp(left + dx, 0, right - min.width);
  if (handle.includes("e")) right = clamp(right + dx, left + min.width, 1);
  if (handle.includes("n")) top = clamp(top + dy, 0, bottom - min.height);
  if (handle.includes("s")) bottom = clamp(bottom + dy, top + min.height, 1);
  return { x: left, y: top, width: right - left, height: bottom - top };
}

/**
 * The strokes a round eraser of `radius` at `x`, `y` (fractions) touches, by
 * index: those whose line, half its width either side, comes within reach.
 * `width` × `height` is the rotated picture in the same unit as `radius`.
 * Text is never erased; the Text tool removes it by clearing it.
 */
export function strokesTouched(
  marks: EditMark[],
  x: number,
  y: number,
  radius: number,
  width: number,
  height: number,
): number[] {
  const edge = Math.max(width, height);
  const px = x * width;
  const py = y * height;
  const touched: number[] = [];
  marks.forEach((mark, index) => {
    if (mark.kind !== "stroke" || mark.points.length < 2) return;
    const reach = radius + (mark.width * edge) / 2;
    let ax = mark.points[0] * width;
    let ay = mark.points[1] * height;
    let nearest = Math.hypot(px - ax, py - ay);
    for (let point = 2; point + 1 < mark.points.length && nearest > reach; point += 2) {
      const bx = mark.points[point] * width;
      const by = mark.points[point + 1] * height;
      const dx = bx - ax;
      const dy = by - ay;
      const length = dx * dx + dy * dy;
      const t = length ? Math.min(1, Math.max(0, ((px - ax) * dx + (py - ay) * dy) / length)) : 0;
      nearest = Math.min(nearest, Math.hypot(px - (ax + t * dx), py - (ay + t * dy)));
      ax = bx;
      ay = by;
    }
    if (nearest <= reach) touched.push(index);
  });
  return touched;
}

export function sameEdit(a: AttachmentEdit | undefined, b: AttachmentEdit | undefined) {
  return JSON.stringify(a ?? NO_EDIT) === JSON.stringify(b ?? NO_EDIT);
}
