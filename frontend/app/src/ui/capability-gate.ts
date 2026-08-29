import type {
  Capability,
  CapabilitySet,
  RuntimeHealthDto,
  UnavailableCapability,
} from "@vox/api-client";
import { append } from "./dom";
import { createDeferred } from "./deferred";

export type CapabilityGateOptions = {
  capabilities: CapabilitySet;
  capability: Capability;
  children?: HTMLElement[];
  runtime?: RuntimeHealthDto;
};

const CAPABILITY_TITLE: Record<Capability, string> = {
  RUNTIME_HEALTH: "Состояние рантайма",
  ACCOUNT_READ_SIDE: "Счета и позиции",
  ORDER_EXECUTION: "Исполнение заявок",
  PROTECTION_EXECUTION: "Защита позиций",
  PROTECTION_DEFAULTS: "Защита по умолчанию",
  BULK_PROTECTION_MIGRATION: "Массовое обновление защиты",
  CONNECTION_MANAGEMENT: "Подключения брокера",
  RBAC: "Права доступа",
  RISK_VERDICT: "Вердикт риска",
  PORTFOLIO_VALUATION: "Оценка портфеля",
  MARKET_DATA: "Рыночные данные",
  STRATEGY: "Стратегии",
  DECISION: "Решения",
  MACHINE_LEARNING: "Модели",
  RESEARCH: "Исследования",
  AGGREGATE_ACCOUNTS: "Все счета",
  MULTI_PROVIDER: "Несколько брокеров",
  NON_LIVE_TRADING_MODE: "Неживой контур",
};

const EXPOSURE_CAPABILITIES: ReadonlySet<Capability> = new Set([
  "ORDER_EXECUTION",
  "PROTECTION_EXECUTION",
]);

export function findUnavailable(
  capabilities: CapabilitySet,
  capability: Capability,
): UnavailableCapability | null {
  return capabilities.unavailable.find((item) => item.capability === capability) ?? null;
}

export function createCapabilityGate(options: CapabilityGateOptions): HTMLElement {
  const children = options.children ?? [];
  const listed = findUnavailable(options.capabilities, options.capability);
  if (listed !== null) {
    return deferredGate(options.capability, listed.reason, children, listed.owner);
  }
  if (!options.capabilities.supported.includes(options.capability)) {
    return deferredGate(
      options.capability,
      "Эта возможность не включена в текущий контур.",
      children,
    );
  }
  const exposureBlock = exposureRefusal(options.capability, options.runtime);
  if (exposureBlock !== null) {
    return deferredGate(options.capability, exposureBlock, children);
  }
  const pass = document.createElement("div");
  append(pass, ...children);
  return pass;
}

function exposureRefusal(capability: Capability, runtime: RuntimeHealthDto | undefined): string | null {
  if (!EXPOSURE_CAPABILITIES.has(capability)) return null;
  if (runtime === undefined) {
    return "Нет фактов рантайма: новая экспозиция закрыта.";
  }
  const parts: string[] = [];
  if (!runtime.execution_authorized) parts.push("исполнение Vox выключено");
  if (!runtime.new_exposure_allowed) parts.push("новая экспозиция запрещена");
  if (parts.length === 0) return null;
  return `${parts.join(". ")}.`;
}

function deferredGate(
  capability: Capability,
  body: string,
  children: HTMLElement[],
  owner?: string,
): HTMLElement {
  for (const child of children) disableControl(child);
  const gate = createDeferred({
    title: CAPABILITY_TITLE[capability],
    body,
    ...(owner !== undefined && owner !== "" ? { owner } : {}),
    actions: children,
  });
  const code = gate.querySelector(".vox-deferred__head");
  if (code instanceof HTMLElement) {
    const badge = document.createElement("span");
    badge.className = "vox-dep";
    badge.textContent = capability;
    code.append(badge);
  }
  const block = (event: Event): void => {
    event.preventDefault();
    event.stopPropagation();
    event.stopImmediatePropagation();
  };
  gate.addEventListener("click", block, true);
  gate.addEventListener("pointerdown", block, true);
  return gate;
}

function disableControl(node: HTMLElement): void {
  node.classList.add("is-disabled");
  node.setAttribute("aria-disabled", "true");
  if (isFormControl(node)) node.disabled = true;
  const nested = Array.from(node.querySelectorAll("button, input, select, textarea, [role='button']"));
  for (const item of nested) {
    const el = item as HTMLElement;
    el.classList.add("is-disabled");
    el.setAttribute("aria-disabled", "true");
    if (isFormControl(el)) el.disabled = true;
  }
}

function isFormControl(
  node: HTMLElement,
): node is HTMLButtonElement | HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement {
  return (
    node instanceof HTMLButtonElement ||
    node instanceof HTMLInputElement ||
    node instanceof HTMLSelectElement ||
    node instanceof HTMLTextAreaElement
  );
}
