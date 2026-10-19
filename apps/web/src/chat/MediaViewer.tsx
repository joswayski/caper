import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import {
  Brush,
  ChevronLeft,
  ChevronRight,
  Crop,
  Download,
  Ellipsis,
  ExternalLink,
  EyeOff,
  Type,
  X,
} from "lucide-react";
import {
  FloatingFocusManager,
  FloatingOverlay,
  FloatingPortal,
  useFloating,
  useInteractions,
  useRole,
} from "@floating-ui/react";
import { movesOf, Velocity } from "../components/swipe.ts";
import type { EditTool } from "./edit.ts";
import type { ChatAttachment } from "./types.ts";
import { formatBytes } from "./uploads.ts";
import {
  FIT,
  clampView,
  fitSize,
  swipeAction,
  toggleZoom,
  viewerPosition,
  viewerStep,
  zoomAt,
  type Point,
  type Size,
  type ZoomView,
} from "./viewer.ts";

/** A file to show: its message, and the thumbnail or button that gets focus back. */
export interface ViewerTarget {
  messageId: string;
  attachmentId: string;
  anchor: HTMLElement;
  /** The file is on the message's forwarded original. */
  forwarded?: boolean;
  /** Re-cover a spoiler in the row that opened this viewer. */
  onHideSpoiler?: (attachmentId: string) => void;
}

export type ViewAttachment = (
  attachmentId: string,
  anchor: HTMLElement,
  onHideSpoiler?: ViewerTarget["onHideSpoiler"],
) => void;

interface Gesture {
  /** Where the gesture's single pointer started, in client pixels. */
  start: Point;
  view: ZoomView;
  pinch?: { distance: number; centre: Point };
  /** After a pinch the remaining finger pans; it never swipes. */
  pinched?: boolean;
  axis?: "x" | "y";
  /** Finger speed along each axis, for flings. */
  speed: { x: Velocity; y: Velocity };
}

/** The bottom of a player with controls is its seek bar and buttons, which keep their own drags. */
const PLAYER_CONTROLS = 64;

