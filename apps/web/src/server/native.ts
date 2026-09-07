import { proxyAccount } from "./account-proxy.ts";
import { proxyMedia } from "./media.ts";

export async function proxyNative(request: Request): Promise<Response> {
  // Deliberately cookie-free: a web session cookie cannot authenticate here.
  const token = request.headers.get("x-caper-account-token") ?? "";
  if (!token) return new Response(null, { status: 401, headers: { "cache-control": "no-store" } });
  const url = new URL(request.url);
  const operation = url.pathname.slice("/api/native/".length);
  if (operation === "account/me" || operation === "account/profile") {
    return proxyAccount(request, token, operation === "account/me" ? "me" : "profile");
  }
  if (operation.startsWith("media/")) {
    url.pathname = `/api/${operation}`;
    return proxyMedia(new Request(url, request), token);
  }
  return new Response(null, { status: 404, headers: { "cache-control": "no-store" } });
}
