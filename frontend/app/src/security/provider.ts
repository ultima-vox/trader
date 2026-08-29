export const FORBIDDEN_PROVIDER = [
  "sandbox-invest-public-api",
  "api-invest.tinkoff.ru",
  "invest-public-api",
  "tinkoff.public.invest",
] as const;

export function assertNoProviderCalls(source: string): void {
  const lower = source.toLowerCase();
  for (const needle of FORBIDDEN_PROVIDER) {
    if (lower.includes(needle.toLowerCase())) {
      throw new Error(`browser source must not call provider host or package: ${needle}`);
    }
  }
}

export function assertSafeBaseUrl(baseUrl: string): void {
  const trimmed = baseUrl.trim();
  if (trimmed === "") return;
  if (/^\/[^/]/.test(trimmed) || trimmed === "/") return;
  if (trimmed.startsWith("//")) {
    throw new Error("browser must not use protocol-relative provider URL");
  }
  let parsed: URL;
  try {
    parsed = new URL(trimmed);
  } catch {
    throw new Error("invalid Vox baseUrl");
  }
  const host = parsed.hostname.toLowerCase().replace(/\.+$/, "");
  if (
    host === "tinkoff.ru" ||
    host.endsWith(".tinkoff.ru") ||
    host === "tbank.ru" ||
    host.endsWith(".tbank.ru")
  ) {
    throw new Error(`browser must not use provider host: ${host}`);
  }
  assertNoProviderCalls(trimmed);
}
