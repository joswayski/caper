import type { MessageNotification } from "./browser-notifications";

/** Render one bounded original in memory. No upload or persistent thumbnail. */
export async function notificationImage(event: MessageNotification, signal: AbortSignal): Promise<string | undefined> {
  const { attachmentUrl, attachmentContentType, attachmentSize } = event;
  if (
    !attachmentUrl ||
    !attachmentContentType ||
    !Number.isSafeInteger(attachmentSize) ||
    !attachmentSize ||
    attachmentSize < 1 ||
    attachmentSize > 10_000_000 ||
    ![
      "image/png",
      "image/jpeg",
      "image/gif",
      "image/webp",
      "image/avif",
      "video/mp4",
      "video/webm",
      "video/quicktime",
    ].includes(attachmentContentType)
  )
    return;
  let original: string | undefined;
  let media: HTMLImageElement | HTMLVideoElement | undefined;
  const controller = new AbortController();
  const cancel = () => controller.abort();
  signal.addEventListener("abort", cancel, { once: true });
  if (signal.aborted) cancel();
  const timer = setTimeout(cancel, 5_000);
  const timeout = controller.signal;
  try {
    const url = new URL(attachmentUrl);
    if (
      url.protocol !== "https:" ||
      url.username ||
      url.password ||
      url.hash ||
      !/^\/original\/[A-Za-z0-9]{1,64}$/.test(url.pathname)
    )
      return;
    const response = await fetch(url, { credentials: "omit", redirect: "error", cache: "no-store", signal: timeout });
    if (response.status !== 200 || !response.body || Number(response.headers.get("content-length")) > attachmentSize)
      return;
    const reader = response.body.getReader();
    const chunks: BlobPart[] = [];
    let total = 0;
    try {
      while (true) {
        const { value, done } = await reader.read();
        if (done) break;
        total += value.length;
        if (total > attachmentSize) return;
        chunks.push(new Uint8Array(value));
      }
    } finally {
      await reader.cancel();
    }
    if (total !== attachmentSize || timeout.aborted) return;
    original = URL.createObjectURL(new Blob(chunks, { type: attachmentContentType }));
    media = attachmentContentType.startsWith("video/") ? document.createElement("video") : new Image();
    const source = media;
    await new Promise<void>((resolve, reject) => {
      const ready = source instanceof HTMLVideoElement ? "loadeddata" : "load";
      const finish = (error?: Event) => {
        source.removeEventListener(ready, loaded);
        source.removeEventListener("error", failed);
        timeout.removeEventListener("abort", failed);
        if (error) reject(new Error("Notification image unavailable"));
        else resolve();
      };
      const loaded = () => finish();
      const failed = (error: Event) => finish(error);
      source.addEventListener(ready, loaded, { once: true });
      source.addEventListener("error", failed, { once: true });
      timeout.addEventListener("abort", failed, { once: true });
      if (timeout.aborted) {
        failed(new Event("abort"));
        return;
      }
      source.src = original!;
    });
    const width = source instanceof HTMLVideoElement ? source.videoWidth : source.naturalWidth;
    const height = source instanceof HTMLVideoElement ? source.videoHeight : source.naturalHeight;
    if (!width || !height || timeout.aborted) return;
    const scale = Math.min(1, 1024 / Math.max(width, height));
    const canvas = document.createElement("canvas");
    canvas.width = Math.max(1, Math.round(width * scale));
    canvas.height = Math.max(1, Math.round(height * scale));
    const context = canvas.getContext("2d");
    if (!context) return;
    context.drawImage(source, 0, 0, canvas.width, canvas.height);
    const png = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/png"));
    if (png && !timeout.aborted) return URL.createObjectURL(png);
  } catch {
    /* Text remains usable when decoding, fetching or CORS fails. */
  } finally {
    clearTimeout(timer);
    signal.removeEventListener("abort", cancel);
    media?.removeAttribute("src");
    if (media && media instanceof HTMLVideoElement) media.load();
    if (original) URL.revokeObjectURL(original);
  }
}
