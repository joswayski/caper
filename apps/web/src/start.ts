import { createCsrfMiddleware, createMiddleware, createStart } from "@tanstack/react-start";
import { authkitMiddleware } from "@workos/authkit-tanstack-react-start";

const privateResponses = createMiddleware().server(async ({ next }) => {
  const result = await next();
  result.response.headers.set("cache-control", "private, no-store");
  return result;
});

export const startInstance = createStart(() => ({
  requestMiddleware: [
    privateResponses,
    createCsrfMiddleware({ filter: (ctx) => ctx.handlerType === "serverFn" }),
    authkitMiddleware(),
  ],
}));
