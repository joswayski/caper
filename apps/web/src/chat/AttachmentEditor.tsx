import {
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { Brush, Crop, Redo2, RotateCcw, Type, Undo2, X } from "lucide-react";
import {
  FloatingFocusManager,
  FloatingOverlay,
  FloatingPortal,
  useFloating,
  useInteractions,
  useRole,
} from "@floating-ui/react";
import Slider from "../components/Slider.tsx";
import {
  EDIT_COLORS,
  FULL_CROP,
  NO_EDIT,
  STROKE_SIZES,
  TEXT_SIZES,
  clampCrop,
  cropPixels,
  dragCrop,
  editFont,
  isEdited,
  loadEditFont,
  paintEdit,
  rotateLeft,
  rotatedSize,
  sameEdit,
  type AttachmentEdit,
  type CropHandle,
  type EditRect,
  type EditTool as Tool,
  type StrokeMark,
  type TextMark,
} from "./edit.ts";
import { encode, fitWithin, surface } from "./prepare.ts";

/** The picture being edited, downscaled for the screen; exports use the original. */
interface Picture {
  image: ImageBitmap;
  width: number;
  height: number;
}

const DISPLAY_EDGE = 2560;
const PREVIEW_EDGE = 1280;
const MAX_TEXT = 200;
/** On-screen padding around the picture; the crop frame's handles need more. */
const PADDING = { crop: 28, other: 16 };
const MIN_CROP_PIXELS = 32;
/** The smallest brush ring, in CSS pixels, so a fine brush still shows where it lands. */
const BRUSH_MIN = 6;
const CROP_HANDLES: CropHandle[] = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];

async function loadPicture(file: Blob, kind: "image" | "video"): Promise<Picture | undefined> {
  let full: ImageBitmap | undefined;
  if (kind === "image") full = await createImageBitmap(file, { imageOrientation: "from-image" }).catch(() => undefined);
  else full = await videoFrame(file);
  if (!full) return;
  const edge = Math.max(full.width, full.height);
  if (edge <= DISPLAY_EDGE) return { image: full, width: full.width, height: full.height };
  const size = fitWithin(full.width, full.height, DISPLAY_EDGE);
  const image = await createImageBitmap(full, {
    resizeWidth: size.width,
    resizeHeight: size.height,
    resizeQuality: "high",
  }).catch(() => undefined);
  if (!image) return { image: full, width: full.width, height: full.height };
  full.close();
  return { image, width: image.width, height: image.height };
}

/** A still from just after the start, like the posters uploads get. */
function videoFrame(file: Blob) {
  return new Promise<ImageBitmap | undefined>((resolve) => {
    const video = document.createElement("video");
    const url = URL.createObjectURL(file);
    let settled = false;
    const done = (image?: ImageBitmap) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      URL.revokeObjectURL(url);
      video.removeAttribute("src");
      resolve(image);
    };
    const timer = setTimeout(() => done(), 10_000);
    video.muted = true;
    video.playsInline = true;
    video.preload = "auto";
    video.onerror = () => done();
    video.onloadeddata = () => {
      video.onseeked = () => {
        if (!video.videoWidth || !video.videoHeight) return done();
        void createImageBitmap(video).then(done, () => done());
      };
      video.currentTime = Math.min(0.5, (video.duration || 1) / 2);
    };
    video.src = url;
  });
}

/** A small render of the edit for the composer's thumbnail and preview. */
async function renderPreview(picture: Picture, edit: AttachmentEdit): Promise<Blob | undefined> {
  await loadEditFont(edit);
  const region = cropPixels(edit, picture.width, picture.height);
  const size = fitWithin(region.width, region.height, PREVIEW_EDGE);
  const canvas = surface(size.width, size.height);
  const context = canvas.getContext("2d") as CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D | null;
  if (!context) return;
  context.imageSmoothingQuality = "high";
  paintEdit(context, picture.image, picture.width, picture.height, edit, region, size.width / region.width);
  return (await encode(canvas, "image/webp", 0.9)) ?? (await encode(canvas, "image/png"));
}

let measuring: CanvasRenderingContext2D | null | undefined;
function textWidth(text: string, pixels: number) {
  measuring ??= document.createElement("canvas").getContext("2d");
  if (!measuring) return text.length * pixels * 0.6;
  measuring.font = editFont(pixels);
  return measuring.measureText(text).width;
}

