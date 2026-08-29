import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

import { assertNoProviderCalls, assertSafeBaseUrl, FORBIDDEN_PROVIDER } from "./provider";

function collectProductionSource(dir: string): string {
  const parts: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      parts.push(collectProductionSource(full));
      continue;
    }
    if (!/\.(?:ts|tsx)$/i.test(entry.name)) continue;
    if (/\.(?:test|spec)\./i.test(entry.name)) continue;
    parts.push(readFileSync(full, "utf8"));
  }
  return parts.join("\n");
}

function stripDenylistLiterals(source: string): string {
  let next = source;
  for (const needle of FORBIDDEN_PROVIDER) {
    next = next.replaceAll(`"${needle}"`, '""');
  }
  return next;
}

describe("assertNoProviderCalls", () => {
  it("fails when source talks to a T-Invest host", () => {
    expect(() =>
      assertNoProviderCalls("fetch('https://invest-public-api.tinkoff.ru')"),
    ).toThrow(/provider host or package: invest-public-api/);
  });

  it("allows VoxClient talking to /api/v1", () => {
    const source = `
      const client = new VoxClient({ baseUrl: "" });
      await client.runtime(); // GET /api/v1/runtime
    `;
    expect(() => assertNoProviderCalls(source)).not.toThrow();
  });

  it("fails on sandbox host and tinkoff protobuf packages", () => {
    expect(() =>
      assertNoProviderCalls("https://sandbox-invest-public-api.tbank.ru"),
    ).toThrow();
    expect(() =>
      assertNoProviderCalls("tinkoff.public.invest.api.contract.v1.OrdersService"),
    ).toThrow();
    expect(() =>
      assertNoProviderCalls("fetch('https://api-invest.tinkoff.ru/openapi')"),
    ).toThrow();
  });

  it("production src has no provider host fetch", () => {
    const srcRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
    const scanned = stripDenylistLiterals(collectProductionSource(srcRoot));
    expect(() => assertNoProviderCalls(scanned)).not.toThrow();
    for (const needle of FORBIDDEN_PROVIDER) {
      expect(scanned.toLowerCase()).not.toContain(needle.toLowerCase());
    }
  });

  it("rejects a provider baseUrl", () => {
    expect(() => assertSafeBaseUrl("https://invest-public-api.tinkoff.ru")).toThrow();
    expect(() => assertSafeBaseUrl("https://api.tbank.ru")).toThrow();
    expect(() => assertSafeBaseUrl("//api-invest.tinkoff.ru")).toThrow();
    expect(() => assertSafeBaseUrl("https://api.tbank.ru.")).toThrow();
    expect(() => assertSafeBaseUrl("")).not.toThrow();
    expect(() => assertSafeBaseUrl("/api/v1")).not.toThrow();
  });
});
