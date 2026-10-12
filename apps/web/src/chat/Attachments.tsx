import { useCallback, useEffect, useRef, useState } from "react";
import { FileText, FileWarning, Maximize2, Play, X } from "lucide-react";
import MediaViewer from "./MediaViewer.tsx";
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
  const [paused, setPaused] = useState(true);
  useEffect(() => {
    const video = ref.current;
    if (!video) return;
    video.muted = true;
    const motion = window.matchMedia("(prefers-reduced-motion: reduce)");
    const apply = () => {
      if (motion.matches) video.pause();
      else void video.play().catch(() => undefined);
    };
    apply();
    motion.addEventListener("change", apply);
    return () => motion.removeEventListener("change", apply);
  }, [attachment.url]);
  const toggle = () => {
    const video = ref.current;
    if (!video) return;
    if (video.paused) void video.play().catch(() => undefined);
    else video.pause();
  };
  return (
    <div className="chat-media chat-animated" style={size}>
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
      {paused && (
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
          return (
            <a
              key={attachment.id}
              className="chat-media"
              href={attachment.url}
              target="_blank"
              rel="noopener noreferrer"
              style={size}
              aria-haspopup={onView ? "dialog" : undefined}
              onClick={(event) => {
                // Modified clicks keep the link's new tab or window.
                if (!onView || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey)
                  return;
                event.preventDefault();
                onView(attachment.id, event.currentTarget);
              }}
            >
              <img
                src={attachment.previewUrl ?? attachment.url}
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
              style={size}
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
  /** Kept locally until Send, which prepares and uploads it (profile cards). */
  staged?: boolean;
  /** Local file for previewing before sending, also used by the pending row. */
  localUrl?: string;
  localKind?: "image" | "video";
}

/** Without `onRemove` (while a message is sending) files cannot be removed;
 * without `onRetry` a failed upload has no chip-level Retry. */
export function DraftAttachments({
  drafts,
  onRemove,
  onRetry,
}: {
  drafts: DraftAttachment[];
  onRemove?: (key: string) => void;
  onRetry?: (key: string) => void;
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
  return (
    <>
      <ul className="chat-drafts" aria-label="Files to send">
        {drafts.map((draft) => {
          const content = (
            <>
              {draft.localUrl && draft.localKind === "image" ? (
                <img src={draft.localUrl} alt="" />
              ) : (
                <FileText aria-hidden="true" size={18} />
              )}
              <span>
                <strong title={draft.name}>{draft.name}</strong>
                <small>
                  {draft.error ??
                    (draft.staged ? (
                      formatBytes(draft.sourceSize)
                    ) : draft.attachment ? (
                      <>
                        {draft.storedSize !== undefined && draft.storedSize < draft.sourceSize && (
                          <span className="chat-draft-source-size">{formatBytes(draft.sourceSize)} → </span>
                        )}
                        {formatBytes(draft.storedSize ?? draft.sourceSize)}
                      </>
                    ) : draft.storedSize === undefined ? (
                      draft.compressing !== undefined ? (
                        `Compressing… ${Math.round(draft.compressing * 100)}%`
                      ) : (
                        "Preparing…"
                      )
                    ) : (
                      `Uploading… ${Math.round(draft.progress * 100)}%`
                    ))}
                </small>
              </span>
            </>
          );
          const url = draft.localUrl ?? draft.attachment?.url;
          const viewable = media.some((attachment) => attachment.id === draft.key);
          return (
            <li key={draft.key} data-error={draft.error ? true : undefined}>
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
                <div className="chat-draft-preview">{content}</div>
              )}
              {!draft.attachment && !draft.error && !draft.staged && (
                <progress max={1} value={draft.progress} aria-label={`Uploading ${draft.name}`} />
              )}
              {draft.error && onRetry && (
                <button
                  type="button"
                  className="chat-draft-retry"
                  aria-label={`Retry uploading ${draft.name}`}
                  onClick={() => onRetry(draft.key)}
                >
                  Retry
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
            </li>
          );
        })}
      </ul>
      {viewing && media.some((attachment) => attachment.id === viewing.key) && (
        <MediaViewer attachments={media} attachmentId={viewing.key} anchor={viewing.anchor} onClose={closeViewer} />
      )}
    </>
  );
}