/** One file, fitted to the stage. Keyed by file, so zoom resets on navigation. */
function ViewerItem({
  attachment,
  enter,
  failed,
  onError,
  onStep,
  onClose,
}: {
  attachment: ChatAttachment;
  /** Paging here, it arrives from the side it was swiped or stepped in from. */
  enter?: 1 | -1;
  failed: boolean;
  onError: () => void;
  onStep: (step: 1 | -1) => void;
  onClose: () => void;
}) {
  const stage = useRef<HTMLDivElement>(null);
  const frame = useRef<HTMLDivElement>(null);
  const [box, setBox] = useState<Size>();
  const [measured, setMeasured] = useState<Size>();
  const [loaded, setLoaded] = useState(false);
  const [view, setView] = useState(FIT);
  const [drag, setDrag] = useState<Point>();
  const pointers = useRef(new Map<number, Point>());
  const gesture = useRef<Gesture>(undefined);
  const moved = useRef(false);
  const pointerType = useRef("");
  const lastTap = useRef<{ time: number; point: Point }>(undefined);
  const image = attachment.kind === "image";
  const animated = !image && !!attachment.animated;
  // The loaded file's own size wins over the stored one, so it never stretches.
  const natural =
    measured ??
    (attachment.width && attachment.height ? { width: attachment.width, height: attachment.height } : undefined);
  // Players fill the box; images and GIF-like loops stay at most their own size.
  const size = natural && box ? fitSize(natural, box, !image && !animated) : undefined;
  // Re-limit the pan when the box changes size, as when a phone rotates.
  const shown = size && box ? clampView(view, size, box) : FIT;

  useLayoutEffect(() => {
    const element = frame.current;
    if (!element) return;
    const measure = () =>
      setBox((current) =>
        current?.width === element.clientWidth && current.height === element.clientHeight
          ? current
          : { width: element.clientWidth, height: element.clientHeight },
      );
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  /** A client position relative to the frame's centre, where the media is centred. */
  const local = (x: number, y: number): Point => {
    const rect = frame.current!.getBoundingClientRect();
    return { x: x - rect.left - rect.width / 2, y: y - rect.top - rect.height / 2 };
  };

  // React's wheel listener is passive; zooming must keep the page from zooming too.
  useEffect(() => {
    const element = stage.current;
    if (!element || !image || !size || !box) return;
    const wheel = (event: WheelEvent) => {
      event.preventDefault();
      const delta = event.deltaMode === WheelEvent.DOM_DELTA_LINE ? event.deltaY * 16 : event.deltaY;
      const point = local(event.clientX, event.clientY);
      // Trackpad pinches arrive as ctrl+wheel with small deltas.
      setView((current) =>
        zoomAt(current, point, current.scale * Math.exp(-delta * (event.ctrlKey ? 0.01 : 0.002)), size, box),
      );
    };
    element.addEventListener("wheel", wheel, { passive: false });
    return () => element.removeEventListener("wheel", wheel);
  }, [image, size?.width, size?.height, box?.width, box?.height]);

  const speeds = () => ({ x: new Velocity(), y: new Velocity() });
  const pinchOf = () => {
    const [a, b] = [...pointers.current.values()];
    return {
      distance: Math.max(1, Math.hypot(b!.x - a!.x, b!.y - a!.y)),
      centre: local((a!.x + b!.x) / 2, (a!.y + b!.y) / 2),
    };
  };
  const pointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    // Buttons and the player's seek bar handle their own presses; the rest of
    // a player still swipes to page or close, like an image.
    const target = event.target as Element;
    const player = target.closest("video[controls]");
    if (
      target.closest("button, a") ||
      (player && event.clientY > player.getBoundingClientRect().bottom - PLAYER_CONTROLS)
    )
      return;
    // Only images pinch; videos follow one pointer.
    if (pointers.current.size && !image) return;
    pointerType.current = event.pointerType;
    pointers.current.set(event.pointerId, { x: event.clientX, y: event.clientY });
    if (pointers.current.size === 1) {
      moved.current = false;
      gesture.current = { start: { x: event.clientX, y: event.clientY }, view: shown, speed: speeds() };
    } else if (pointers.current.size === 2) {
      moved.current = true;
      setDrag(undefined);
      gesture.current = {
        start: { x: event.clientX, y: event.clientY },
        view: shown,
        pinch: pinchOf(),
        pinched: true,
        speed: speeds(),
      };
    }
  };
  const pointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    const current = gesture.current;
    if (!current || !pointers.current.has(event.pointerId)) return;
    // The button was released outside the window.
    if (event.pointerType === "mouse" && !event.buttons) return pointerEnd(event);
    pointers.current.set(event.pointerId, { x: event.clientX, y: event.clientY });
    if (current.pinch && pointers.current.size > 1) {
      if (!size || !box) return;
      const { distance, centre } = pinchOf();
      const zoomed = zoomAt(
        current.view,
        current.pinch.centre,
        current.view.scale * (distance / current.pinch.distance),
        size,
        box,
      );
      setView(
        clampView(
          {
            ...zoomed,
            x: zoomed.x + centre.x - current.pinch.centre.x,
            y: zoomed.y + centre.y - current.pinch.centre.y,
          },
          size,
          box,
        ),
      );
      return;
    }
    const dx = event.clientX - current.start.x;
    const dy = event.clientY - current.start.y;
    for (const point of movesOf(event.nativeEvent)) {
      current.speed.x.add(point.timeStamp, point.clientX);
      current.speed.y.add(point.timeStamp, point.clientY);
    }
    if (Math.hypot(dx, dy) > 10) moved.current = true;
    if (current.view.scale > 1 && size && box)
      setView(clampView({ ...current.view, x: current.view.x + dx, y: current.view.y + dy }, size, box));
    else if (event.pointerType !== "mouse" && !current.pinched && moved.current) {
      current.axis ??= Math.abs(dx) > Math.abs(dy) ? "x" : "y";
      setDrag(current.axis === "x" ? { x: dx, y: 0 } : { x: 0, y: Math.max(0, dy) });
    }
  };
  const pointerEnd = (event: ReactPointerEvent<HTMLDivElement>) => {
    const current = gesture.current;
    if (!current || !pointers.current.delete(event.pointerId)) return;
    const [rest] = pointers.current.values();
    if (rest) {
      // Lifting one finger of a pinch carries on as a pan from here.
      gesture.current = { start: rest, view: shown, pinched: true, speed: speeds() };
      return;
    }
    gesture.current = undefined;
    setDrag(undefined);
    if (event.type !== "pointerup") return;
    if (current.axis) {
      current.speed[current.axis].add(event.timeStamp, current.axis === "x" ? event.clientX : event.clientY);
      const velocity = current.speed[current.axis].speed(event.timeStamp);
      const action =
        current.axis === "x"
          ? swipeAction(event.clientX - current.start.x, 0, velocity)
          : swipeAction(0, event.clientY - current.start.y, velocity);
      if (action === "close") onClose();
      else if (action) onStep(action === "next" ? 1 : -1);
      return;
    }
    // Touch double-tap; mice and pens use dblclick.
    if (
      event.pointerType !== "touch" ||
      moved.current ||
      !size ||
      !box ||
      !(event.target as Element).closest(".chat-viewer-image")
    )
      return;
    const point = local(event.clientX, event.clientY);
    const tap = lastTap.current;
    if (tap && event.timeStamp - tap.time < 300 && Math.hypot(point.x - tap.point.x, point.y - tap.point.y) < 30) {
      lastTap.current = undefined;
      setView(toggleZoom(shown, point, size, box));
    } else lastTap.current = { time: event.timeStamp, point };
  };

  const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  return (
    <div
      ref={stage}
      className="chat-viewer-stage"
      onPointerDown={pointerDown}
      onPointerMove={pointerMove}
      onPointerUp={pointerEnd}
      onPointerCancel={pointerEnd}
      onClick={(event) => {
        // The dark area around the media closes, unless the press was a drag.
        if (event.target === event.currentTarget && !moved.current) onClose();
      }}
    >
      <div ref={frame} className="chat-viewer-frame">
        {failed ? (
          <p className="chat-viewer-message">Couldn’t load this file.</p>
        ) : (
          <div
            className="chat-viewer-media"
            data-dragging={drag ? "" : undefined}
            data-enter={enter === 1 ? "next" : enter === -1 ? "previous" : undefined}
            style={drag && { transform: `translate(${drag.x}px, ${drag.y}px)` }}
          >
            {image ? (
              <div
                className="chat-viewer-image"
                data-zoomed={shown.scale > 1 || undefined}
                style={{
                  width: size?.width ?? 0,
                  height: size?.height ?? 0,
                  transform: `translate(${shown.x}px, ${shown.y}px) scale(${shown.scale})`,
                }}
                onDoubleClick={(event) => {
                  if (pointerType.current === "touch" || !size || !box) return;
                  setView(toggleZoom(shown, local(event.clientX, event.clientY), size, box));
                }}
              >
                {!loaded && attachment.previewUrl && <img src={attachment.previewUrl} alt="" decoding="async" />}
                <img
                  src={attachment.url}
                  alt={attachment.name}
                  decoding="async"
                  draggable={false}
                  onLoad={(event) => {
                    const { naturalWidth: width, naturalHeight: height } = event.currentTarget;
                    setMeasured({ width, height });
                    setLoaded(true);
                  }}
                  onError={onError}
                />
              </div>
            ) : (
              <video
                src={attachment.url}
                poster={attachment.previewUrl}
                aria-label={attachment.name}
                playsInline
                // GIF-like files loop silently; with reduced motion they wait for Play.
                controls={!animated || reducedMotion}
                autoPlay={!animated || !reducedMotion}
                loop={animated}
                muted={animated}
                style={size ?? { maxWidth: box?.width, maxHeight: box?.height }}
                onLoadedMetadata={(event) => {
                  const { videoWidth: width, videoHeight: height } = event.currentTarget;
                  if (width && height) setMeasured({ width, height });
                }}
                onError={onError}
              />
            )}
          </div>
        )}
        {image && !failed && !loaded && !(size && attachment.previewUrl) && (
          <span className="chat-spinner" role="status" aria-label="Loading" />
        )}
      </div>
    </div>
  );
}

