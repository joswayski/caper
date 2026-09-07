// Browser profile transport. Rust is
// the authority for JWT verification, account ownership, and profile validation.
export async function proxyAccount(request: Request, accountToken: string): Promise<Response> {
  const headers = { "cache-control": "no-store" };
  if (!accountToken) return new Response(null, { status: 401, headers });
  if (request.method !== "POST") return new Response(null, { status: 405, headers });
  let body: Uint8Array | undefined;
  if (request.method === "POST") {
    if (!request.headers.get("content-type")?.startsWith("application/json")) return new Response(null, { status: 415, headers });
    const reader = request.body?.getReader();
    const chunks: Uint8Array[] = [];
    let length = 0;
    if (reader) for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      length += value.length;
      if (length > 4096) { await reader.cancel(); return new Response(null, { status: 413, headers }); }
      chunks.push(value);
    }
    body = Buffer.concat(chunks);
  }
  const base = process.env.MEDIA_API_URL;
  if (!base) return new Response(null, { status: 503, headers });
  try {
    const response = await fetch(`${base.replace(/\/$/, "")}/api/account/profile`, {
      method: request.method,
      headers: { "content-type": "application/json", authorization: `Bearer ${accountToken}` },
      body: body as BodyInit | undefined,
      signal: AbortSignal.any([request.signal, AbortSignal.timeout(10_000)]),
      redirect: "error",
    });
    return new Response(response.body, { status: response.status, headers: { ...headers, "content-type": "application/json" } });
  } catch { return new Response(null, { status: 503, headers }); }
}
