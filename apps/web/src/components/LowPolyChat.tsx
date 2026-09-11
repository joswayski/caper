import { useEffect, useRef } from "react";

export default function LowPolyChat() {
  const hostRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    let cancelled = false;
    let unmount = () => {};
    // Canvas text is rasterized when the Three.js model is built. Wait for the
    // same web font as the poster so the handoff cannot swap typefaces.
    const regularPreviewFont = document.fonts.load('400 48px "Satoshi"');
    const boldPreviewFont = document.fonts.load('700 48px "Satoshi"');
    void import("./mountChatPreview").then(async ({ mountChatPreview }) => {
      // A blocked font CDN should still reveal the model using its fallback stack.
      await Promise.allSettled([regularPreviewFont, boldPreviewFont]);
      if (cancelled) return;
      unmount = mountChatPreview(host, () => {
        if (!cancelled) host.classList.add("is-live");
      });
      if (cancelled) unmount();
    });

    return () => {
      cancelled = true;
      host.classList.remove("is-live");
      unmount();
    };
  }, []);

  return (
    <div
      className="low-poly-chat"
      ref={hostRef}
      tabIndex={0}
      role="group"
      aria-label="Interactive 3D Caper chat preview. Drag to rotate; on touch screens, drag with two fingers."
    >
      <picture aria-hidden="true">
        <source media="(max-width: 1000px)" srcSet="/images/chat-preview-stacked.webp?v=20260906" type="image/webp" />
        <img
          className="low-poly-chat-poster"
          src="/images/chat-preview-wide.webp?v=20260906"
          alt=""
          width={1720}
          height={1211}
          fetchPriority="high"
        />
      </picture>
    </div>
  );
}
