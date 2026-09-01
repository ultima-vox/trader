import { expect, test, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const WIDTHS = [1280, 1440, 1920] as const;
const DENSITIES = ["compact", "standard", "comfortable"] as const;
const ARTIFACTS = join(dirname(fileURLToPath(import.meta.url)), "artifacts");

test.beforeAll(() => mkdirSync(ARTIFACTS, { recursive: true }));

const connection = {
  connection_id: "conn-real", provider: "T_INVEST", environment: "PRODUCTION",
  display_label: "Primary broker", enabled: true, credential_status: "VALID",
  credential_class: "FULL_ACCESS", credential_scope: "ALL_ACCESSIBLE_ACCOUNTS",
  capabilities: ["PORTFOLIO_READ", "PRODUCTION_ORDERS_PROVIDER_ALLOWED"],
  health: { state: "HEALTHY", reason_code: "NONE", retryable: false },
  created_at_unix_ms: 1, updated_at_unix_ms: 2,
};

const runtime = {
  state: "READY", reason_code: "RECONCILIATION_COMPLETE", reason: "ready",
  provider: "T_INVEST", environment: "PRODUCTION", account_display: "Capital account",
  runtime_epoch: 9, connected: true, unresolved_unknown_count: 0, open_order_count: 0,
  active_stop_count: 0, stream_states: [], persistence_healthy: true,
  execution_authorized: true, new_exposure_allowed: true,
};

async function mockPlatform(page: Page): Promise<void> {
  await page.route("**/api/v1/**", async (route) => {
    const path = new URL(route.request().url()).pathname;
    const bodies: Record<string, unknown> = {
      "/api/v1/auth/session": {
        user_id: "operator-1",
        effective_permissions: ["VIEW_CONNECTION_METADATA", "SUBMIT_PRODUCTION_MANUAL_ORDERS"],
        csrf_token: "csrf-real", expires_at_unix_ms: 99_999_999_999,
      },
      "/api/v1/broker-connections": [connection],
      "/api/v1/broker-connections/conn-real": {
        connection,
        accounts: [{
          connection_id: "conn-real", provider: "T_INVEST", environment: "PRODUCTION",
          provider_account_id: "provider-4417", display_name: "Capital account",
          account_type: "BROKER", account_status: "OPEN", access_level: "FULL_ACCESS",
          accessible: true, capabilities: ["PORTFOLIO_READ", "PRODUCTION_ORDERS_PROVIDER_ALLOWED"],
          discovered_at_unix_ms: 3,
        }],
        bindings: [{
          binding_id: "binding-1", connection_id: "conn-real", provider: "T_INVEST",
          environment: "PRODUCTION", provider_account_id: "provider-4417",
          account_id: "account-real", enabled: true, created_at_unix_ms: 4, updated_at_unix_ms: 5,
        }],
        execution_authorizations: [{
          connection_id: "conn-real", provider_account_id: "provider-4417",
          mode: "MANUAL_ALLOWED", authorization_revision: 7, changed_by: "admin",
          changed_at_unix_ms: 6,
        }],
      },
      "/api/v1/runtime": runtime,
      "/api/v1/runtime/scoped": runtime,
      "/api/v1/capabilities": {
        provider: "T_INVEST", environment: "PRODUCTION", account_id: "account-real",
        supported: ["RUNTIME_HEALTH", "ORDER_EXECUTION", "MARKET_DATA"],
        unavailable: [
          { capability: "PROTECTION_EXECUTION", reason: "not projected", owner: "#23" },
          { capability: "RISK_VERDICT", reason: "not projected", owner: "#24" },
        ],
      },
      "/api/v1/market/instruments": [{
        identity: { provider: "T_INVEST", uid: "uid-sber", ticker: "SBER", class_code: "TQBR" },
        name: "Сбербанк",
        instrument_type: "Акция",
        lot_size: 10, min_price_increment: "0.01", currency: "RUB", tradable: true,
      }],
    };
    const body = bodies[path];
    if (body === undefined) {
      await route.fulfill({ status: 404, json: { code: "NOT_FOUND", message: path, correlation_id: "e2e", category: "NOT_FOUND", retryable: false } });
      return;
    }
    await route.fulfill({ status: 200, json: body });
  });
}

async function openPlatform(page: Page): Promise<void> {
  await mockPlatform(page);
  await page.goto("/");
  await page.getByLabel("Bootstrap credential").fill("browser-only-secret");
  await page.getByRole("button", { name: "Открыть сессию" }).click();
  await expect(page.locator(".vox-shell")).toBeVisible();
  await expect(page.locator("[data-widget-id='order-ticket']")).toBeVisible();
}

for (const width of WIDTHS) {
  test(`real platform shell at ${width}px stays exact across densities`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await openPlatform(page);
    await expect(page.getByText("Capital account", { exact: true }).first()).toBeVisible();
    await expect(page.getByText("Primary broker", { exact: true }).first()).toBeVisible();
    await expect(page.locator(".vox-ticket__action--buy")).toBeVisible();
    await expect(page.locator(".vox-ticket__action--sell")).toBeVisible();

    for (const density of DENSITIES) {
      await page.locator(`[data-density-choice='${density}']`).click();
      await expect(page.locator("#app")).toHaveAttribute("data-density", density);
      const geometry = await page.evaluate(() => {
        const target = document.querySelector(".vox-ticket__target")?.getBoundingClientRect();
        return {
          overflow: document.documentElement.scrollWidth - document.documentElement.clientWidth,
          targetLeft: target?.left ?? -1,
          targetRight: target?.right ?? Number.MAX_SAFE_INTEGER,
          viewport: document.documentElement.clientWidth,
        };
      });
      expect(geometry.overflow).toBeLessThanOrEqual(1);
      expect(geometry.targetLeft).toBeGreaterThanOrEqual(0);
      expect(geometry.targetRight).toBeLessThanOrEqual(geometry.viewport);
    }

    const persisted = await page.evaluate(() => `${JSON.stringify(localStorage)}${JSON.stringify(sessionStorage)}`);
    expect(persisted).not.toContain("browser-only-secret");
    await page.screenshot({ path: join(ARTIFACTS, `platform-${width}.png`), fullPage: true });
  });
}

test("401 leaves coherent session screen and no fake platform", async ({ page }) => {
  await page.route("**/api/v1/auth/session", (route) => route.fulfill({
    status: 401,
    json: { code: "UNAUTHENTICATED", message: "invalid credential", correlation_id: "e2e", category: "AUTHENTICATION", retryable: false },
  }));
  await page.goto("/");
  await page.getByLabel("Bootstrap credential").fill("wrong-secret");
  await page.getByRole("button", { name: "Открыть сессию" }).click();
  await expect(page.getByRole("status")).toContainText("401: invalid credential");
  await expect(page.locator(".vox-shell")).toHaveCount(0);
  await expect(page.getByLabel("Bootstrap credential")).toHaveValue("");
});

test("403 capability denial keeps authenticated shell coherent and actions absent", async ({ page }) => {
  await mockPlatform(page);
  await page.route("**/api/v1/capabilities?**", (route) => route.fulfill({
    status: 403,
    json: { code: "FORBIDDEN", message: "permission denied", correlation_id: "e2e", category: "PERMISSION", retryable: false },
  }));
  await page.goto("/");
  await page.getByLabel("Bootstrap credential").fill("browser-only-secret");
  await page.getByRole("button", { name: "Открыть сессию" }).click();
  await expect(page.locator(".vox-shell")).toBeVisible();
  await expect(page.getByText("403: permission denied")).toBeVisible();
  await expect(page.locator(".vox-ticket__action")).toHaveCount(0);
});
