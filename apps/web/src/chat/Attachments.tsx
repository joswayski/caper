import { useRef } from "react";
import { FileText, X } from "lucide-react";
import type { ChatAttachment } from "./types.ts";
import { formatBytes } from "./uploads.ts";

const MAX_WIDTH = 360;
const MAX_HEIGHT = 300;

/** Display size that reserves layout space before the image loads. */
function frame(attachment: ChatAttachment) {
  const { width, height } = attachment;
  if (!width || !height) return undefined;
  const scale = Math.min(1, MAX_WIDTH / width, MAX_HEIGHT / height);
  return { width: Math.round(width * scale), height: Math.round(height * scale) };
}

function FileCard({ attachment }: { attachment: ChatAttachment }) {
  const body = <><FileText aria-hidden="true" size={18} /><span><strong>{attachment.name}</strong><small>{attachment.unavailable ? "File removed" : formatBytes(attachment.size)}</small></span></>;
  return attachment.url && !attachment.unavailable
    ? <a className="chat-file" href={attachment.url} target="_blank" rel="noopener noreferrer">{body}</a>
    : <div className="chat-file" data-unavailable={attachment.unavailable || undefined}>{body}</div>;
}

export function MessageAttachments({ attachments, onExpired }: { attachments: ChatAttachment[]; onExpired?: (ids: string[]) => void }) {
  // Ask for fresh URLs once per attachment if a long-open tab outlives them.
  const reported = useRef(new Set<string>());
  if (!attachments.length) return null;
  const expired = (attachment: ChatAttachment) => {
    if (reported.current.has(attachment.id)) return;
    reported.current.add(attachment.id);
    onExpired?.([attachment.id]);
  };
  return <div className="chat-attachments">
    {attachments.map((attachment) => {
      const size = frame(attachment);
      if (attachment.unavailable || !attachment.url) return <FileCard key={attachment.id} attachment={attachment} />;
      if (attachment.kind === "image") {
        return <a key={attachment.id} className="chat-media" href={attachment.url} target="_blank" rel="noopener noreferrer" style={size}>
          <img src={attachment.previewUrl ?? attachment.url} alt={attachment.name} loading="lazy" decoding="async" width={size?.width} height={size?.height} onError={() => expired(attachment)} />
        </a>;
      }
      if (attachment.kind === "video") {
        return <video key={attachment.id} className="chat-media" controls playsInline preload="metadata" poster={attachment.previewUrl} src={attachment.url} style={size} aria-label={attachment.name} onError={() => expired(attachment)} />;
      }
      if (attachment.kind === "audio") {
        return <figure key={attachment.id} className="chat-audio">
          <figcaption>{attachment.name}</figcaption>
          <audio controls preload="metadata" src={attachment.url} onError={() => expired(attachment)} />
        </figure>;
      }
      return <FileCard key={attachment.id} attachment={attachment} />;
    })}
  </div>;
}

export interface DraftAttachment {
  key: string;
  name: string;
  kind: ChatAttachment["kind"];
  localUrl?: string;
  sourceSize: number;
  storedSize?: number;
  progress: number;
  /** Set while a video is being re-encoded before upload. */
  compressing?: number;
  error?: string;
  attachment?: ChatAttachment;
}

export function DraftAttachments({ drafts, onRemove }: { drafts: DraftAttachment[]; onRemove: (key: string) => void }) {
  if (!drafts.length) return null;
  return <ul className="chat-drafts" aria-label="Files to send">
    {drafts.map((draft) => <li key={draft.key} data-error={draft.error ? true : undefined}>
      {draft.localUrl && draft.kind === "image" ? <img src={draft.localUrl} alt="" /> : <FileText aria-hidden="true" size={18} />}
      <span>
        <strong title={draft.name}>{draft.name}</strong>
        <small>{draft.error ?? (draft.attachment
          ? draft.storedSize && draft.storedSize < draft.sourceSize ? `${formatBytes(draft.sourceSize)} → ${formatBytes(draft.storedSize)}` : formatBytes(draft.storedSize ?? draft.sourceSize)
          : draft.storedSize === undefined ? draft.compressing !== undefined ? `Compressing… ${Math.round(draft.compressing * 100)}%` : "Preparing…"
          : `Uploading… ${Math.round(draft.progress * 100)}%`)}</small>
      </span>
      {!draft.attachment && !draft.error && <progress max={1} value={draft.progress} aria-label={`Uploading ${draft.name}`} />}
      <button type="button" aria-label={`Remove ${draft.name}`} onClick={() => onRemove(draft.key)}><X size={14} aria-hidden="true" /></button>
    </li>)}
  </ul>;
}
