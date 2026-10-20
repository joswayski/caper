import {
  lazy,
  Suspense,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactElement,
  type ReactNode,
} from "react";
import {
  autoUpdate,
  flip,
  FloatingFocusManager,
  FloatingPortal,
  offset,
  shift,
  useDismiss,
  useFloating,
  useInteractions,
  useRole,
} from "@floating-ui/react";
import { Eye, EyeOff, FileText, FileWarning, Maximize2, Pencil, Play, X } from "lucide-react";
import GifImage from "./GifImage.tsx";
import MediaViewer, { type ViewAttachment, type ViewerEditing } from "./MediaViewer.tsx";
import { MediaPreviewContext } from "./MediaPreviewContext.ts";
import type { AttachmentEdit, EditTool } from "./edit.ts";
import { attachmentView, type ChatAttachment } from "./types.ts";
import { formatBytes, type UploadState } from "./uploads.ts";
import { attachmentLabel, isVoiceAttachment } from "./voice.ts";
import { VoiceMessage, VoicePlayer } from "./VoiceMessage.tsx";

const AttachmentEditor = lazy(() => import("./AttachmentEditor.tsx"));

const MAX_WIDTH = 360;
const MAX_HEIGHT = 300;
/** Placeholder for a processing image/video whose dimensions are not known yet. */
const DEFAULT_FRAME = { width: 240, height: 160 };

/** Display size that reserves layout space before the image loads. */
function frame(attachment: ChatAttachment) {
  const { width, height } = attachment;
  if (!width || !height) return undefined;
  const scale = Math.min(1, MAX_WIDTH / width, MAX_HEIGHT / height);
  return { width: Math.round(width * scale), height: Math.round(height * scale) };
}

