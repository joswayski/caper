import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    projects: [
      "apps/web/vitest.config.ts",
      {
        test: {
          name: "native",
          environment: "node",
          include: ["tests/**/*.test.mjs"],
        },
      },
      {
        test: {
          name: "cdn",
          environment: "node",
          include: ["apps/cdn/**/*.test.mjs"],
        },
      },
    ],
  },
});
