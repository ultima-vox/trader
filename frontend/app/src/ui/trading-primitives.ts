import type {
  Capability,
  CapabilitySet,
  InstrumentSummaryDto,
  OrderSideDto,
  RuntimeHealthDto,
} from "@vox/api-client";
import type { CommandHandle } from "../command";
import type { BrowserSession, PlatformAccount } from "../platform";
import { append, clear, el } from "./dom";
import { createDeferred } from "./deferred";
import { createEnvBadge, providerLabel } from "./env-badge";

export type ExecutionGate = Readonly<{ allowed: boolean; reason?: string }>;

export function executionGate(
  account: PlatformAccount,
  session: BrowserSession,
  capabilities: CapabilitySet,
  runtime: RuntimeHealthDto,
): ExecutionGate {
  if (!session.csrfReady) return blocked("CSRF session state unavailable");
  if (!account.connectionEnabled) return blocked("broker connection disabled");
  if (account.connectionHealth.state !== "HEALTHY") {
    return blocked(`connection ${account.connectionHealth.state}: ${account.connectionHealth.reason_code}`);
  }
  if (!account.accessible) return blocked("broker account inaccessible");
  if (account.executionAuthorization?.mode === undefined || account.executionAuthorization.mode === "DISABLED") {
    return blocked("execution authorization disabled");
  }
  const permission = account.scope.environment === "SANDBOX"
    ? "SUBMIT_SANDBOX_ORDERS"
    : "SUBMIT_PRODUCTION_MANUAL_ORDERS";
  if (!session.effectivePermissions.has(permission)) return blocked(`missing permission ${permission}`);
  const unavailable = capabilities.unavailable.find((item) => item.capability === "ORDER_EXECUTION");
  if (!capabilities.supported.includes("ORDER_EXECUTION")) {
    return blocked(unavailable?.reason ?? "ORDER_EXECUTION unavailable");
  }
  if (!runtime.execution_authorized) return blocked("runtime execution authorization false");
  if (!runtime.new_exposure_allowed) {
    return blocked(`${runtime.state}: ${runtime.reason_code}`);
  }
  return Object.freeze({ allowed: true });
}

export type ExecutionTargetIndicatorOptions = {
  account: PlatformAccount;
  command: CommandHandle;
};

export function createExecutionTargetIndicator(
  options: ExecutionTargetIndicatorOptions,
): HTMLElement {
  const { account, command } = options;
  const root = el("div", "vox-ticket__target is-frozen");
  if (command.scope.environment === "PRODUCTION") root.classList.add("is-live");
  if (
    account.scope.broker_connection_id !== command.scope.broker_connection_id ||
    account.scope.account_id !== command.scope.account_id
  ) root.classList.add("is-mismatch");
  const display = command.targetDisplay ?? {
    accountDisplay: command.scope.account_id,
    connectionLabel: command.scope.broker_connection_id,
    providerAccountId: command.scope.account_id,
  };
  const identity = el("span", "vox-ticket__target-account");
  append(
    identity,
    el("span", "vox-ticket__target-name", display.accountDisplay),
    el(
      "span",
      "vox-ticket__target-broker",
      `${providerLabel(command.scope.provider)} · ${display.connectionLabel} · ${maskId(display.providerAccountId)}`,
    ),
  );
  const facts = el("span", "vox-ticket__target-lock", "ЗАФИКСИРОВАНО");
  facts.title = `${command.scope.broker_connection_id} · ${command.scope.account_id} · ${command.logicalRequestId}`;
  append(root, identity, createEnvBadge(command.scope.environment), facts);
  return root;
}

export type OrderTicketOptions = {
  account: PlatformAccount;
  session: BrowserSession;
  capabilities: CapabilitySet;
  runtime: RuntimeHealthDto;
  command: CommandHandle;
  instrument?: InstrumentSummaryDto;
  onAction?: (side: OrderSideDto, command: CommandHandle) => void;
};

export function createOrderTicket(options: OrderTicketOptions): HTMLElement {
  const root = el("section", "vox-widget vox-ticket");
  root.dataset.primitive = "order-ticket";
  const header = el("div", "vox-widget__header", "Order Ticket");
  const body = el("div", "vox-ticket__body");
  append(body, createExecutionTargetIndicator({ account: options.account, command: options.command }));

  const instrument = options.instrument;
  const instrumentRow = el("div", "vox-ticket__instrument");
  append(
    instrumentRow,
    el(
      "span",
      undefined,
      instrument === undefined
        ? "Инструмент не выбран"
        : `${instrument.identity.ticker} · ${instrument.identity.class_code}`,
    ),
    el("span", "vox-num", instrument?.min_price_increment ?? "—"),
  );
  append(body, instrumentRow);

  const gate = instrument === undefined
    ? blocked("instrument required")
    : executionGate(options.account, options.session, options.capabilities, options.runtime);
  append(body, createCapabilityRegion(options.capabilities, "PROTECTION_EXECUTION", "Защита позиции"));
  append(body, createCapabilityRegion(options.capabilities, "RISK_VERDICT", "Результат риск-проверки"));

  const actions = el("div", "vox-ticket__actions");
  append(
    actions,
    actionButton("BUY", "Купить", gate, options),
    actionButton("SELL", "Продать", gate, options),
  );
  append(body, actions);
  append(body, createCommandLifecycle(options.account, options.command));
  if (!gate.allowed) append(body, el("div", "vox-ticket__hint", gate.reason ?? "execution unavailable"));
  append(root, header, body);
  return root;
}