/** A text field open over the picture: a new mark, or `index` being changed. */
interface TextField {
  index?: number;
  x: number;
  y: number;
  value: string;
  color: string;
  /** Points on screen; stored as a fraction when committed. */
  points: number;
}

type Gesture =
  | { kind: "draw"; pointer: number; stroke: StrokeMark; last: { x: number; y: number } }
  | { kind: "crop"; pointer: number; handle: CropHandle; start: EditRect; from: { x: number; y: number } }
  | {
      kind: "text";
      pointer: number;
      index?: number;
      from: { x: number; y: number };
      origin: { x: number; y: number };
      moved: boolean;
    };

/** The edit with an open text field applied, as committing it would. */
function withField(edit: AttachmentEdit, field: TextField | undefined, edge: number): AttachmentEdit {
  if (!field) return edit;
  const text = field.value.trim().slice(0, MAX_TEXT);
  const size = field.points / edge;
  const marks = [...edit.marks];
  if (field.index !== undefined) {
    const mark = marks[field.index] as TextMark | undefined;
    if (!mark) return edit;
    if (!text) marks.splice(field.index, 1);
    else if (text !== mark.text || field.color !== mark.color || Math.abs(size - mark.size) > 1e-9)
      marks[field.index] = { ...mark, text, color: field.color, size };
    else return edit;
  } else if (text) marks.push({ kind: "text", color: field.color, size, x: field.x, y: field.y, text });
  else return edit;
  return { ...edit, marks };
}

/**
 * Full-window editor for a file waiting to be sent: crop and turn it, draw on
 * it and add text. It edits a description of the changes, never the file; the
 * composer prepares the original again with them (docs/media.md, "Editing
 * attachments before sending").
 */
