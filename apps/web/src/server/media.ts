const operations = new Set(["status", "join", "snapshot", "events", "publish", "subscribe", "negotiate", "close", "state", "leave"]);

// Development/orb adapter. Production ingress routes directly to the same single Rust service.
export async function proxyMedia(request: Request): Promise<Response> {
  const operation = new URL(request.url).pathname.slice("/api/media/".length);
  const headers = { "cache-control": "no-store" };
  if (!operations.has(operation)) return new Response(null, { status: 404, headers });
  const streaming = operation === "events";
  if (request.method !== (operation === "status" || streaming ? "GET" : "POST")) {
    return new Response(null, { status: 405, headers });
  }
  if (request.headers.get("sec-fetch-site") === "cross-site") {
    return new Response(null, { status: 403, headers });
  }
  const base = process.env.MEDIA_API_URL;
  if (!base) return operation === "status"
    ? Response.json({ enabled: false }, { headers })
    : Response.json({ error: "Calls are currently unavailable." }, { status: 503, headers });
  let body: Uint8Array | undefined;
  if (request.method === "POST") {
    if (!request.headers.get("content-type")?.startsWith("application/json")) {
      return new Response(null, { status: 415, headers });
    }
    const reader = request.body?.getReader();
    const chunks: Uint8Array[] = [];
    let size = 0;
    if (reader) {
      while (true) {
        const { value, done } = await reader.read();
        if (done) break;
        size += value.byteLength;
        if (size > 256 * 1024) { await reader.cancel(); return new Response(null, { status: 413, headers }); }
        chunks.push(value);
      }
    }
    body = new Uint8Array(size);
    let offset = 0;
    for (const chunk of chunks) { body.set(chunk, offset); offset += chunk.length; }
  }
  const controller = new AbortController();
  // Limit time to headers, not the lifetime of a healthy SSE response.
  const timer = streaming ? setTimeout(() => controller.abort(), 25_000) : undefined;
  try {
    const upstream = await fetch(`${base.replace(/\/$/, "")}/api/media/${operation}`, {
      method: request.method,
      headers: { "content-type": "application/json", authorization: request.headers.get("authorization") ?? "" },
      body: body as BodyInit | undefined,
      signal: AbortSignal.any([request.signal, streaming ? controller.signal : AbortSignal.timeout(25_000)]),
      redirect: "error",
    });
    const errorId = upstream.headers.get("x-caper-error-id");
    return new Response(upstream.body, { status: upstream.status, headers: {
      ...(errorId ? { "x-caper-error-id": errorId } : {}),
      ...headers,
      "content-type": upstream.headers.get("content-type") ?? "application/json",
      ...(streaming ? { "x-accel-buffering": "no" } : {}),
    } });
  } catch {
    return Response.json({ error: "The call service is temporarily unavailable." }, { status: 503, headers });
  } finally {
    clearTimeout(timer);
  }
}
