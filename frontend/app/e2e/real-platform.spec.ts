import { expect, test } from "@playwright/test";

test("real browser and Vox boundary keep selected runtime scope atomic", async ({ page, context }) => {
  const scopedAccounts: string[] = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname === "/api/v1/runtime/scoped") {
      scopedAccounts.push(url.searchParams.get("account_id") ?? "missing");
    }
  });

  await page.goto("/");
  const anonymousStatus = await page.evaluate(async () =>
    (await fetch("/api/v1/broker-connections")).status
  );
  expect(anonymousStatus).toBe(401);

  await page.getByLabel("Bootstrap credential").fill(
    "frontend-e2e-bootstrap-credential-material-0001",
  );
  await page.getByRole("button", { name: "Открыть сессию" }).click();
  await expect(page.locator(".vox-shell")).toBeVisible();
  await expect(page.locator(".vox-account__label")).toHaveText("Alpha account");
  await expect(page.locator(".vox-runtime__label")).toHaveText("READY");
  await expect(page.locator(".vox-ticket__action--buy")).toBeEnabled();
  await expect(page.getByText("Сбербанк", { exact: false }).first()).toBeVisible();
  await expect(page.locator("body")).not.toContainText("provider-diagnostic-uid");

  const cookies = await context.cookies();
  expect(cookies.find((cookie) => cookie.name === "vox_session")?.httpOnly).toBe(true);
  const missingCsrfStatus = await page.evaluate(async () =>
    (await fetch("/api/v1/broker-connections", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: "{}",
    })).status
  );
  expect(missingCsrfStatus).toBe(403);

  await page.locator(".vox-account").click();
  await page.getByRole("option").filter({ hasText: "Beta account" }).click();
  await expect(page.locator(".vox-account__label")).toHaveText("Beta account");
  await expect(page.locator(".vox-runtime__label")).toHaveText("HALTED");
  await expect(page.locator(".vox-ticket__action--buy")).toBeDisabled();
  await expect(page.locator(".vox-ticket__hint")).toContainText("runtime execution authorization false");
  expect(scopedAccounts).toEqual(["account:alpha", "account:beta"]);
});
