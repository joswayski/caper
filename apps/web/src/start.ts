import { createCsrfMiddleware, createMiddleware, createStart } from "@tanstack/react-start";
import { getWebHealth } from "./server/health";

// Container probes must work before authentication secrets are configured.
const healthProbe = createMiddleware().server(async ({ request, next }) => {
  if (request.method === "GET" && new URL(request.url).pathname === "/health") {
    return getWebHealth(request);
  }
  return next();
});

const privateResponses = createMiddleware().server(async ({ next }) => {
  const result = await next();
  const headers = result.response.headers;
  headers.set("cache-control", "private, no-store");
  // Browsers only honour HSTS over HTTPS, so local development is unaffected.
  headers.set("strict-transport-security", "max-age=31536000");
  // Nothing embeds Caper; refuse framing (clickjacking) and MIME sniffing.
  headers.set("content-security-policy", "frame-ancestors 'none'; base-uri 'none'; object-src 'none'");
  headers.set("x-frame-options", "DENY");
  headers.set("x-content-type-options", "nosniff");
  headers.set("referrer-policy", "strict-origin-when-cross-origin");
  return result;
});

export const startInstance = createStart(() => ({
  requestMiddleware: [
    healthProbe,
    privateResponses,
    createCsrfMiddleware({ filter: (ctx) => ctx.handlerType === "serverFn" }),
  ],
}));