export default function AttachmentEditor({
  file,
  kind,
  name,
  edit: initial,
  tool: initialTool = "draw",
  anchor,
  pictureMode = false,
  onCancel,
  onSave,
}: {
  file: Blob;
  kind: "image" | "video";
  name: string;
  edit?: AttachmentEdit;
  /** The tool it opens on. */
  tool?: Tool;
  /** Gets focus back on close. */
  anchor?: HTMLElement;
  /** Square framing for profile pictures and space icons; Done always saves. */
  pictureMode?: boolean;
  /** Keep this stable: on phones it owns a history entry. */
  onCancel: () => void;
  /** Called only for a change; `edit` is undefined when it returns to the original. */
  onSave: (edit: AttachmentEdit | undefined, preview?: Blob) => void;
}) {
  const [picture, setPicture] = useState<Picture>();
  const [failed, setFailed] = useState(false);
  const [history, setHistory] = useState(() => ({ states: [initial ?? NO_EDIT], index: 0 }));
  const [tool, setTool] = useState<Tool>(initialTool);
  const [color, setColor] = useState<string>(EDIT_COLORS[0]);
  const [strokePoints, setStrokePoints] = useState<number>(STROKE_SIZES.initial);
  const [textPoints, setTextPoints] = useState<number>(TEXT_SIZES.initial);
  const [box, setBox] = useState<{ width: number; height: number }>();
  const [gesture, setGesture] = useState<Gesture>();
  const [liveCrop, setLiveCrop] = useState<EditRect>();
  const [liveText, setLiveText] = useState<{ index: number; x: number; y: number }>();
  const [field, setField] = useState<TextField>();
  const [fontsReady, setFontsReady] = useState(0);
  const [saving, setSaving] = useState(false);
  const [stage, setStage] = useState<HTMLDivElement | null>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const brush = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const dialog = useRef<HTMLDivElement | null>(null);
  const returnFocus = useRef<HTMLElement | null>(anchor ?? null);
  const titleId = useId();
  const hintId = useId();
  const { refs, context } = useFloating({ open: true });
  const { getFloatingProps } = useInteractions([useRole(context)]);
  const current = history.states[history.index];
  const canUndo = history.index > 0;
  const canRedo = history.index < history.states.length - 1;

  useEffect(() => {
    let active = true;
    let loaded: Picture | undefined;
    void loadPicture(file, kind).then((result) => {
      if (!active) return result?.image.close();
      loaded = result;
      if (result) {
        setPicture(result);
        if (pictureMode) {
          const side = Math.min(result.width, result.height);
          setHistory({
            states: [
              {
                ...NO_EDIT,
                crop: {
                  x: (1 - side / result.width) / 2,
                  y: (1 - side / result.height) / 2,
                  width: side / result.width,
                  height: side / result.height,
                },
              },
            ],
            index: 0,
          });
        }
      } else setFailed(true);
    });
    return () => {
      active = false;
      loaded?.image.close();
    };
  }, [file, kind, pictureMode]);
  useEffect(() => {
    // Canvas text needs Satoshi loaded; repaint once it is.
    void document.fonts?.load(editFont(32)).then(() => setFontsReady((count) => count + 1));
  }, []);
  // The portal attaches the stage after this component's first effects run.
  useLayoutEffect(() => {
    if (!stage) return;
    const measure = () =>
      setBox((current) =>
        current?.width === stage.clientWidth && current.height === stage.clientHeight
          ? current
          : { width: stage.clientWidth, height: stage.clientHeight },
      );
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(stage);
    return () => observer.disconnect();
  }, [stage]);
  useEffect(() => {
    // Back cancels on phones and tablets, like the media viewer.
    if (!window.matchMedia("(max-width: 760px), (pointer: coarse)").matches) return;
    const key = crypto.randomUUID();
    // A remount takes over the entry its last mount left rather than pushing a
    // second one that the first's Back would then close. Opened from the media
    // viewer, it takes over the viewer's entry the same way, which also stops
    // the closing viewer from going back.
    const { caperViewer: _viewer, ...state } = window.history.state ?? {};
    if (window.history.state?.caperEditor || window.history.state?.caperViewer)
      window.history.replaceState({ ...state, caperEditor: key }, "");
    else window.history.pushState({ ...state, caperEditor: key }, "");
    const back = () => {
      if (window.history.state?.caperEditor !== key) onCancel();
    };
    window.addEventListener("popstate", back);
    return () => {
      window.removeEventListener("popstate", back);
      // Left until the next task, so a remount in the same commit can take the entry over.
      setTimeout(() => {
        if (window.history.state?.caperEditor === key) window.history.back();
      });
    };
  }, [onCancel]);

  const rotated = picture ? rotatedSize(picture.width, picture.height, current.rotation) : undefined;
  const region = tool === "crop" ? FULL_CROP : current.crop;
  const layout = useMemo(() => {
    if (!rotated || !box) return;
    const padding = tool === "crop" ? PADDING.crop : PADDING.other;
    const width = region.width * rotated.width;
    const height = region.height * rotated.height;
    const scale = Math.max(0.0001, Math.min((box.width - padding * 2) / width, (box.height - padding * 2) / height));
    return {
      scale,
      width: width * scale,
      height: height * scale,
      // The full rotated picture's longer edge on screen: sizes are fractions of it.
      edge: scale * Math.max(rotated.width, rotated.height),
    };
  }, [rotated?.width, rotated?.height, box, tool, region.x, region.y, region.width, region.height]);

  // What is on screen: the committed edit plus any drag in progress.
  const shown = useMemo<AttachmentEdit>(() => {
    let marks = current.marks.map((mark, index) =>
      liveText?.index === index ? { ...mark, x: liveText.x, y: liveText.y } : mark,
    );
    if (field?.index !== undefined) marks = marks.filter((_, index) => index !== field.index);
    if (gesture?.kind === "draw") marks = [...marks, gesture.stroke];
    return { ...current, crop: liveCrop ?? current.crop, marks };
  }, [current, liveText, field?.index, gesture, liveCrop]);

  useLayoutEffect(() => {
    const element = canvas.current;
    if (!element || !picture || !layout || !rotated) return;
    const ratio = window.devicePixelRatio || 1;
    element.width = Math.max(1, Math.round(layout.width * ratio));
    element.height = Math.max(1, Math.round(layout.height * ratio));
    const context = element.getContext("2d");
    if (!context) return;
    context.setTransform(1, 0, 0, 1, 0, 0);
    context.clearRect(0, 0, element.width, element.height);
    context.imageSmoothingQuality = "high";
    paintEdit(
      context,
      picture.image,
      picture.width,
      picture.height,
      shown,
      {
        left: region.x * rotated.width,
        top: region.y * rotated.height,
        width: region.width * rotated.width,
        height: region.height * rotated.height,
      },
      element.width / (region.width * rotated.width),
    );
  }, [picture, layout, shown, region, fontsReady]);

  useEffect(() => {
    if (field) input.current?.focus();
  }, [field?.index, field?.x, field?.y, !!field]);
  useEffect(() => {
    // Undo, Redo and Reset disable themselves at either end; keep focus (and
    // the keyboard shortcuts) in the editor rather than on the page behind.
    if (dialog.current && !dialog.current.contains(document.activeElement)) dialog.current.focus();
  }, [history]);

  const commit = (next: AttachmentEdit) =>
    setHistory((history) =>
      sameEdit(next, history.states[history.index])
        ? history
        : { states: [...history.states.slice(0, history.index + 1), next], index: history.index + 1 },
    );
  // A press on the picture and the field's blur can both commit it in one turn.
  const openField = useRef(field);
  openField.current = field;
  const commitField = () => {
    const open = openField.current;
    if (!open || !layout) return;
    openField.current = undefined;
    commit(withField(current, open, layout.edge));
    setField(undefined);
  };
  const undo = () => {
    setField(undefined);
    setHistory((history) => (history.index > 0 ? { ...history, index: history.index - 1 } : history));
  };
  const redo = () => {
    setField(undefined);
    setHistory((history) =>
      history.index < history.states.length - 1 ? { ...history, index: history.index + 1 } : history,
    );
  };
  const done = async () => {
    if (saving) return;
    const final = layout ? withField(current, field, layout.edge) : current;
    if (!pictureMode && sameEdit(final, initial)) {
      onCancel();
      return;
    }
    const result = pictureMode || isEdited(final) ? final : undefined;
    setSaving(true);
    const preview = result && picture ? await renderPreview(picture, result).catch(() => undefined) : undefined;
    onSave(result, preview);
  };

  /** A pointer position in fractions of the rotated, uncropped picture. */
  const pointAt = (event: { clientX: number; clientY: number }) => {
    const rect = canvas.current!.getBoundingClientRect();
    return {
      x: region.x + ((event.clientX - rect.left) / rect.width) * region.width,
      y: region.y + ((event.clientY - rect.top) / rect.height) * region.height,
    };
  };
  /** The topmost text mark under a pointer, by its on-screen box. */
  const textAt = (point: { x: number; y: number }) => {
    if (!layout || !rotated) return;
    for (let index = current.marks.length - 1; index >= 0; index--) {
      const mark = current.marks[index];
      if (mark.kind !== "text") continue;
      const pixels = mark.size * layout.edge;
      const halfWidth = textWidth(mark.text, pixels) / 2 + 8;
      const halfHeight = pixels * 0.6 + 8;
      const dx = (point.x - mark.x) * rotated.width * layout.scale;
      const dy = (point.y - mark.y) * rotated.height * layout.scale;
      if (Math.abs(dx) <= halfWidth && Math.abs(dy) <= halfHeight) return index;
    }
  };

  // Pointer events can outpace renders, so handlers read the live gesture here.
  const live = useRef<Gesture>(undefined);
  const track = (next: Gesture | undefined) => {
    live.current = next;
    setGesture(next);
  };
  /** Where a dragged text lands: its origin moved by the pointer since the press. */
  const movedText = (moving: Extract<Gesture, { kind: "text" }>, event: { clientX: number; clientY: number }) => {
    const point = pointAt(event);
    return {
      x: Math.min(1, Math.max(0, moving.origin.x + point.x - moving.from.x)),
      y: Math.min(1, Math.max(0, moving.origin.y + point.y - moving.from.y)),
    };
  };
  /** The crop a frame drag leaves, from where it started. */
  const draggedCrop = (moving: Extract<Gesture, { kind: "crop" }>, event: { clientX: number; clientY: number }) => {
    const rect = canvas.current!.getBoundingClientRect();
    if (pictureMode)
      return clampCrop(
        {
          ...moving.start,
          x: moving.start.x + event.clientX / rect.width - moving.from.x,
          y: moving.start.y + event.clientY / rect.height - moving.from.y,
        },
        0,
      );
    return dragCrop(
      moving.start,
      moving.handle,
      event.clientX / rect.width - moving.from.x,
      event.clientY / rect.height - moving.from.y,
      {
        width: Math.min(1, Math.max(0.02, MIN_CROP_PIXELS / rect.width)),
        height: Math.min(1, Math.max(0.02, MIN_CROP_PIXELS / rect.height)),
      },
    );
  };

  const pointerDown = (event: ReactPointerEvent<HTMLCanvasElement>) => {
    if (!layout || live.current || event.button !== 0 || tool === "crop") return;
    event.preventDefault();
    if (field) {
      // A press outside the open field commits it, like a blur.
      commitField();
      if (tool === "text") return;
    }
    const point = pointAt(event);
    event.currentTarget.setPointerCapture(event.pointerId);
    if (tool === "draw") {
      track({
        kind: "draw",
        pointer: event.pointerId,
        stroke: { kind: "stroke", color, width: strokePoints / layout.edge, points: [point.x, point.y] },
        last: { x: event.clientX, y: event.clientY },
      });
    } else if (tool === "text") {
      const index = textAt(point);
      const mark = index === undefined ? undefined : (current.marks[index] as TextMark);
      track({
        kind: "text",
        pointer: event.pointerId,
        index,
        from: point,
        origin: mark ? { x: mark.x, y: mark.y } : point,
        moved: false,
      });
    }
  };
  /** Draw's ring follows a hovering mouse or pen; it is moved directly, without re-rendering. */
  const moveBrush = (event: ReactPointerEvent<HTMLCanvasElement>) => {
    const ring = brush.current;
    if (!ring) return;
    if (event.pointerType === "touch") {
      delete ring.dataset.visible;
      return;
    }
    const rect = event.currentTarget.getBoundingClientRect();
    ring.style.translate = `${event.clientX - rect.left}px ${event.clientY - rect.top}px`;
    ring.dataset.visible = "";
  };
  const hideBrush = () => {
    if (brush.current) delete brush.current.dataset.visible;
  };
  const pointerMove = (event: ReactPointerEvent<HTMLCanvasElement>) => {
    if (tool === "draw") moveBrush(event);
    const moving = live.current;
    if (!moving || event.pointerId !== moving.pointer || !layout || !rotated) return;
    if (moving.kind === "draw") {
      const events = event.nativeEvent.getCoalescedEvents?.() ?? [];
      let { last } = moving;
      const points = [...moving.stroke.points];
      for (const step of events.length ? events : [event.nativeEvent]) {
        if (Math.hypot(step.clientX - last.x, step.clientY - last.y) < 1.5) continue;
        const point = pointAt(step);
        points.push(point.x, point.y);
        last = { x: step.clientX, y: step.clientY };
      }
      if (points.length !== moving.stroke.points.length)
        track({ ...moving, stroke: { ...moving.stroke, points }, last });
    } else if (moving.kind === "text" && moving.index !== undefined) {
      const point = pointAt(event);
      const distance = Math.hypot(
        (point.x - moving.from.x) * rotated.width * layout.scale,
        (point.y - moving.from.y) * rotated.height * layout.scale,
      );
      if (!moving.moved && distance <= 4) return;
      if (!moving.moved) track({ ...moving, moved: true });
      setLiveText({ index: moving.index, ...movedText(moving, event) });
    }
  };
  const pointerUp = (event: ReactPointerEvent<HTMLCanvasElement>) => {
    const moving = live.current;
    if (!moving || event.pointerId !== moving.pointer || !layout) return;
    track(undefined);
    setLiveText(undefined);
    if (moving.kind === "draw") {
      commit({ ...current, marks: [...current.marks, moving.stroke] });
    } else if (moving.kind === "text") {
      if (moving.index === undefined) {
        setField({ x: moving.from.x, y: moving.from.y, value: "", color, points: textPoints });
      } else if (moving.moved) {
        const position = movedText(moving, event);
        commit({
          ...current,
          marks: current.marks.map((mark, index) => (index === moving.index ? { ...mark, ...position } : mark)),
        });
      } else {
        const mark = current.marks[moving.index] as TextMark;
        const points = Math.round(mark.size * layout.edge);
        setColor(mark.color);
        setTextPoints(Math.min(TEXT_SIZES.max, Math.max(TEXT_SIZES.min, points)));
        setField({ index: moving.index, x: mark.x, y: mark.y, value: mark.text, color: mark.color, points });
      }
    }
  };
  const pointerCancel = () => {
    track(undefined);
    setLiveText(undefined);
    setLiveCrop(undefined);
  };

  const cropDown = (handle: CropHandle) => (event: ReactPointerEvent<HTMLElement>) => {
    if (event.button !== 0 || !canvas.current || live.current) return;
    event.preventDefault();
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    const rect = canvas.current.getBoundingClientRect();
    track({
      kind: "crop",
      pointer: event.pointerId,
      handle,
      start: current.crop,
      from: { x: event.clientX / rect.width, y: event.clientY / rect.height },
    });
  };
  const cropMove = (event: ReactPointerEvent<HTMLElement>) => {
    const moving = live.current;
    if (moving?.kind !== "crop" || event.pointerId !== moving.pointer || !canvas.current) return;
    setLiveCrop(draggedCrop(moving, event));
  };
  const cropUp = (event: ReactPointerEvent<HTMLElement>) => {
    const moving = live.current;
    if (moving?.kind !== "crop" || event.pointerId !== moving.pointer || !canvas.current) return;
    track(undefined);
    setLiveCrop(undefined);
    commit({ ...current, crop: clampCrop(draggedCrop(moving, event), pictureMode ? 0 : 0.02) });
  };
  const resizePictureCrop = (fraction: number) => {
    if (!rotated) return;
    const side = Math.min(rotated.width, rotated.height) * Math.max(0.1, Math.min(1, fraction));
    const width = side / rotated.width;
    const height = side / rotated.height;
    commit({
      ...current,
      crop: clampCrop(
        {
          x: current.crop.x + (current.crop.width - width) / 2,
          y: current.crop.y + (current.crop.height - height) / 2,
          width,
          height,
        },
        0,
      ),
    });
  };
  const cropKey = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = event.altKey ? 0.002 : 0.01;
    const delta = (
      { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] } as Record<
        string,
        [number, number]
      >
    )[event.key];
    if (!delta) return;
    event.preventDefault();
    event.stopPropagation();
    if (pictureMode && event.shiftKey && rotated) {
      resizePictureCrop(
        (current.crop.width * rotated.width) / Math.min(rotated.width, rotated.height) + delta[0] + delta[1],
      );
      return;
    }
    if (pictureMode) {
      commit({
        ...current,
        crop: clampCrop({ ...current.crop, x: current.crop.x + delta[0], y: current.crop.y + delta[1] }, 0),
      });
      return;
    }
    const min = { width: 0.02, height: 0.02 };
    commit({ ...current, crop: dragCrop(current.crop, event.shiftKey ? "se" : "move", delta[0], delta[1], min) });
  };

  const keyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.nativeEvent.isComposing) return;
    const command = event.metaKey || event.ctrlKey;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      if (field) setField(undefined);
      else onCancel();
    } else if (command && !field && event.key.toLowerCase() === "z") {
      event.preventDefault();
      if (event.shiftKey) redo();
      else undo();
    } else if (command && !field && event.key.toLowerCase() === "y" && !event.metaKey) {
      event.preventDefault();
      redo();
    }
  };

  const chooseTool = (next: Tool) => {
    if (field) commitField();
    setTool(next);
  };
  const chooseColor = (next: string) => {
    const value = next.toUpperCase();
    setColor(value);
    if (field) setField({ ...field, color: value });
  };
  const chooseTextSize = (points: number) => {
    setTextPoints(points);
    if (field) setField({ ...field, points });
  };
  const fieldPosition =
    field && layout && rotated
      ? {
          left: ((field.x - region.x) / region.width) * layout.width,
          top: ((field.y - region.y) / region.height) * layout.height,
        }
      : undefined;
  const crop = liveCrop ?? current.crop;
  const custom = !EDIT_COLORS.some((preset) => preset === color);
  const hint =
    tool === "crop"
      ? pictureMode
        ? "Drag the square to reposition. Adjust Crop size to zoom. Arrow keys move the focused frame; Shift and arrow keys resize it."
        : "Drag the frame’s corners or edges to crop, or inside it to move it. Arrow keys move the focused frame; Shift and arrow keys resize it."
      : tool === "draw"
        ? "Drag on the picture to draw."
        : "Click or tap the picture to add text. Click text to change it, or drag it to move it.";

  return (
    <FloatingPortal root={anchor?.closest<HTMLElement>("dialog") ?? undefined}>
      <FloatingOverlay lockScroll className="chat-editor-overlay">
        <FloatingFocusManager context={context} returnFocus={returnFocus}>
          <div
            ref={(element) => {
              dialog.current = element;
              refs.setFloating(element);
            }}
            className="chat-editor"
            tabIndex={-1}
            aria-labelledby={titleId}
            {...getFloatingProps({ onKeyDown: keyDown })}
          >
            <header className="chat-editor-bar">
              <button
                type="button"
                className="chat-editor-icon"
                aria-label="Cancel editing"
                title="Cancel"
                onClick={onCancel}
              >
                <X size={22} aria-hidden="true" />
              </button>
              <div className="chat-editor-title">
                <strong id={titleId}>
                  {pictureMode ? "Adjust picture" : kind === "video" ? "Edit video" : "Edit image"}
                </strong>
                <small title={name}>{name}</small>
              </div>
              <button
                type="button"
                className="chat-editor-icon"
                aria-label="Undo"
                title="Undo"
                disabled={!canUndo || !picture}
                onClick={undo}
              >
                <Undo2 size={20} aria-hidden="true" />
              </button>
              <button
                type="button"
                className="chat-editor-icon"
                aria-label="Redo"
                title="Redo"
                disabled={!canRedo || !picture}
                onClick={redo}
              >
                <Redo2 size={20} aria-hidden="true" />
              </button>
              <button
                type="button"
                className="chat-editor-reset"
                disabled={!picture || !isEdited(current)}
                onClick={() => {
                  setField(undefined);
                  commit(pictureMode ? history.states[0] : NO_EDIT);
                }}
              >
                Reset
              </button>
              <button
                type="button"
                className="chat-editor-done"
                disabled={!picture || saving}
                onClick={() => void done()}
              >
                Done
              </button>
            </header>
            <div ref={setStage} className="chat-editor-stage">
              {failed ? (
                <p className="chat-editor-message" role="alert">
                  This file can’t be edited here.
                </p>
              ) : !picture || !layout ? (
                <span className="chat-spinner" role="status" aria-label="Loading" />
              ) : (
                <div className="chat-editor-picture" style={{ width: layout.width, height: layout.height }}>
                  <canvas
                    ref={canvas}
                    role="img"
                    aria-label={kind === "video" ? "Video frame being edited" : "Image being edited"}
                    aria-describedby={hintId}
                    data-tool={tool}
                    style={{ width: layout.width, height: layout.height }}
                    onPointerDown={pointerDown}
                    onPointerMove={pointerMove}
                    onPointerUp={pointerUp}
                    onPointerCancel={pointerCancel}
                    onPointerEnter={(event) => tool === "draw" && moveBrush(event)}
                    onPointerLeave={hideBrush}
                  />
                  {tool === "draw" && (
                    <div
                      ref={brush}
                      className="chat-editor-brush"
                      aria-hidden="true"
                      style={{
                        width: Math.max(BRUSH_MIN, strokePoints),
                        height: Math.max(BRUSH_MIN, strokePoints),
                        background: `${color}4D`,
                      }}
                    />
                  )}
                  {tool === "crop" && (
                    <div className="chat-editor-shade" aria-hidden="true">
                      <span
                        style={{
                          left: `${crop.x * 100}%`,
                          top: `${crop.y * 100}%`,
                          width: `${crop.width * 100}%`,
                          height: `${crop.height * 100}%`,
                        }}
                      />
                    </div>
                  )}
                  {tool === "crop" && (
                    <div
                      className="chat-editor-crop"
                      tabIndex={0}
                      role="group"
                      aria-label="Crop frame"
                      aria-describedby={hintId}
                      style={{
                        left: `${crop.x * 100}%`,
                        top: `${crop.y * 100}%`,
                        width: `${crop.width * 100}%`,
                        height: `${crop.height * 100}%`,
                      }}
                      onKeyDown={cropKey}
                      onPointerDown={cropDown("move")}
                      onPointerMove={cropMove}
                      onPointerUp={cropUp}
                      onPointerCancel={pointerCancel}
                    >
                      {(pictureMode ? [] : CROP_HANDLES).map((handle) => (
                        <span
                          key={handle}
                          className="chat-editor-handle"
                          data-handle={handle}
                          aria-hidden="true"
                          onPointerDown={cropDown(handle)}
                          onPointerMove={cropMove}
                          onPointerUp={cropUp}
                          onPointerCancel={pointerCancel}
                        />
                      ))}
                    </div>
                  )}
                  {field && fieldPosition && layout && (
                    <input
                      ref={input}
                      className="chat-editor-text"
                      aria-label={field.index === undefined ? "New text" : "Edit text"}
                      maxLength={MAX_TEXT}
                      value={field.value}
                      enterKeyHint="done"
                      autoComplete="off"
                      spellCheck
                      style={{
                        left: fieldPosition.left,
                        top: fieldPosition.top,
                        color: field.color,
                        fontSize: field.points,
                        width: Math.max(48, textWidth(field.value || "M", field.points) + field.points),
                      }}
                      onChange={(event) => setField({ ...field, value: event.target.value })}
                      onKeyDown={(event) => {
                        if (event.key === "Enter" && !event.nativeEvent.isComposing) {
                          event.preventDefault();
                          commitField();
                        }
                      }}
                      onBlur={(event) => {
                        // Moving to the colour or size controls keeps the field open.
                        if (
                          !(
                            event.relatedTarget instanceof Element &&
                            event.relatedTarget.closest(".chat-editor-options")
                          )
                        )
                          commitField();
                      }}
                    />
                  )}
                </div>
              )}
            </div>
            <p id={hintId} className="sr-only">
              {hint}
            </p>
            {kind === "video" && picture && (
              <p className="chat-editor-hint">Drawings and text stay on screen for the whole video.</p>
            )}
            {pictureMode && (
              <p className="chat-editor-hint">Drag the square to reposition. Adjust crop size to zoom.</p>
            )}
            <div className="chat-editor-tools">
              <div className="chat-editor-options">
                {tool === "crop" ? (
                  <>
                    {pictureMode && rotated && (
                      <div className="chat-editor-size">
                        <Slider
                          label="Crop size"
                          min={10}
                          max={100}
                          value={((current.crop.width * rotated.width) / Math.min(rotated.width, rotated.height)) * 100}
                          onChange={(value) => resizePictureCrop(value / 100)}
                        />
                      </div>
                    )}
                    <button
                      type="button"
                      className="chat-editor-action"
                      disabled={!picture}
                      onClick={() => commit(rotateLeft(current))}
                    >
                      <RotateCcw size={18} aria-hidden="true" />
                      Rotate left
                    </button>
                  </>
                ) : (
                  <>
                    <div className="chat-editor-colors" role="radiogroup" aria-label="Colour">
                      {EDIT_COLORS.map((preset) => (
                        <button
                          key={preset}
                          type="button"
                          role="radio"
                          aria-checked={color === preset}
                          aria-label={COLOR_NAMES[preset]}
                          title={COLOR_NAMES[preset]}
                          className="chat-editor-swatch"
                          style={{ "--swatch": preset } as CSSProperties}
                          onMouseDown={(event) => event.preventDefault()}
                          onClick={() => chooseColor(preset)}
                        />
                      ))}
                      <label
                        className="chat-editor-swatch chat-editor-custom"
                        data-selected={custom || undefined}
                        title="Custom colour"
                        style={custom ? ({ "--swatch": color } as CSSProperties) : undefined}
                      >
                        <input
                          type="color"
                          aria-label="Custom colour"
                          value={color.toLowerCase()}
                          onChange={(event) => chooseColor(event.target.value)}
                        />
                      </label>
                    </div>
                    <div className="chat-editor-size">
                      <span className="chat-editor-size-preview" aria-hidden="true">
                        {tool === "draw" ? (
                          <span
                            className="chat-editor-dot"
                            style={{ width: strokePoints, height: strokePoints, background: color }}
                          />
                        ) : (
                          <span style={{ fontSize: Math.min(textPoints, 36), color }}>Aa</span>
                        )}
                      </span>
                      <Slider
                        label={tool === "draw" ? "Line size" : "Text size"}
                        value={tool === "draw" ? strokePoints : textPoints}
                        min={tool === "draw" ? STROKE_SIZES.min : TEXT_SIZES.min}
                        max={tool === "draw" ? STROKE_SIZES.max : TEXT_SIZES.max}
                        defaultValue={tool === "draw" ? STROKE_SIZES.initial : TEXT_SIZES.initial}
                        onChange={tool === "draw" ? setStrokePoints : chooseTextSize}
                      />
                    </div>
                  </>
                )}
              </div>
              <div className="chat-editor-modes" role="toolbar" aria-label="Editing tools">
                {(
                  [
                    ["crop", "Crop", Crop],
                    ["draw", "Draw", Brush],
                    ["text", "Text", Type],
                  ] as const
                ).map(([value, label, Icon]) => (
                  <button
                    key={value}
                    type="button"
                    aria-pressed={tool === value}
                    disabled={!picture}
                    onClick={() => chooseTool(value)}
                  >
                    <Icon size={18} aria-hidden="true" />
                    {label}
                  </button>
                ))}
              </div>
            </div>
          </div>
        </FloatingFocusManager>
      </FloatingOverlay>
    </FloatingPortal>
  );
}

const COLOR_NAMES: Record<string, string> = {
  "#E5484D": "Red",
  "#F5C542": "Yellow",
  "#4CAF50": "Green",
  "#3B82F6": "Blue",
  "#FFFFFF": "White",
  "#000000": "Black",
};
