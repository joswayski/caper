import { createCsrfMiddleware, createMiddleware, createStart } from "@tanstack/react-start";
import { getHealth } from "./server/api";

// Container probes must work before authentication secrets are configured.
const healthProbe = createMiddleware().server(async ({ request, next }) => {
  if (request.method === "GET" && new URL(request.url).pathname === "/api/health") {
    return getHealth(request);
  }
  return next();
});

const privateResponses = createMiddleware().server(async ({ next }) => {
  const result = await next();
  result.response.headers.set("cache-control", "private, no-store");
  return result;
});

export const startInstance = createStart(() => ({
  requestMiddleware: [
    healthProbe,
    privateResponses,
    createCsrfMiddleware({ filter: (ctx) => ctx.handlerType === "serverFn" }),
  ],
}));
