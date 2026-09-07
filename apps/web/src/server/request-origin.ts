// Keep state-changing endpoints same-origin while accounts are unavailable.
export function sameOrigin(request: Request): boolean {
  if (request.headers.get("sec-fetch-site") === "cross-site") return false;
  return request.headers.get("origin") === new URL(request.url).origin;
}
