import { defineConfig } from "vitest/config";

// Keep unit tests independent of the app's build plugins and GitHub history fetch.
export default defineConfig({
  define: { __SOURCE_REVISION__: JSON.stringify("") },
  test: {
    name: "web",
    root: import.meta.dirname,
    environment: "node",
    include: ["src/**/*.test.ts", "src/**/*.test.tsx"],
    setupFiles: ["./vitest.setup.ts"],
    restoreMocks: true,
    unstubGlobals: true,
  },
});
