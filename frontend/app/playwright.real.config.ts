import { defineConfig, devices } from "@playwright/test";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.dirname(fileURLToPath(import.meta.url));
const fixture = path.resolve(
  root,
  process.platform === "win32"
    ? "../../target/debug/frontend-e2e-server.exe"
    : "../../target/debug/frontend-e2e-server",
);

export default defineConfig({
  testDir: "e2e",
  testMatch: "real-platform.spec.ts",
  fullyParallel: false,
  forbidOnly: Boolean(process.env.CI),
  retries: 0,
  reporter: [["list"]],
  outputDir: "e2e/test-results-real",
  use: {
    baseURL: "http://127.0.0.1:4174",
    browserName: "chromium",
    ...devices["Desktop Chrome"],
    viewport: { width: 1440, height: 900 },
  },
  webServer: [
    {
      command: JSON.stringify(fixture),
      url: "http://127.0.0.1:18100/api/v1/system/health",
      reuseExistingServer: false,
      timeout: 30_000,
      env: {
        VOX_FRONTEND_E2E_BOOTSTRAP: "frontend-e2e-bootstrap-credential-material-0001",
      },
    },
    {
      command: "npx vite --port 4174 --strictPort --host 127.0.0.1",
      url: "http://127.0.0.1:4174",
      reuseExistingServer: false,
      timeout: 60_000,
      env: { VOX_API_PROXY: "http://127.0.0.1:18100" },
    },
  ],
});
