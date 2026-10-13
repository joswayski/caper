import { useRef, type ImgHTMLAttributes } from "react";

/** Keep the original GIF, not its still upload preview. The canvas gives
 * reduced-motion readers a still even when the upload has no preview. */
export default function GifImage({ alt, onLoad, ...props }: ImgHTMLAttributes<HTMLImageElement>) {
  const canvas = useRef<HTMLCanvasElement>(null);
  return (
    <span className="chat-gif" role="img" aria-label={alt}>
      <img
        {...props}
        alt=""
        onLoad={(event) => {
          const image = event.currentTarget;
          const still = canvas.current;
          if (still) {
            const scale = Math.min(1, 720 / Math.max(image.naturalWidth, image.naturalHeight));
            still.width = Math.max(1, Math.round(image.naturalWidth * scale));
            still.height = Math.max(1, Math.round(image.naturalHeight * scale));
            still.getContext("2d")?.drawImage(image, 0, 0, still.width, still.height);
          }
          onLoad?.(event);
        }}
      />
      <canvas ref={canvas} aria-hidden="true" />
    </span>
  );
}
