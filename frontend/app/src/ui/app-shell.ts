import type { AccountStore } from "../account";
import type { BrokerEnvironment, RuntimeHealthDto } from "@vox/api-client";
import type { PlatformAccount } from "../platform";
import { createAccountContextIndicator } from "./account-context-indicator";
import { createAccountSelector } from "./account-selector";
import { append, el } from "./dom";
import { createEnvBadge } from "./env-badge";
import { createRuntimeStatus } from "./runtime-status";

export type AppShellOptions = {
  environment: BrokerEnvironment;
  accountStore: AccountStore;
  runtime: RuntimeHealthDto;
  accounts?: readonly PlatformAccount[];
  body?: HTMLElement;
};

const NAV_PRIMARY: ReadonlyArray<{ id: string; label: string }> = [
  { id: "markets", label: "Рынки" },
  { id: "trade", label: "Торговля" },
  { id: "portfolio", label: "Портфель" },
  { id: "strategy", label: "Стратегии" },
  { id: "decision", label: "Решения" },
  { id: "research", label: "Исследования" },
  { id: "ml", label: "ML / Модели" },
  { id: "system", label: "Система" },
];

const NAV_SECONDARY: ReadonlyArray<{ id: string; label: string }> = [
  { id: "settings", label: "Настройки" },
];

export type AppShellHandle = {
  element: HTMLElement;
  body: HTMLElement;
};

export function createAppShell(options: AppShellOptions): AppShellHandle {
  const shell = el("div", "vox-shell");
  const topbar = el("div", "vox-topbar");

  const brand = el("div", "vox-topbar__group vox-topbar__brand", "VOX TRADER");
  const envGroup = el("div", "vox-topbar__group");
  append(envGroup, createEnvBadge(options.environment));

  const accountGroup = el("div", "vox-topbar__group");
  append(
    accountGroup,
    createAccountSelector({
      store: options.accountStore,
      ...(options.accounts === undefined ? {} : { accounts: options.accounts }),
    }),
    createAccountContextIndicator({ store: options.accountStore }),
  );

  const runtimeGroup = el("div", "vox-topbar__group");
  append(runtimeGroup, createRuntimeStatus({ health: options.runtime }));

  const clockGroup = el("div", "vox-topbar__group");
  const clock = el("span", "vox-topbar__clock", formatMskClock());
  clock.setAttribute("aria-label", "время MSK");
  append(clockGroup, clock);
  window.setInterval(() => {
    clock.textContent = formatMskClock();
  }, 1000);

  append(topbar, brand, envGroup, accountGroup, runtimeGroup, el("div", "vox-grow"), clockGroup);

  const shellBody = el("div", "vox-shell__body");
  const nav = el("nav", "vox-nav");
  nav.setAttribute("aria-label", "Навигация");
  for (const item of NAV_PRIMARY) {
    append(nav, navItem(item.id, item.label));
  }
  const secondary = el("div", "vox-nav__secondary");
  for (const item of NAV_SECONDARY) {
    append(secondary, navItem(item.id, item.label));
  }
  append(nav, secondary);

  const content = el("div", "vox-grow");
  if (options.body !== undefined) append(content, options.body);
  append(shellBody, nav, content);
  append(shell, topbar, shellBody);
  return { element: shell, body: content };
}

function navItem(id: string, label: string): HTMLElement {
  const item = el("span", "vox-nav__item", label);
  item.dataset.nav = id;
  item.title = "#30";
  append(item, el("span", "vox-dep", "#30"));
  return item;
}

function formatMskClock(now = new Date()): string {
  const formatted = new Intl.DateTimeFormat("ru-RU", {
    timeZone: "Europe/Moscow",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(now);
  return `${formatted} MSK`;
}
