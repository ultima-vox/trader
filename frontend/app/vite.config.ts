import path from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

const root = path.dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  resolve: {
    alias: {
      "@vox/api-client": path.resolve(root, "../api-client/src/index.ts"),
    },
  },
  server: {
    fs: {
      allow: [path.resolve(root, "..")],
    },
  },
});