export function createCommandLifecycle(account: PlatformAccount, command: CommandHandle): HTMLElement {
  const receipt = command.receipt;
  const state = receipt?.state ?? "NOT_DISPATCHED";
  const root = el("div", state === "UNKNOWN_AFTER_DISPATCH" ? "vox-recon" : "vox-ticket__preview");
  root.dataset.commandState = state;
  append(root, el("strong", undefined, state));
  if (receipt !== undefined) {
    append(
      root,
      el("span", "vox-num", receipt.logical_request_id),
      el("span", undefined, receipt.decision),
      createExecutionTargetIndicator({ account, command }),
    );
  }
  return root;
}

function actionButton(
  side: OrderSideDto,
  label: string,
  gate: ExecutionGate,
  options: OrderTicketOptions,
): HTMLButtonElement {
  const button = el(
    "button",
    `vox-ticket__action vox-ticket__action--${side === "BUY" ? "buy" : "sell"}`,
  );
  button.type = "button";
  button.disabled = !gate.allowed;
  button.setAttribute("aria-disabled", String(!gate.allowed));
  if (!gate.allowed) button.classList.add("is-blocked");
  append(
    button,
    el("span", "vox-ticket__action-label", label),
    el("span", "vox-ticket__action-note", gate.allowed ? "готово к подтверждению" : gate.reason ?? "недоступно"),
  );
  button.addEventListener("click", () => options.onAction?.(side, options.command));
  return button;
}

function createCapabilityRegion(
  capabilities: CapabilitySet,
  capability: Capability,
  title: string,
): HTMLElement {
  if (capabilities.supported.includes(capability)) {
    return el("div", "vox-ticket__preview", `${title}: backend contract available`);
  }
  const unavailable = capabilities.unavailable.find((item) => item.capability === capability);
  return createDeferred({
    title,
    body: unavailable?.reason ?? `${capability} unavailable`,
    ...(unavailable?.owner === undefined ? {} : { owner: unavailable.owner }),
  });
}

export type ConfirmationOptions = {
  title: string;
  consequence: string;
  phrase: string;
  onConfirm: () => void;
};

export function createCapitalConfirmation(options: ConfirmationOptions): HTMLElement {
  const root = el("div", "vox-stack vox-gap-2");
  const input = document.createElement("input");
  input.className = "vox-input vox-input__field";
  input.autocomplete = "off";
  input.placeholder = options.phrase;
  const confirm = el("button", "vox-btn vox-btn--danger", "Подтвердить");
  confirm.type = "button";
  confirm.disabled = true;
  input.addEventListener("input", () => {
    confirm.disabled = input.value !== options.phrase;
  });
  confirm.addEventListener("click", () => {
    if (input.value === options.phrase) options.onConfirm();
  });
  append(root, el("strong", undefined, options.title), el("span", undefined, options.consequence), input, confirm);
  return root;
}

export type InstrumentPickerOptions = {
  instruments: readonly InstrumentSummaryDto[];
  onSelect: (instrument: InstrumentSummaryDto) => void;
};

export function createInstrumentPicker(options: InstrumentPickerOptions): HTMLElement {
  const root = el("div", "vox-stack vox-gap-1");
  const input = document.createElement("input");
  input.className = "vox-input vox-input__field";
  input.placeholder = "Тикер, площадка или UID";
  input.setAttribute("aria-label", "Инструмент");
  const list = el("div", "vox-menu");
  const paint = (): void => {
    clear(list);
    const query = input.value.trim().toLocaleUpperCase("ru-RU");
    const visible = options.instruments.filter((item) => {
      const identity = item.identity;
      return query === "" || [identity.ticker, identity.class_code, identity.uid]
        .some((value) => value.toLocaleUpperCase("ru-RU").includes(query));
    });
    if (visible.length === 0) {
      append(list, el("span", "vox-menu__item is-disabled", "Vox не вернул совпадений"));
      return;
    }
    for (const item of visible) {
      const row = el(
        "button",
        "vox-menu__item",
        `${item.identity.ticker} · ${item.identity.class_code} · ${item.currency}`,
      );
      row.type = "button";
      row.disabled = !item.tradable;
      row.addEventListener("click", () => options.onSelect(item));
      append(list, row);
    }
  };
  input.addEventListener("input", paint);
  paint();
  append(root, input, list);
  return root;
}

function blocked(reason: string): ExecutionGate {
  return Object.freeze({ allowed: false, reason });
}

function maskId(id: string): string {
  return id.length <= 4 ? id : `****${id.slice(-4)}`;
}
