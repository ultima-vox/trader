import { AccountStore } from "../account";
import type { CapabilitySet, RuntimeHealthDto } from "@vox/api-client";
import { freezeInstrumentRef } from "../instrument";
import { empty } from "../state";
import { LayoutStore } from "../workspace";
import { createAppShell } from "./app-shell";
import { createCapabilityGate } from "./capability-gate";
import { createDataState } from "./data-state";
import { append, el } from "./dom";
import { createWidget } from "./widget";
import { createWorkspaceGrid } from "./workspace-grid";

export function mountFoundationPlayground(root: HTMLElement): void {
  root.classList.add("vox-root");
  root.dataset.density = "compact";
  root.dataset.theme = "dark";

  const runtime = stoppedRuntime();
  const { element, body } = createAppShell({
    environment: "SANDBOX",
    accountStore: new AccountStore(),
    runtime,
  });

  const capabilities = playgroundCapabilities();
  const linkedRef = freezeInstrumentRef({
    provider: "T_INVEST",
    uid: "SBER",
    ticker: "SBER",
    class_code: "TQBR",
  });
  const pinnedRef = freezeInstrumentRef({
    provider: "T_INVEST",
    uid: "GAZP",
    ticker: "GAZP",
    class_code: "TQBR",
  });

  const emptyState = empty();
  const linked = createWidget({
    title: "Связанный контекст",
    instrument: linkedRef,
    mode: "LINKED",
    dataKind: emptyState.kind,
    body: createDataState({ state: emptyState }),
  });

  const execGate = createCapabilityGate({
    capabilities,
    capability: "ORDER_EXECUTION",
    runtime,
    children: [actionButton("Купить"), actionButton("Продать")],
  });
  const marketGate = createCapabilityGate({
    capabilities,
    capability: "MARKET_DATA",
    runtime,
  });
  const accountGate = createCapabilityGate({
    capabilities,
    capability: "ACCOUNT_READ_SIDE",
    runtime,
  });
  const pinnedBody = el("div", "vox-stack vox-gap-2");
  append(pinnedBody, marketGate, accountGate, execGate);

  const pinned = createWidget({
    title: "Закреплённый контекст",
    instrument: pinnedRef,
    mode: "PINNED",
    body: pinnedBody,
  });

  const grid = createWorkspaceGrid({
    workspaceId: "foundation",
    layoutStore: new LayoutStore(layoutStorage()),
    items: [
      { id: "linked", colSpan: 6, element: linked, col: 0, row: 0, rowSpan: 4 },
      { id: "pinned", colSpan: 6, element: pinned, col: 6, row: 0, rowSpan: 4 },
    ],
  });

  append(body, grid);
  append(root, element);
}

function actionButton(label: string): HTMLButtonElement {
  const button = el("button", "vox-btn vox-btn--secondary", label);
  button.type = "button";
  return button;
}

function layoutStorage(): Storage {
  try {
    if (typeof sessionStorage !== "undefined") return sessionStorage;
  } catch {
    /* fall through */
  }
  const data = new Map<string, string>();
  return {
    get length() {
      return data.size;
    },
    clear() {
      data.clear();
    },
    getItem(key: string) {
      return data.get(key) ?? null;
    },
    key(index: number) {
      return [...data.keys()][index] ?? null;
    },
    removeItem(key: string) {
      data.delete(key);
    },
    setItem(key: string, value: string) {
      data.set(key, value);
    },
  };
}

function playgroundCapabilities(): CapabilitySet {
  return {
    provider: "T_INVEST",
    environment: "SANDBOX",
    supported: ["RUNTIME_HEALTH"],
    unavailable: [
      {
        capability: "MARKET_DATA",
        reason: "нет проекции рыночных данных",
        owner: "#38",
      },
      {
        capability: "ACCOUNT_READ_SIDE",
        reason: "нет read-side счетов",
        owner: "#17",
      },
      {
        capability: "ORDER_EXECUTION",
        reason: "нет порта исполнения",
        owner: "#10",
      },
    ],
  };
}

function stoppedRuntime(): RuntimeHealthDto {
  return {
    state: "STOPPED",
    reason_code: "SHUTDOWN_COMPLETE",
    reason: "рантайм не подключён",
    provider: "T_INVEST",
    environment: "SANDBOX",
    account_display: "—",
    runtime_epoch: 0,
    connected: false,
    unresolved_unknown_count: 0,
    open_order_count: 0,
    active_stop_count: 0,
    stream_states: [],
    persistence_healthy: true,
    execution_authorized: false,
    new_exposure_allowed: false,
  };
}