/** Count, not media dimensions or readiness, owns the sent gallery's layout. */
function AttachmentGallery({ items }: { items: ReactElement[] }) {
  const count = items.length;
  const rows = count <= 3 ? (count === 3 ? 2 : 1) : Math.ceil(count / 3);
  const shortRows = rows * 3 - count;
  const cells =
    count <= 3
      ? items.map((_, index) => ({
          row: count === 3 && index === 2 ? 2 : 1,
          column: index === 0 ? 1 : 2,
          span: 1,
          rowSpan: count === 3 && index === 0 ? 2 : 1,
          left: index === 0,
          right: count === 2 ? index === 1 : index > 0,
          top: index < 2,
          bottom: count === 2 || index !== 1,
        }))
      : Array.from({ length: rows }, (_, row) => {
          const columns = row < shortRows ? 2 : 3;
          return Array.from({ length: columns }, (_, column) => ({
            row: row + 1,
            column: column * (6 / columns) + 1,
            span: 6 / columns,
            rowSpan: 1,
            left: column === 0,
            right: column === columns - 1,
            top: row === 0,
            bottom: row === rows - 1,
          }));
        }).flat();
  const height =
    count <= 3 ? "66.666667%" : count === 4 ? "100%" : `calc((100% - 8px) / 3 * ${rows} + ${(rows - 1) * 4}px)`;
  return (
    <div className="chat-gallery" role="group" aria-label={`${count} attachments`}>
      <div className="chat-gallery-layout" style={{ paddingBottom: height }}>
        <div
          className="chat-gallery-grid"
          style={{
            gridTemplateColumns: count === 3 ? "2fr 1fr" : `repeat(${count === 2 ? 2 : 6}, minmax(0, 1fr))`,
            gridTemplateRows: `repeat(${rows}, minmax(0, 1fr))`,
          }}
        >
          {items.map((item, index) => {
            const cell = cells[index];
            const corners = [
              cell.left && cell.top,
              cell.right && cell.top,
              cell.right && cell.bottom,
              cell.left && cell.bottom,
            ];
            return (
              <div
                key={item.key}
                className="chat-gallery-tile"
                style={{
                  gridColumn: `${cell.column} / span ${cell.span}`,
                  gridRow: `${cell.row} / span ${cell.rowSpan}`,
                  borderRadius: corners.map((outer) => (outer ? "12px" : "1px")).join(" "),
                }}
              >
                {item}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

/** The sender's own copy, retained for this conversation's mounted lifetime. */
export interface LocalPreview {
  url: string;
  video: boolean;
}

function FileCard({ attachment }: { attachment: ChatAttachment }) {
  const body = (
    <>
      <FileText aria-hidden="true" size={18} />
      <span>
        <strong>{attachmentLabel(attachment)}</strong>
        <small>{attachment.unavailable ? "File removed" : formatBytes(attachment.size)}</small>
      </span>
    </>
  );
  return attachment.url && !attachment.unavailable ? (
    <a className="chat-file" href={attachment.url} target="_blank" rel="noopener noreferrer">
      {body}
    </a>
  ) : (
    <div className="chat-file" data-unavailable={attachment.unavailable || undefined}>
      {body}
    </div>
  );
}

function processingLabel(percent: number | undefined) {
  return percent === undefined ? "Processing…" : `Processing… ${percent}%`;
}

function Processing({
  attachment,
  percent,
  local,
}: {
  attachment: ChatAttachment;
  percent?: number;
  local?: LocalPreview;
}) {
  const visual = attachment.kind === "image" || attachment.kind === "video" || !!attachment.previewUrl || !!local;
  if (!visual) {
    return (
      <div
        className="chat-file"
        data-processing
        role="status"
        aria-label={`${attachmentLabel(attachment)}: ${processingLabel(percent)}`}
      >
        <span className="chat-spinner" aria-hidden="true" />
        <span>
          <strong>{attachmentLabel(attachment)}</strong>
          <small>{processingLabel(percent)}</small>
        </span>
      </div>
    );
  }
  const size = frame(attachment) ?? DEFAULT_FRAME;
  return (
    <div
      className="chat-media chat-processing"
      role="status"
      aria-label={`${attachment.name}: ${processingLabel(percent)}`}
      style={{ width: size.width, aspectRatio: `${size.width} / ${size.height}` }}
    >
      {local ? (
        local.video ? (
          <video src={local.url} muted playsInline preload="metadata" aria-hidden="true" />
        ) : (
          <img src={local.url} alt="" decoding="async" />
        )
      ) : (
        attachment.previewUrl && <img src={attachment.previewUrl} alt="" decoding="async" />
      )}
      <span className="chat-processing-status">
        <span className="chat-spinner" aria-hidden="true" />
        {processingLabel(percent)}
      </span>
    </div>
  );
}

/** Opens a video in the viewer; the inline player keeps its own controls. */
function ExpandButton({ name, onClick }: { name: string; onClick: (anchor: HTMLElement) => void }) {
  return (
    <button
      type="button"
      className="chat-media-expand"
      aria-label={`Open ${name} in viewer`}
      title="Open in viewer"
      aria-haspopup="dialog"
      onClick={(event) => onClick(event.currentTarget)}
    >
      <Maximize2 size={16} aria-hidden="true" />
    </button>
  );
}

/** GIF-like playback for animated files: muted, looping, no controls. With
 * reduced motion it stays paused until clicked. */
function AnimatedVideo({
  attachment,
  size,
  onError,
  onView,
}: {
  attachment: ChatAttachment;
  size?: { width: number; height: number };
  onError: () => void;
  onView?: (anchor: HTMLElement) => void;
}) {
  const ref = useRef<HTMLVideoElement>(null);
  const preview = useContext(MediaPreviewContext);
  const [paused, setPaused] = useState(true);
  useEffect(() => {
    const video = ref.current;
    if (!video) return;
    video.muted = true;
    const motion = window.matchMedia("(prefers-reduced-motion: reduce)");
    const apply = () => {
      if (preview || motion.matches) video.pause();
      else void video.play().catch(() => undefined);
    };
    apply();
    motion.addEventListener("change", apply);
    return () => motion.removeEventListener("change", apply);
  }, [attachment.url, preview]);
  const toggle = () => {
    const video = ref.current;
    if (!video || preview) return;
    if (video.paused) void video.play().catch(() => undefined);
    else video.pause();
  };
  return (
    <div
      className="chat-media chat-animated"
      style={size && { width: size.width, aspectRatio: size.width / size.height }}
    >
      <video
        ref={ref}
        src={attachment.url}
        poster={attachment.previewUrl}
        muted
        loop
        playsInline
        preload="auto"
        aria-label={attachment.name}
        onClick={toggle}
        onPlay={() => setPaused(false)}
        onPause={() => setPaused(true)}
        onError={onError}
      />
      {paused && !preview && (
        <button type="button" className="chat-animated-play" aria-label={`Play ${attachment.name}`} onClick={toggle}>
          <Play size={18} aria-hidden="true" />
        </button>
      )}
      {onView && <ExpandButton name={attachment.name} onClick={onView} />}
    </div>
  );
}

export function MessageAttachments({
  attachments,
  progress = {},
  localPreviews = {},
  onExpired,
  onView,
}: {
  attachments: ChatAttachment[];
  progress?: Record<string, number>;
  localPreviews?: Record<string, LocalPreview>;
  onExpired?: (ids: string[]) => void;
  /** Opens a sent image or video in the viewer; pending rows leave it unset. */
  onView?: ViewAttachment;
}) {
  // Ask for fresh URLs once per attachment if a long-open tab outlives them.
  const reported = useRef(new Set<string>());
  const [revealed, setRevealed] = useState<ReadonlySet<string>>(() => new Set());
  const root = useRef<HTMLDivElement>(null);
  const focusSpoiler = useRef<string>(undefined);
  useLayoutEffect(() => {
    const id = focusSpoiler.current;
    if (!id) return;
    root.current?.querySelector<HTMLElement>(`[data-spoiler-id="${CSS.escape(id)}"]`)?.focus({ preventScroll: true });
    focusSpoiler.current = undefined;
  }, [revealed]);
  const hideSpoiler = (id: string) => {
    focusSpoiler.current = id;
    setRevealed((current) => {
      const next = new Set(current);
      next.delete(id);
      return next;
    });
  };
  const viewAttachment = (attachment: ChatAttachment, anchor: HTMLElement) =>
    onView?.(attachment.id, anchor, attachment.spoiler ? hideSpoiler : undefined);
  // Some originals (e.g. HEIC) cannot be decoded here, even though the server
  // can convert them. Do not keep a broken local copy over a usable result.
  const [failedLocalPreviews, setFailedLocalPreviews] = useState(() => new Set<string>());
  if (!attachments.length) return null;
  const expired = (attachment: ChatAttachment) => {
    if (reported.current.has(attachment.id)) return;
    reported.current.add(attachment.id);
    onExpired?.([attachment.id]);
  };
  const renderVisibleAttachment = (attachment: ChatAttachment) => {
    const size = frame(attachment);
    const view = attachmentView(attachment);
    const local = failedLocalPreviews.has(attachment.id) ? undefined : localPreviews[attachment.id];
    const processing = view === "processing";
    if (processing && !(attachment.kind === "image" || (local && !local.video)))
      return <Processing key={attachment.id} attachment={attachment} percent={progress[attachment.id]} local={local} />;
    if (view === "failed") {
      return (
        <div key={attachment.id} className="chat-file" data-failed>
          <FileWarning aria-hidden="true" size={18} />
          <span>
            <strong>{attachmentLabel(attachment)}</strong>
            <small>
              {visuals.length > 1 && (attachment.kind === "image" || attachment.kind === "video")
                ? "Processing failed"
                : "Couldn’t process this file"}
            </small>
          </span>
        </div>
      );
    }
    // Keep the same image node and decoded local source through acceptance
    // and processing. The viewer/link still opens the server's final file.
    if (view === "image" || processing) {
      const gif = attachment.contentType === "image/gif";
      const Image = gif ? GifImage : "img";
      const src =
        local && !local.video
          ? local.url
          : gif && !processing
            ? attachment.url
            : (attachment.previewUrl ?? attachment.url);
      const imageSize = size ?? (processing ? DEFAULT_FRAME : undefined);
      return (
        <a
          key={attachment.id}
          className={`chat-media${processing ? " chat-processing" : ""}`}
          href={processing ? undefined : attachment.url}
          target={processing ? undefined : "_blank"}
          rel={processing ? undefined : "noopener noreferrer"}
          style={imageSize && { width: imageSize.width, aspectRatio: imageSize.width / imageSize.height }}
          role={processing ? "status" : undefined}
          aria-label={processing ? `${attachment.name}: ${processingLabel(progress[attachment.id])}` : undefined}
          aria-haspopup={!processing && onView ? "dialog" : undefined}
          onClick={(event) => {
            // Modified clicks keep the link's new tab or window.
            if (
              processing ||
              !onView ||
              event.button !== 0 ||
              event.metaKey ||
              event.ctrlKey ||
              event.shiftKey ||
              event.altKey
            )
              return;
            event.preventDefault();
            viewAttachment(attachment, event.currentTarget);
          }}
        >
          {src && (
            <Image
              src={src}
              alt={attachment.name}
              loading={local ? "eager" : "lazy"}
              decoding="async"
              width={imageSize?.width}
              height={imageSize?.height}
              onError={() => {
                if (local && !local.video) setFailedLocalPreviews((current) => new Set(current).add(attachment.id));
                else if (!processing) expired(attachment);
              }}
            />
          )}
          {processing && (
            <span className="chat-processing-status">
              <span className="chat-spinner" aria-hidden="true" />
              {processingLabel(progress[attachment.id])}
            </span>
          )}
        </a>
      );
    }
    if (view === "animated")
      return (
        <AnimatedVideo
          key={attachment.id}
          attachment={attachment}
          size={size}
          onError={() => expired(attachment)}
          onView={onView && ((anchor) => viewAttachment(attachment, anchor))}
        />
      );
    if (view === "video") {
      return (
        <div
          key={attachment.id}
          className={size ? "chat-media chat-video" : "chat-media chat-video intrinsic"}
          style={size && { width: size.width, aspectRatio: size.width / size.height }}
        >
          <video
            controls
            playsInline
            preload="metadata"
            poster={attachment.previewUrl}
            src={attachment.url}
            aria-label={attachment.name}
            onError={() => expired(attachment)}
          />
          {onView && (
            <ExpandButton
              name={attachment.name}
              onClick={(anchor) => {
                // The viewer plays its own copy; don't play both.
                anchor.parentElement?.querySelector("video")?.pause();
                viewAttachment(attachment, anchor);
              }}
            />
          )}
        </div>
      );
    }
    if (view === "audio" && isVoiceAttachment(attachment))
      return <VoiceMessage key={attachment.id} attachment={attachment} onError={() => expired(attachment)} />;
    if (view === "audio") {
      return (
        <figure key={attachment.id} className="chat-audio">
          <figcaption>{attachment.name}</figcaption>
          <audio controls preload="metadata" src={attachment.url} onError={() => expired(attachment)} />
        </figure>
      );
    }
    return <FileCard key={attachment.id} attachment={attachment} />;
  };
  const renderAttachment = (attachment: ChatAttachment) => {
    if (!attachment.spoiler) return renderVisibleAttachment(attachment);
    const size = frame(attachment) ?? DEFAULT_FRAME;
    const visual = attachment.kind === "image" || attachment.kind === "video";
    if (revealed.has(attachment.id)) {
      return (
        <div
          key={attachment.id}
          className="chat-spoiler-revealed"
          data-visual={visual || undefined}
          style={visual ? { width: size.width } : undefined}
        >
          {renderVisibleAttachment(attachment)}
          <button
            type="button"
            className="chat-spoiler-hide"
            data-spoiler-id={attachment.id}
            onClick={() => hideSpoiler(attachment.id)}
          >
            <EyeOff size={16} aria-hidden="true" />
            Hide spoiler
          </button>
        </div>
      );
    }
    const local = localPreviews[attachment.id];
    const thumbnail =
      visual && !attachment.unavailable && attachment.status !== "failed"
        ? (attachment.previewUrl ??
          (local && !local.video ? local.url : undefined) ??
          (attachment.kind === "image" ? attachment.url : undefined))
        : undefined;
    return (
      <button
        key={attachment.id}
        type="button"
        className="chat-media chat-spoiler"
        style={{ width: size.width, aspectRatio: size.width / size.height }}
        aria-label="Reveal spoiler attachment"
        data-spoiler-id={attachment.id}
        onClick={() => {
          focusSpoiler.current = attachment.id;
          setRevealed((current) => new Set([...current, attachment.id]));
        }}
      >
        {thumbnail && (
          <span className="chat-spoiler-preview" aria-hidden="true">
            <MediaPreviewContext value={true}>
              <GifImage src={thumbnail} alt="" onError={() => expired(attachment)} />
            </MediaPreviewContext>
          </span>
        )}
        <span className="chat-spoiler-label" aria-hidden="true">
          <EyeOff size={24} />
          <strong>Spoiler</strong>
          <span className="chat-spoiler-click">Click to reveal</span>
          <span className="chat-spoiler-tap">Tap to reveal</span>
        </span>
      </button>
    );
  };
  // Keep files/audio readable instead of squeezing their controls into photo cells.
  const visuals = attachments.filter((attachment) => attachment.kind === "image" || attachment.kind === "video");
  const files = attachments.filter((attachment) => attachment.kind !== "image" && attachment.kind !== "video");
  return (
    <div ref={root} className="chat-attachments">
      {visuals.length > 1 ? <AttachmentGallery items={visuals.map(renderAttachment)} /> : visuals.map(renderAttachment)}
      {files.length > 0 && <div className="chat-attachment-files">{files.map(renderAttachment)}</div>}
    </div>
  );
}

export interface DraftAttachment extends UploadState {
  key: string;
  /** A recorded voice message staged with the files: shown as a voice chip. */
  voice?: { durationMs: number; waveform?: string };
  spoiler?: boolean;
  /** A thread's files stay separate from the channel and other thread drafts. */
  threadRootId?: string;
  /** Kept locally until Send, which prepares and uploads it (profile cards). */
  staged?: boolean;
  /** Local file for previewing before sending, also used by the pending row.
   * An edited image's is a render of the edit; an edited video has none (the
   * upload is its preview). */
  localUrl?: string;
  localKind?: "image" | "video";
  /** What the editor can open this file as, once known. */
  editable?: "image" | "video";
  /** Edits the file will be prepared with; the original is kept. */
  edit?: AttachmentEdit;
  /** A render of the edited picture for the tile (edited videos). */
  thumbnailUrl?: string;
}

/** Opening a draft in the editor, and keeping what it produces. */
export interface DraftEditing {
  /** The original file, unless it is no longer available. */
  source: (key: string) => Blob | undefined;
  /** `edit` is undefined when the file goes back to its original. */
  save: (key: string, edit: AttachmentEdit | undefined, preview?: Blob) => void;
}

/** Decode one early frame, then release the decoder instead of keeping a
 * player alive for every draft. This also works before a staged file uploads. */
function DraftVideoThumbnail({ url, poster }: { url?: string; poster?: string }) {
  const [frame, setFrame] = useState<{ url: string; image: string }>();
  useEffect(() => {
    if (!url || poster) return;
    const video = document.createElement("video");
    const release = () => {
      clearTimeout(timer);
      video.onloadeddata = video.onseeked = video.onerror = null;
      video.removeAttribute("src");
      video.load();
    };
    const capture = () => {
      try {
        if (!video.videoWidth || !video.videoHeight) return;
        const scale = Math.min(1, 312 / Math.max(video.videoWidth, video.videoHeight));
        const canvas = document.createElement("canvas");
        canvas.width = Math.max(1, Math.round(video.videoWidth * scale));
        canvas.height = Math.max(1, Math.round(video.videoHeight * scale));
        const context = canvas.getContext("2d");
        if (!context) return;
        context.drawImage(video, 0, 0, canvas.width, canvas.height);
        setFrame({ url, image: canvas.toDataURL("image/jpeg", 0.8) });
      } catch {
        // Unsupported media or cross-origin pixels keep the labelled fallback.
      } finally {
        release();
      }
    };
    const timer = setTimeout(release, 8_000);
    video.muted = true;
    video.playsInline = true;
    video.preload = "auto";
    video.onloadeddata = () => {
      video.onloadeddata = null;
      const time = Number.isFinite(video.duration) && video.duration > 0 ? Math.min(0.5, video.duration / 2) : 0;
      if (time > 0) {
        video.onseeked = capture;
        video.currentTime = time;
      } else capture();
    };
    video.onerror = release;
    video.src = url;
    return release;
  }, [url, poster]);
  const thumbnail = poster ?? (frame?.url === url ? frame?.image : undefined);
  return thumbnail ? <img src={thumbnail} alt="" draggable={false} /> : null;
}

/** Keep the long-press lifetime tied to its tile. Scrolling cancels the press;
 * releasing after the menu opens must not also open the media viewer. */
function DraftAttachmentTile({
  draft,
  onRemove,
  children,
}: {
  draft: DraftAttachment;
  onRemove?: (key: string) => void;
  children: ReactNode;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  const press = useRef<{ timer: ReturnType<typeof setTimeout>; x: number; y: number }>(undefined);
  const suppressClick = useRef(false);
  const returnFocus = useRef<HTMLElement | null>(null);
  const cancelPress = useCallback(() => {
    if (press.current) clearTimeout(press.current.timer);
    press.current = undefined;
  }, []);
  useEffect(() => cancelPress, [cancelPress]);
  useEffect(() => {
    if (!onRemove) {
      cancelPress();
      setMenuOpen(false);
    }
  }, [onRemove, cancelPress]);
  const open = menuOpen && !!onRemove;
  const { refs, floatingStyles, context } = useFloating({
    open,
    onOpenChange: setMenuOpen,
    placement: "top-end",
    strategy: "fixed",
    middleware: [offset(6), flip(), shift({ padding: 12 })],
    whileElementsMounted: autoUpdate,
  });
  const { getFloatingProps } = useInteractions([useDismiss(context), useRole(context, { role: "menu" })]);
  return (
    <>
      <li
        ref={refs.setReference}
        data-error={draft.error ? true : undefined}
        data-spoiler={draft.spoiler || undefined}
        data-voice={draft.voice ? true : undefined}
        title={draft.error ?? draft.name}
        onPointerDown={(event) => {
          cancelPress();
          suppressClick.current = false;
          if (!onRemove || !event.isPrimary || event.button !== 0) return;
          const target = event.target as HTMLElement;
          if (target.closest(".chat-draft-actions, .chat-draft-retry, .chat-voice")) return;
          returnFocus.current = event.currentTarget.querySelector<HTMLElement>(".chat-draft-preview");
          press.current = {
            x: event.clientX,
            y: event.clientY,
            timer: setTimeout(() => {
              suppressClick.current = true;
              setMenuOpen(true);
            }, 500),
          };
        }}
        onPointerMove={(event) => {
          const current = press.current;
          if (current && Math.hypot(event.clientX - current.x, event.clientY - current.y) > 10) cancelPress();
        }}
        onPointerUp={cancelPress}
        onPointerCancel={cancelPress}
        onPointerLeave={cancelPress}
        onClickCapture={(event) => {
          if (suppressClick.current) {
            event.preventDefault();
            event.stopPropagation();
            suppressClick.current = false;
          }
        }}
        onContextMenu={(event) => {
          if (!onRemove) return;
          event.preventDefault();
          cancelPress();
          returnFocus.current = event.currentTarget.querySelector<HTMLElement>(".chat-draft-preview");
          setMenuOpen(true);
        }}
        onKeyDown={(event) => {
          if (!onRemove || (event.key !== "ContextMenu" && !(event.shiftKey && event.key === "F10"))) return;
          event.preventDefault();
          returnFocus.current = event.target as HTMLElement;
          setMenuOpen(true);
        }}
      >
        {children}
      </li>
      {open && (
        <FloatingPortal>
          <FloatingFocusManager context={context} returnFocus={returnFocus}>
            <div
              ref={refs.setFloating}
              style={floatingStyles}
              className="chat-message-actions chat-draft-menu"
              aria-label={`Attachment options for ${draft.name}`}
              {...getFloatingProps()}
            >
              <div className="chat-copy-actions">
                <button
                  type="button"
                  role="menuitem"
                  onClick={() => {
                    setMenuOpen(false);
                    onRemove?.(draft.key);
                  }}
                >
                  <X size={16} aria-hidden="true" />
                  Remove attachment
                </button>
              </div>
            </div>
          </FloatingFocusManager>
        </FloatingPortal>
      )}
    </>
  );
}

/** Without `onRemove` (while a message is sending) files cannot be removed,
 * edited or marked as spoilers; without `onRetry` a failed upload has no
 * chip-level Retry. */
export function DraftAttachments({
  drafts,
  onRemove,
  onRetry,
  onToggleSpoiler,
  editing,
}: {
  drafts: DraftAttachment[];
  onRemove?: (key: string) => void;
  onRetry?: (key: string) => void;
  onToggleSpoiler?: (key: string) => void;
  editing?: DraftEditing;
}) {
  const [viewing, setViewing] = useState<{ key: string; anchor: HTMLElement }>();
  const closeViewer = useCallback(() => setViewing(undefined), []);
  const [editingKey, setEditingKey] = useState<{ key: string; anchor: HTMLElement; tool?: EditTool }>();
  const closeEditor = useCallback(() => setEditingKey(undefined), []);
  const edited = editingKey && drafts.find((draft) => draft.key === editingKey.key);
  const editedSource = edited?.editable && editing?.source(edited.key);
  // The viewer offers the editor's tools for the same files the pencil does.
  const viewerEditing: ViewerEditing | undefined =
    editing && onRemove
      ? {
          editable: (key) => !!drafts.find((draft) => draft.key === key)?.editable && !!editing.source(key),
          open: (key, tool) => {
            if (!viewing) return;
            setViewing(undefined);
            setEditingKey({ key, anchor: viewing.anchor, tool });
          },
        }
      : undefined;
  const media: ChatAttachment[] = drafts.flatMap((draft) => {
    const kind = draft.localUrl ? draft.localKind : draft.attachment?.kind;
    const url = draft.localUrl ?? draft.attachment?.url;
    if (!url || (kind !== "image" && kind !== "video")) return [];
    return [
      {
        ...draft.attachment,
        id: draft.key,
        name: draft.name,
        kind,
        contentType: draft.attachment?.contentType ?? `${kind}/*`,
        // The size that will be sent, once known; the viewer omits a zero.
        size: draft.storedSize ?? draft.attachment?.size ?? 0,
        url,
        status: draft.localUrl ? "ready" : draft.attachment?.status,
      },
    ];
  });
  if (!drafts.length) return null;
  const failed = drafts.filter((draft) => draft.error).length;
  return (
    <>
      {failed > 0 && (
        <p className="chat-draft-notice" data-error={true} role="status">
          <FileWarning size={16} aria-hidden="true" />
          <span>
            {`${failed} ${failed === 1 ? "file" : "files"} failed to upload. Send will retry ${failed === 1 ? "it" : "them"}, or you can remove ${failed === 1 ? "it" : "them"}.`}
          </span>
        </p>
      )}
      <ul className="chat-drafts" aria-label="Files to send">
        {drafts.map((draft) => {
          const url = draft.localUrl ?? draft.attachment?.url;
          const kind = draft.localKind ?? draft.attachment?.kind;
          const thumbnail =
            draft.thumbnailUrl ??
            (kind === "image" ? (draft.localUrl ?? draft.attachment?.previewUrl ?? url) : undefined);
          const busy = !draft.attachment && !draft.error && !draft.staged;
          const status = draft.error
            ? "Failed"
            : !busy
              ? undefined
              : draft.storedSize === undefined
                ? draft.compressing !== undefined
                  ? `Prep ${Math.round(draft.compressing * 100)}%`
                  : "Preparing…"
                : `Uploading ${Math.round(draft.progress * 100)}%`;
          const content = (
            <>
              {kind === "video" && !draft.thumbnailUrl ? (
                <DraftVideoThumbnail url={url} poster={draft.attachment?.previewUrl} />
              ) : thumbnail ? (
                draft.spoiler ? (
                  <MediaPreviewContext value={true}>
                    <GifImage src={thumbnail} alt="" draggable={false} />
                  </MediaPreviewContext>
                ) : (
                  <img src={thumbnail} alt="" draggable={false} />
                )
              ) : (
                <span className="chat-draft-kind" aria-hidden="true">
                  {kind === "audio" ? <Play size={24} /> : <FileText size={24} />}
                  {kind === "image" ? "Image" : kind === "audio" ? "Audio" : "File"}
                </span>
              )}
            </>
          );
          const viewable = media.some((attachment) => attachment.id === draft.key);
          if (draft.voice)
            return (
              <DraftAttachmentTile key={draft.key} draft={draft} onRemove={onRemove}>
                <VoicePlayer
                  compact
                  attachment={{
                    url: draft.localUrl ?? draft.attachment?.url,
                    durationMs: draft.voice.durationMs,
                    waveform: draft.voice.waveform,
                  }}
                />
                {busy && <progress max={1} value={draft.progress} aria-label={`Uploading ${draft.name}`} />}
                {draft.error && onRetry && (
                  <button
                    type="button"
                    className="chat-draft-retry"
                    aria-label={`Retry uploading ${draft.name}`}
                    title={draft.error}
                    onClick={() => onRetry(draft.key)}
                  >
                    Retry upload
                  </button>
                )}
                {onRemove && (
                  <div className="chat-draft-actions">
                    <button
                      type="button"
                      className="chat-draft-remove"
                      aria-label="Remove voice message"
                      title="Remove"
                      onClick={() => onRemove(draft.key)}
                    >
                      <X size={14} aria-hidden="true" />
                    </button>
                  </div>
                )}
              </DraftAttachmentTile>
            );
          return (
            <DraftAttachmentTile key={draft.key} draft={draft} onRemove={onRemove}>
              {viewable ? (
                <button
                  type="button"
                  className="chat-draft-preview"
                  aria-label={`Preview ${draft.name}`}
                  aria-haspopup="dialog"
                  onClick={(event) => setViewing({ key: draft.key, anchor: event.currentTarget })}
                >
                  {content}
                </button>
              ) : url ? (
                <a
                  className="chat-draft-preview"
                  href={url}
                  target="_blank"
                  rel="noopener noreferrer"
                  aria-label={`Open ${draft.name}`}
                >
                  {content}
                </a>
              ) : (
                <div className="chat-draft-preview" aria-label={draft.name}>
                  {content}
                </div>
              )}
              {kind === "video" && !draft.spoiler && (
                <span
                  className="chat-draft-video-play"
                  role="img"
                  aria-label="Video"
                  data-status={!!status || undefined}
                  data-retry={!!(draft.error && onRetry) || undefined}
                >
                  <Play size={16} fill="currentColor" aria-hidden="true" />
                </span>
              )}
              {status && !(draft.error && onRetry) && (
                <small className="chat-draft-status" aria-label={draft.error ?? status}>
                  {status}
                </small>
              )}
              {busy && <progress max={1} value={draft.progress} aria-label={`Uploading ${draft.name}`} />}
              {draft.error && onRetry && (
                <button
                  type="button"
                  className="chat-draft-retry"
                  aria-label={`Retry uploading ${draft.name}`}
                  title={draft.error}
                  onClick={() => onRetry(draft.key)}
                >
                  Retry upload
                </button>
              )}
              {draft.spoiler && (
                <span className="chat-draft-spoiler-label">
                  <EyeOff size={18} aria-hidden="true" />
                  Spoiler
                </span>
              )}
              {(onToggleSpoiler || onRemove) && (
                // One bar in the corner, like Discord's: Spoiler, Edit (when the editor can open the file), then Remove.
                <div className="chat-draft-actions">
                  {onToggleSpoiler && (
                    <button
                      type="button"
                      className="chat-draft-spoiler"
                      aria-label={`Spoiler attachment: ${draft.name}`}
                      aria-pressed={!!draft.spoiler}
                      title={draft.spoiler ? "Remove spoiler" : "Mark as spoiler"}
                      onClick={() => onToggleSpoiler(draft.key)}
                    >
                      {draft.spoiler ? <EyeOff size={14} aria-hidden="true" /> : <Eye size={14} aria-hidden="true" />}
                    </button>
                  )}
                  {onRemove && editing && draft.editable && (
                    <button
                      type="button"
                      className="chat-draft-edit"
                      aria-label={`Edit ${draft.name}`}
                      title="Edit"
                      aria-haspopup="dialog"
                      onClick={(event) => setEditingKey({ key: draft.key, anchor: event.currentTarget })}
                    >
                      <Pencil size={14} aria-hidden="true" />
                    </button>
                  )}
                  {onRemove && (
                    <button
                      type="button"
                      className="chat-draft-remove"
                      aria-label={`Remove ${draft.name}`}
                      title="Remove"
                      onClick={() => onRemove(draft.key)}
                    >
                      <X size={14} aria-hidden="true" />
                    </button>
                  )}
                </div>
              )}
            </DraftAttachmentTile>
          );
        })}
      </ul>
      {viewing && media.some((attachment) => attachment.id === viewing.key) && (
        <MediaViewer
          attachments={media}
          attachmentId={viewing.key}
          anchor={viewing.anchor}
          editing={viewerEditing}
          onClose={closeViewer}
        />
      )}
      {edited?.editable && editedSource && editing && onRemove && (
        <Suspense fallback={null}>
          <AttachmentEditor
            key={edited.key}
            file={editedSource}
            kind={edited.editable}
            name={edited.name}
            edit={edited.edit}
            tool={editingKey?.tool}
            anchor={editingKey?.anchor}
            onCancel={closeEditor}
            onSave={(edit, preview) => {
              closeEditor();
              editing.save(edited.key, edit, preview);
            }}
          />
        </Suspense>
      )}
    </>
  );
}
