import { useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
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
import { Eye, EyeOff, FileText, FileWarning, Maximize2, Play, X } from "lucide-react";
import GifImage from "./GifImage.tsx";
import MediaViewer from "./MediaViewer.tsx";
import { MediaPreviewContext } from "./MediaPreviewContext.ts";
import { attachmentView, type ChatAttachment } from "./types.ts";
import { formatBytes, type UploadState } from "./uploads.ts";

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

/** The sender's own copy of a just-sent file, shown until the result arrives. */
export interface LocalPreview {
  url: string;
  video: boolean;
}

function FileCard({ attachment }: { attachment: ChatAttachment }) {
  const body = (
    <>
      <FileText aria-hidden="true" size={18} />
      <span>
        <strong>{attachment.name}</strong>
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
        aria-label={`${attachment.name}: ${processingLabel(percent)}`}
      >
        <span className="chat-spinner" aria-hidden="true" />
        <span>
          <strong>{attachment.name}</strong>
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
  onView?: (attachmentId: string, anchor: HTMLElement) => void;
}) {
  // Ask for fresh URLs once per attachment if a long-open tab outlives them.
  const reported = useRef(new Set<string>());
  const [revealed, setRevealed] = useState<ReadonlySet<string>>(() => new Set());
  if (!attachments.length) return null;
  const expired = (attachment: ChatAttachment) => {
    if (reported.current.has(attachment.id)) return;
    reported.current.add(attachment.id);
    onExpired?.([attachment.id]);
  };
  return (
    <div className="chat-attachments">
      {attachments.map((attachment) => {
        const size = frame(attachment);
        const view = attachmentView(attachment);
        if (attachment.spoiler && !revealed.has(attachment.id)) {
          const hiddenSize = size ?? DEFAULT_FRAME;
          const local = localPreviews[attachment.id];
          const visual = attachment.kind === "image" || attachment.kind === "video";
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
              style={{ width: hiddenSize.width, aspectRatio: hiddenSize.width / hiddenSize.height }}
              aria-label="Reveal spoiler attachment"
              onClick={() => setRevealed((current) => new Set([...current, attachment.id]))}
            >
              {thumbnail && (
                <span className="chat-spoiler-preview" aria-hidden="true">
                  <MediaPreviewContext value={true}>
                    <GifImage src={thumbnail} alt="" onError={() => expired(attachment)} />
                  </MediaPreviewContext>
                </span>
              )}
              <span className="chat-spoiler-label">Spoiler</span>
            </button>
          );
        }
        if (view === "processing")
          return (
            <Processing
              key={attachment.id}
              attachment={attachment}
              percent={progress[attachment.id]}
              local={localPreviews[attachment.id]}
            />
          );
        if (view === "failed") {
          return (
            <div key={attachment.id} className="chat-file" data-failed>
              <FileWarning aria-hidden="true" size={18} />
              <span>
                <strong>{attachment.name}</strong>
                <small>Couldn’t process this file</small>
              </span>
            </div>
          );
        }
        if (view === "image") {
          const gif = attachment.contentType === "image/gif";
          const Image = gif ? GifImage : "img";
          return (
            <a
              key={attachment.id}
              className="chat-media"
              href={attachment.url}
              target="_blank"
              rel="noopener noreferrer"
              style={size && { width: size.width, aspectRatio: size.width / size.height }}
              aria-haspopup={onView ? "dialog" : undefined}
              onClick={(event) => {
                // Modified clicks keep the link's new tab or window.
                if (!onView || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey)
                  return;
                event.preventDefault();
                onView(attachment.id, event.currentTarget);
              }}
            >
              <Image
                src={gif ? attachment.url : (attachment.previewUrl ?? attachment.url)}
                alt={attachment.name}
                loading="lazy"
                decoding="async"
                width={size?.width}
                height={size?.height}
                onError={() => expired(attachment)}
              />
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
              onView={onView && ((anchor) => onView(attachment.id, anchor))}
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
                    onView(attachment.id, anchor);
                  }}
                />
              )}
            </div>
          );
        }
        if (view === "audio") {
          return (
            <figure key={attachment.id} className="chat-audio">
              <figcaption>{attachment.name}</figcaption>
              <audio controls preload="metadata" src={attachment.url} onError={() => expired(attachment)} />
            </figure>
          );
        }
        return <FileCard key={attachment.id} attachment={attachment} />;
      })}
    </div>
  );
}

export interface DraftAttachment extends UploadState {
  key: string;
  spoiler?: boolean;
  /** A thread's files stay separate from the channel and other thread drafts. */
  threadRootId?: string;
  /** Kept locally until Send, which prepares and uploads it (profile cards). */
  staged?: boolean;
  /** Local file for previewing before sending, also used by the pending row. */
  localUrl?: string;
  localKind?: "image" | "video";
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
        title={draft.error ?? draft.name}
        onPointerDown={(event) => {
          cancelPress();
          suppressClick.current = false;
          if (!onRemove || !event.isPrimary || event.button !== 0) return;
          const target = event.target as HTMLElement;
          if (target.closest(".chat-draft-remove, .chat-draft-retry, .chat-draft-spoiler")) return;
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

/** Without `onRemove` (while a message is sending) files cannot be removed;
 * without `onRetry` a failed upload has no chip-level Retry. */
export function DraftAttachments({
  drafts,
  onRemove,
  onRetry,
  onToggleSpoiler,
}: {
  drafts: DraftAttachment[];
  onRemove?: (key: string) => void;
  onRetry?: (key: string) => void;
  onToggleSpoiler?: (key: string) => void;
}) {
  const [viewing, setViewing] = useState<{ key: string; anchor: HTMLElement }>();
  const closeViewer = useCallback(() => setViewing(undefined), []);
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
  const uploading = drafts.filter((draft) => !draft.attachment && !draft.error && !draft.staged).length;
  return (
    <>
      {(failed > 0 || uploading > 0) && (
        <p className="chat-draft-notice" data-error={failed > 0 || undefined} role="status">
          {failed > 0 ? (
            <FileWarning size={16} aria-hidden="true" />
          ) : (
            <span className="chat-spinner" aria-hidden="true" />
          )}
          <span>
            {failed > 0
              ? `${failed} ${failed === 1 ? "file" : "files"} failed to upload. Send will retry ${failed === 1 ? "it" : "them"}, or you can remove ${failed === 1 ? "it" : "them"}.`
              : `Uploading ${uploading} ${uploading === 1 ? "file" : "files"}… Your message won’t send until every file is ready.`}
          </span>
        </p>
      )}
      <ul className="chat-drafts" aria-label="Files to send">
        {drafts.map((draft) => {
          const url = draft.localUrl ?? draft.attachment?.url;
          const kind = draft.localKind ?? draft.attachment?.kind;
          const thumbnail = kind === "image" ? (draft.localUrl ?? draft.attachment?.previewUrl ?? url) : undefined;
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
              {kind === "video" ? (
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
              {draft.spoiler && <span className="chat-draft-spoiler-label">Spoiler</span>}
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
              {onRemove && (
                <button
                  type="button"
                  className="chat-draft-remove"
                  aria-label={`Remove ${draft.name}`}
                  onClick={() => onRemove(draft.key)}
                >
                  <X size={14} aria-hidden="true" />
                </button>
              )}
            </DraftAttachmentTile>
          );
        })}
      </ul>
      {viewing && media.some((attachment) => attachment.id === viewing.key) && (
        <MediaViewer attachments={media} attachmentId={viewing.key} anchor={viewing.anchor} onClose={closeViewer} />
      )}
    </>
  );
}