const EDIT_TOOLS = [
  { tool: "crop", label: "Crop", Icon: Crop },
  { tool: "draw", label: "Draw", Icon: Brush },
  { tool: "text", label: "Text", Icon: Type },
] as const;

/** Opening the editor from the viewer, for files not sent yet. */
export interface ViewerEditing {
  editable: (id: string) => boolean;
  /** Called instead of closing; the caller closes the viewer and opens the editor on `tool`. */
  open: (id: string, tool: EditTool) => void;
}

/**
 * Full-window viewer for one message's images and videos, like Discord's.
 * Escape, the close button, the dark area, swiping down and (on phones) Back close it.
 */
export default function MediaViewer({
  attachments: allAttachments,
  attachmentId,
  anchor,
  author,
  sentAt,
  editing,
  onClose,
  onExpired,
  onHideSpoiler,
}: {
  /** The message's files, with fresh URLs where the caller has them. */
  attachments: ChatAttachment[];
  attachmentId: string;
  anchor: HTMLElement;
  author?: string;
  sentAt?: string;
  /** Staged files the editor can open show its tools under the picture. */
  editing?: ViewerEditing;
  /** Keep this stable: on phones it owns a history entry. */
  onClose: () => void;
  onExpired?: (ids: string[]) => void;
  onHideSpoiler?: ViewerTarget["onHideSpoiler"];
}) {
  /** The editor takes this viewer's history entry over, so closing for it must not go back. */
  const handedToEditor = useRef(false);
  // Opening one file must not reveal another spoiler by paging through it.
  const attachments = allAttachments.filter((attachment) => !attachment.spoiler || attachment.id === attachmentId);
  const [currentId, setCurrentId] = useState(attachmentId);
  /** Which way the last page turn went, so the next file arrives from that side. */
  const [entering, setEntering] = useState<1 | -1>();
  // URLs that failed to load. A refreshed URL is a new key, so it loads again.
  const [failed, setFailed] = useState<ReadonlySet<string>>(() => new Set());
  const reported = useRef(new Set<string>());
  const returnFocus = useRef(anchor);
  const closeButton = useRef<HTMLButtonElement>(null);
  const actionsMenu = useRef<HTMLDetailsElement>(null);
  const titleId = useId();
  const counterId = useId();
  const { refs, context } = useFloating({ open: true });
  const { getFloatingProps } = useInteractions([useRole(context)]);
  const { items, index } = viewerPosition(attachments, currentId);
  const current = items[index];
  const named = current ?? attachments.find((attachment) => attachment.id === currentId);
  const previous = viewerStep(attachments, currentId, -1);
  const next = viewerStep(attachments, currentId, 1);
  const step = (direction: 1 | -1) => {
    const id = direction === 1 ? next : previous;
    if (!id) return;
    setEntering(direction);
    setCurrentId(id);
  };
  const date = sentAt ? new Date(sentAt) : undefined;
  const time =
    date && !Number.isNaN(date.valueOf())
      ? new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" }).format(date)
      : undefined;
  // What download costs; message files always carry their stored size.
  const size = current?.size ? formatBytes(current.size) : undefined;

  useEffect(() => {
    const dismiss = (event: PointerEvent) => {
      const menu = actionsMenu.current;
      if (menu?.open && event.target instanceof Node && !menu.contains(event.target)) menu.open = false;
    };
    document.addEventListener("pointerdown", dismiss);
    return () => document.removeEventListener("pointerdown", dismiss);
  }, []);

  useEffect(() => {
    // Back closes the viewer on phones and tablets, like the thread panel.
    if (!window.matchMedia("(max-width: 760px), (pointer: coarse)").matches) return;
    const key = crypto.randomUUID();
    // A viewer remounting elsewhere (pins opening or closing under it) takes over the entry its
    // last mount left, rather than pushing a second one that the first's Back would then close.
    if (window.history.state?.caperViewer)
      window.history.replaceState({ ...window.history.state, caperViewer: key }, "");
    else window.history.pushState({ ...window.history.state, caperViewer: key }, "");
    const back = () => {
      if (window.history.state?.caperViewer !== key) onClose();
    };
    window.addEventListener("popstate", back);
    return () => {
      window.removeEventListener("popstate", back);
      // Left until the next task, so a remount in the same commit can take the entry over.
      setTimeout(() => {
        if (!handedToEditor.current && window.history.state?.caperViewer === key) window.history.back();
      });
    };
  }, [onClose]);

  const expired = (attachment: ChatAttachment) => {
    setFailed((urls) => new Set(urls).add(attachment.url!));
    // Ask for fresh URLs once per file; a long-open tab can outlive them.
    if (reported.current.has(attachment.id)) return;
    reported.current.add(attachment.id);
    onExpired?.([attachment.id]);
  };
  const keyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape" && !event.nativeEvent.isComposing) {
      // Close only the viewer, not the pins or forward dialog, thread panel or live window beneath.
      event.preventDefault();
      event.stopPropagation();
      onClose();
    } else if (
      (event.key === "ArrowLeft" || event.key === "ArrowRight") &&
      !event.altKey &&
      !event.ctrlKey &&
      !event.metaKey &&
      !event.shiftKey &&
      // A focused player seeks instead.
      !(event.target instanceof HTMLMediaElement)
    ) {
      event.preventDefault();
      step(event.key === "ArrowRight" ? 1 : -1);
    }
  };
  return (
    <FloatingPortal>
      <FloatingOverlay lockScroll className="chat-viewer-overlay">
        <FloatingFocusManager context={context} initialFocus={closeButton} returnFocus={returnFocus}>
          <div
            ref={refs.setFloating}
            className="chat-viewer"
            tabIndex={-1}
            aria-label={current?.kind === "video" ? "Video viewer" : "Image viewer"}
            aria-describedby={items.length > 1 && current ? `${titleId} ${counterId}` : titleId}
            {...getFloatingProps({ onKeyDown: keyDown })}
          >
            <header className="chat-viewer-bar">
              <div className="chat-viewer-title">
                <strong id={titleId} title={named?.name}>
                  {named?.name ?? "File removed"}
                </strong>
                {(author || time || size) && (
                  <small>
                    {(author || time) && (
                      <span>
                        {author}
                        {author && time && " · "}
                        {time && <time dateTime={sentAt}>{time}</time>}
                      </span>
                    )}
                    {/* Kept whole on narrow screens; the author and time truncate first. */}
                    {size && (
                      <span className="chat-viewer-size">
                        {(author || time) && " · "}
                        {size}
                      </span>
                    )}
                  </small>
                )}
              </div>
              {items.length > 1 && current && (
                <span id={counterId} className="chat-viewer-counter" aria-live="polite" aria-atomic="true">
                  <span aria-hidden="true">
                    {index + 1} / {items.length}
                  </span>
                  <span className="sr-only">
                    {index + 1} of {items.length}
                  </span>
                </span>
              )}
              {current && (
                <>
                  {/* The CDN is another origin, so browsers may open the file instead of saving it. */}
                  <a
                    className="chat-viewer-desktop-action"
                    href={current.url}
                    download={current.name}
                    target="_blank"
                    rel="noopener noreferrer"
                    aria-label="Download"
                    title="Download"
                  >
                    <Download size={20} aria-hidden="true" />
                  </a>
                  <a
                    className="chat-viewer-desktop-action"
                    href={current.url}
                    target="_blank"
                    rel="noopener noreferrer"
                    aria-label="Open in browser"
                    title="Open in browser"
                  >
                    <ExternalLink size={20} aria-hidden="true" />
                  </a>
                  <details
                    key={current.id}
                    ref={actionsMenu}
                    className="chat-viewer-actions"
                    onBlur={(event) => {
                      // Safari may blur the summary without focusing the tapped link.
                      // Outside taps are handled separately; keep the link until its click.
                      if (event.relatedTarget && !event.currentTarget.contains(event.relatedTarget))
                        event.currentTarget.open = false;
                    }}
                    onKeyDown={(event) => {
                      if (!event.currentTarget.open) return;
                      event.stopPropagation();
                      if (event.key === "Escape") {
                        event.preventDefault();
                        event.currentTarget.open = false;
                        event.currentTarget.querySelector("summary")?.focus();
                      }
                    }}
                  >
                    <summary aria-label="More media actions" title="More media actions">
                      <Ellipsis size={20} aria-hidden="true" />
                    </summary>
                    <div
                      className="chat-viewer-actions-panel"
                      onClick={() => {
                        if (actionsMenu.current) actionsMenu.current.open = false;
                        actionsMenu.current?.querySelector("summary")?.focus();
                      }}
                    >
                      <a href={current.url} download={current.name} target="_blank" rel="noopener noreferrer">
                        <Download size={20} aria-hidden="true" />
                        Download
                      </a>
                      <a href={current.url} target="_blank" rel="noopener noreferrer">
                        <ExternalLink size={20} aria-hidden="true" />
                        Open in browser
                      </a>
                    </div>
                  </details>
                </>
              )}
              <button ref={closeButton} type="button" aria-label="Close viewer" title="Close" onClick={onClose}>
                <X size={22} aria-hidden="true" />
              </button>
            </header>
            {current?.spoiler && onHideSpoiler && (
              <button
                type="button"
                className="chat-spoiler-hide chat-viewer-spoiler"
                onClick={() => {
                  onHideSpoiler(current.id);
                  onClose();
                }}
              >
                <EyeOff size={16} aria-hidden="true" />
                Hide spoiler
              </button>
            )}
            {current ? (
              <ViewerItem
                key={current.id}
                attachment={current}
                enter={entering}
                failed={failed.has(current.url!)}
                onError={() => expired(current)}
                onStep={step}
                onClose={onClose}
              />
            ) : (
              <div
                className="chat-viewer-stage"
                onClick={(event) => {
                  if (event.target === event.currentTarget) onClose();
                }}
              >
                <div className="chat-viewer-frame">
                  <p className="chat-viewer-message">File removed</p>
                </div>
              </div>
            )}
            {current && editing?.editable(current.id) && (
              <div className="chat-viewer-tools" role="group" aria-label={`Edit ${current.name}`}>
                {EDIT_TOOLS.map(({ tool, label, Icon }) => (
                  <button
                    key={tool}
                    type="button"
                    aria-haspopup="dialog"
                    onClick={() => {
                      handedToEditor.current = true;
                      editing.open(current.id, tool);
                    }}
                  >
                    <Icon size={18} aria-hidden="true" />
                    {label}
                  </button>
                ))}
              </div>
            )}
            {(previous || next) && (
              <>
                {/* aria-disabled keeps focus on the button at either end. */}
                <button
                  type="button"
                  className="chat-viewer-nav"
                  data-direction="previous"
                  aria-label="Previous file"
                  aria-disabled={!previous}
                  onClick={() => step(-1)}
                >
                  <ChevronLeft size={24} aria-hidden="true" />
                </button>
                <button
                  type="button"
                  className="chat-viewer-nav"
                  data-direction="next"
                  aria-label="Next file"
                  aria-disabled={!next}
                  onClick={() => step(1)}
                >
                  <ChevronRight size={24} aria-hidden="true" />
                </button>
              </>
            )}
          </div>
        </FloatingFocusManager>
      </FloatingOverlay>
    </FloatingPortal>
  );
}
