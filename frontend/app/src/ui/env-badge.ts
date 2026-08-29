import type { BrokerEnvironment } from "@vox/api-client";
import { append, el } from "./dom";

export function createEnvBadge(environment: BrokerEnvironment): HTMLElement {
  const badge = el("span", "vox-env");
  badge.classList.add(environment === "PRODUCTION" ? "vox-env--live" : "vox-env--sandbox");
  append(badge, el("span", "vox-dot"), document.createTextNode(environment));
  return badge;
}

export function providerLabel(provider: string): string {
  return provider === "T_INVEST" ? "Т-Инвест" : provider;
}
