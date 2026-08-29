import path from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

const root = path.dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  resolve: {
    alias: {
      "@vox/api-client": path.resolve(root, "../api-client/src/index.ts"),
    },
  },
  test: {
    environment: "happy-dom",
    passWithNoTests: true,
    exclude: ["e2e/**", "node_modules/**"],
  },
});
