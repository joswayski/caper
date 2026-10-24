import { useCallback, useContext, useEffect, useRef, useState } from "react";
import GifImage from "./GifImage.tsx";
import { gifLabel, gifOf } from "./klipy.ts";
import { MediaPreviewContext } from "./MediaPreviewContext.ts";
import MediaViewer from "./MediaViewer.tsx";
import type { ChatAttachment } from "./types.ts";

/** Longer-edge caps: GIFs share the inline media size, stickers stay small. */
const GIF_EDGE = 300;
const STICKER_EDGE = 160;

/**
 * A message's KLIPY GIF or sticker, loaded straight from KLIPY as its terms
 * require. GIFs prefer the silent looping MP4; stickers keep WebP for their
 * transparency. Reduced motion shows KLIPY's still (or the first frame).
 */
export default function MessageGif({ content }: { content: { gif?: unknown } }) {
  const gif = gifOf(content);
  const video = useRef<HTMLVideoElement>(null);
  const preview = useContext(MediaPreviewContext);
  const [failed, setFailed] = useState(false);
  const [videoFailed, setVideoFailed] = useState(false);
  const [anchor, setAnchor] = useState<HTMLElement>();
  const close = useCallback(() => setAnchor(undefined), []);
  // Server markup has no media queries; the effect applies the reader's
  // setting right after hydration.
  const [reduced, setReduced] = useState(false);
  useEffect(() => {
    const motion = window.matchMedia("(prefers-reduced-motion: reduce)");
    const apply = () => setReduced(motion.matches);
    apply();
    motion.addEventListener("change", apply);
    return () => motion.removeEventListener("change", apply);
  }, []);
  const still = reduced || preview;
  useEffect(() => {
    const element = video.current;
    if (!element) return;
    element.muted = true;
    if (still || anchor) element.pause();
    else void element.play().catch(() => undefined);
  }, [still, gif?.mp4, anchor]);
  if (!gif) return null;
  const label = gifLabel(gif);
  const edge = gif.kind === "sticker" ? STICKER_EDGE : GIF_EDGE;
  const scale = Math.min(1, edge / Math.max(gif.width, gif.height));
  const size = { width: Math.round(gif.width * scale), aspectRatio: `${gif.width} / ${gif.height}` };
  if (failed)
    return (
      <div className="chat-message-gif chat-message-gif-failed" style={size}>
        GIF unavailable
      </div>
    );
  const image = gif.kind === "sticker" ? (gif.webp ?? gif.gif) : (gif.gif ?? gif.webp);
  const useVideo = gif.kind === "gif" && !!gif.mp4 && !videoFailed;
  const attachment: ChatAttachment = {
    id: `klipy:${gif.slug}`,
    kind: useVideo ? "video" : "image",
    contentType: useVideo ? "video/mp4" : "image/*",
    name: label,
    size: 0,
    width: gif.width,
    height: gif.height,
    url: useVideo ? gif.mp4 : image,
    previewUrl: gif.still,
    animated: true,
  };
  return (
    <>
      <button
        type="button"
        className="chat-message-gif"
        data-kind={gif.kind}
        style={size}
        title={gif.title || undefined}
        aria-label={`Open ${label} in viewer`}
        aria-haspopup="dialog"
        disabled={preview}
        onClick={(event) => setAnchor(event.currentTarget)}
      >
        {still && gif.still ? (
          <img src={gif.still} alt={label} referrerPolicy="origin" onError={() => setFailed(true)} />
        ) : useVideo ? (
          <video
            ref={video}
            src={gif.mp4}
            poster={gif.still}
            muted
            loop
            playsInline
            autoPlay={!still && !anchor}
            preload="auto"
            aria-label={label}
            onError={() => setVideoFailed(true)}
          />
        ) : (
          <GifImage src={image} alt={label} referrerPolicy="origin" decoding="async" onError={() => setFailed(true)} />
        )}
      </button>
      {anchor && (
        <MediaViewer attachments={[attachment]} attachmentId={attachment.id} anchor={anchor} onClose={close} />
      )}
    </>
  );
}
