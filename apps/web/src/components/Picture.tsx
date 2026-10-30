import { useLayoutEffect, useRef, useState, useSyncExternalStore } from "react";
import { pictureFailed, pictureLoaded, pictureShown, pictureSource, squareCrop } from "../account/pictures";
import "./picture.css";

const REDUCED_MOTION = "(prefers-reduced-motion: reduce)";
/** Longest side of a reduced-motion still: sharp at the largest avatar on a 2x screen. */
const STILL_EDGE = 256;

function subscribeMotion(listener: () => void) {
  if (typeof matchMedia === "undefined") return () => {};
  const query = matchMedia(REDUCED_MOTION);
  query.addEventListener("change", listener);
  return () => query.removeEventListener("change", listener);
}
const reducedMotion = () => typeof matchMedia !== "undefined" && matchMedia(REDUCED_MOTION).matches;

/**
 * A custom picture (profile picture or space icon) laid over the mascot or
 * initial its parent already shows: invisible until it loads, removed if it
 * fails, so there is never a broken image. Fills the parent's shape (the
 * parent must be positioned). GIFs animate; with reduced motion a still frame
 * is shown instead.
 */
export default function Picture({ asset, url, localPreview }: { asset?: string; url?: string; localPreview?: string }) {
  const [, setRevision] = useState(0);
  // Only editor-owned object URLs bypass the remote-picture URL policy. Use
  // the same renderer so animated drafts honor reduced motion too.
  const preview = localPreview?.startsWith("blob:") ? localPreview : undefined;
  const source = preview ?? pictureSource(asset, url);
  asset = preview ?? asset;
  if (!asset || !source) return null;
  // A new URL (a re-signed or replaced picture) starts a fresh load.
  return <PictureImage key={source} asset={asset} source={source} onFailed={() => setRevision((value) => value + 1)} />;
}

function drawStill(image: HTMLImageElement, canvas: HTMLCanvasElement) {
  const crop = squareCrop(image.naturalWidth, image.naturalHeight);
  const edge = Math.min(STILL_EDGE, crop.side);
  canvas.width = edge;
  canvas.height = edge;
  // A cross-origin picture taints the canvas, which still displays it.
  canvas.getContext("2d")?.drawImage(image, crop.x, crop.y, crop.side, crop.side, 0, 0, edge, edge);
}

function PictureImage({ asset, source, onFailed }: { asset: string; source: string; onFailed: () => void }) {
  // `ready` appears at once (already shown on this page); `arrived` fades in.
  const [state, setState] = useState<"loading" | "ready" | "arrived" | "failed">(() =>
    pictureShown(asset, source) ? "ready" : "loading",
  );
  const still = useSyncExternalStore(subscribeMotion, reducedMotion, () => false);
  // The still replaces the image only once it is drawn: a picture already shown
  // elsewhere starts `ready` while this element's own copy may still be decoding.
  const [drawn, setDrawn] = useState(false);
  const image = useRef<HTMLImageElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const loaded = state === "ready" || state === "arrived";
  const draw = () => {
    if (!image.current?.naturalWidth || !canvas.current) return;
    drawStill(image.current, canvas.current);
    setDrawn(true);
  };
  useLayoutEffect(() => {
    if (still && loaded) draw();
    else setDrawn(false);
  }, [still, loaded]);
  const settle = (node: HTMLImageElement) => {
    if (node.naturalWidth > 0) {
      pictureLoaded(asset, source);
      setState((current) => (current === "loading" ? "arrived" : current));
      if (still) draw();
    } else {
      pictureFailed(asset, source);
      setState("failed");
      onFailed();
    }
  };
  if (state === "failed") return null;
  return (
    <span className="caper-picture" data-state={state} data-still={still && drawn ? "" : undefined}>
      <img
        ref={(node) => {
          image.current = node;
          // Server-rendered markup can finish (or fail) loading before hydration attaches the handlers below.
          if (node?.complete && node.currentSrc && state === "loading") settle(node);
        }}
        src={source}
        alt=""
        decoding="async"
        loading="lazy"
        draggable={false}
        onLoad={(event) => settle(event.currentTarget)}
        onError={(event) => settle(event.currentTarget)}
      />
      {still && <canvas ref={canvas} aria-hidden="true" />}
    </span>
  );
}
