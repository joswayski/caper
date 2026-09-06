import { tanstackStart } from "@tanstack/react-start/plugin/vite";
import react from "@vitejs/plugin-react";
import { nitro } from "nitro/vite";
import { defineConfig, searchForWorkspaceRoot } from "vite";

export default defineConfig({
  plugins: [
    tanstackStart({
      prerender: { enabled: false },
      server: { build: { inlineCss: true } },
    }),
    react(),
    nitro({
      routeRules: {
        "/assets/**": {
          headers: { "cache-control": "public, max-age=31536000, immutable" },
        },
        "/brand/**": {
          headers: { "cache-control": "public, max-age=86400" },
        },
      },
    }),
  ],
  server: {
    port: 5174,
    allowedHosts: [".onamp.dev"],
    fs: { allow: [searchForWorkspaceRoot(import.meta.dirname)] },
  },
});
