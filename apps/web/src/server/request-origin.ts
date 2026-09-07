// Browsers must prove the origin for cookie-authenticated POSTs. Native clients
// authenticate directly against Rust using account tokens, not these routes.
export function sameOrigin(request: Request): boolean {
  const expected = process.env.WORKOS_REDIRECT_URI;
  if (!expected || request.headers.get("sec-fetch-site") === "cross-site") return false;
  return request.headers.get("origin") === new URL(expected).origin;
}
