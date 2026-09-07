import type {
  CapabilitySet,
  InstrumentSummaryDto,
  MutationReceiptDto,
  OrderSideDto,
  RiskStatusDto,
} from "@vox/api-client";
import { AccountStore } from "../account";
import { bindCommand, type CommandHandle } from "../command";
import {
  establishAndLoadPlatform,
  loadPlatform,
  type BrowserSession,
  type PlatformAccount,
  type PlatformSnapshot,
} from "../platform";
import { LayoutStore } from "../workspace";
import { VoxService } from "../vox";
import { createAppShell } from "./app-shell";
import { append, clear, el } from "./dom";
import { createDenseTable } from "./dense-table";
import { createInstrumentPicker, createOrderTicket } from "./trading-primitives";
import { createWorkspaceGrid } from "./workspace-grid";

const DENSITIES = ["compact", "standard", "comfortable"] as const;

export function mountApplication(root: HTMLElement): void {
  root.classList.add("vox-root");
  root.dataset.density = "compact";
  root.dataset.theme = "dark";
  const store = new AccountStore();
  const service = new VoxService(store);
  showSession(root, store, service);
}

function showSession(root: HTMLElement, store: AccountStore, service: VoxService): void {
  clear(root);
  const form = el("form", "vox-session");
  const credential = input("Bootstrap credential", "password");
  credential.name = "bootstrap-credential";
  credential.autocomplete = "current-password";
  credential.required = true;
  const submit = button("Open session", "vox-btn vox-btn--primary");
  submit.type = "submit";
  const status = statusNode();
  append(
    form,
    el("h1", undefined, "Vox Trader"),
    el("p", "vox-text--caption", "Credential is exchanged for an HttpOnly session cookie and is never stored in browser storage."),
    credential,
    submit,
    status,
  );
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    submit.disabled = true;
    status.textContent = "Loading platform session…";
    const secret = credential.value;
    credential.value = "";
    void establishAndLoadPlatform(service, secret).then((result) => {
      if (!result.ok) {
        status.textContent = `${result.error.status}: ${result.error.body.message}`;
        submit.disabled = false;
        credential.focus();
        return;
      }
      mountPlatformSession(root, store, service, result.value.session, result.value);
    });
  });
  append(root, form);
  credential.focus();
}

function mountPlatformSession(
  root: HTMLElement,
  store: AccountStore,
  service: VoxService,
  session: BrowserSession,
  initial: PlatformSnapshot,
): void {
  let snapshot = initial;
  let renderVersion = 0;
  const render = (): void => {
    const version = ++renderVersion;
    if (store.current() === null && snapshot.accounts[0] !== undefined) {
      store.switchTo(snapshot.accounts[0].scope);
      return;
    }
    void showPlatform(root, store, service, snapshot, refresh, () => version === renderVersion);
  };
  const refresh = async (): Promise<void> => {
    const loaded = await loadPlatform(service, session);
    if (!loaded.ok) throw loaded.error;
    snapshot = loaded.value;
    if (findCurrentAccount(store, snapshot) === undefined && snapshot.accounts[0] !== undefined) {
      store.switchTo(snapshot.accounts[0].scope);
      return;
    }
    render();
  };
  store.subscribe(render);
  render();
}

async function showPlatform(
  root: HTMLElement,
  store: AccountStore,
  service: VoxService,
  snapshot: PlatformSnapshot,
  refresh: () => Promise<void>,
  isCurrent: () => boolean,
): Promise<void> {
  const account = findCurrentAccount(store, snapshot);
  const [capabilities, scopedRuntime] = account === undefined
    ? [undefined, undefined]
    : await Promise.all([service.capabilities(), service.runtime()]);
  if (!isCurrent()) return;
  clear(root);
  const body = el("main", "vox-platform");
  append(body, createDensityControl(root));

  if (account === undefined) {
    append(body, connectionTable(snapshot), onboardingWidget(service, refresh));
  } else if (
    capabilities === undefined || !capabilities.ok ||
    scopedRuntime === undefined || !scopedRuntime.ok
  ) {
    const failed = capabilities !== undefined && !capabilities.ok ? capabilities : scopedRuntime;
    const message = failed !== undefined && "error" in failed
      ? `${failed.error.status}: ${failed.error.body.message}`
      : "Selected-scope runtime or capability contract unavailable";
    append(body, connectionTable(snapshot), el("div", "vox-deferred", message));
  } else {
    const workspace = await tradingWorkspace(
      service,
      snapshot,
      account,
      capabilities.value,
      scopedRuntime.value,
      refresh,
    );
    if (!isCurrent()) return;
    append(body, workspace);
  }

  const shell = createAppShell({
    environment: account?.scope.environment ?? snapshot.processRuntime.environment,
    accountStore: store,
    runtime: scopedRuntime?.ok === true ? scopedRuntime.value : snapshot.processRuntime,
    accounts: snapshot.accounts,
    body,
  });
  append(root, shell.element);
}

function onboardingWidget(service: VoxService, refresh: () => Promise<void>): HTMLElement {
  const widget = el("section", "vox-widget");
  const body = el("div", "vox-widget__body vox-stack vox-gap-2");
  const label = input("Connection label");
  label.value = "T-Invest Sandbox";
  const token = input("T-Invest Sandbox token", "password");
  token.autocomplete = "off";
  const create = button("Connect and discover accounts", "vox-btn vox-btn--primary");
  const accounts = el("div", "vox-stack vox-gap-1");
  const status = statusNode();
  create.addEventListener("click", () => {
    create.disabled = true;
    status.textContent = "Validating encrypted connection and discovering accounts…";
    const brokerToken = token.value;
    token.value = "";
    void service.createConnection({
      provider: "T_INVEST",
      environment: "SANDBOX",
      display_label: label.value.trim() || "T-Invest Sandbox",
      credential: brokerToken,
    }).then(async (created) => {
      if (!created.ok) {
        status.textContent = `${created.error.status}: ${created.error.body.message}`;
        create.disabled = false;
        return;
      }
      const details = await service.connectionDetails(created.value.connection_id);
      if (!details.ok) {
        status.textContent = `${details.error.status}: ${details.error.body.message}`;
        create.disabled = false;
        return;
      }
      clear(accounts);
      const accessible = details.value.accounts.filter((item) => item.accessible);
      if (accessible.length === 0) {
        status.textContent = "Broker returned no accessible Sandbox account.";
        create.disabled = false;
        return;
      }
      status.textContent = "Select Sandbox account:";
      for (const discovered of accessible) {
        const bind = button(
          `${discovered.display_name ?? discovered.account_type} · ${maskId(discovered.provider_account_id)}`,
          "vox-btn vox-btn--ghost",
        );
        bind.addEventListener("click", () => {
          bind.disabled = true;
          void bindAndAuthorize(service, created.value.connection_id, discovered.provider_account_id, status, refresh);
        });
        append(accounts, bind);
      }
    });
  });
  append(widget, el("div", "vox-widget__header", "T-Invest Sandbox setup"), body);
  append(
    body,
    el("p", "vox-text--caption", "Token goes only to Vox secure connection API. Source edits are unnecessary."),
    label,
    token,
    create,
    status,
    accounts,
  );
  return widget;
}

async function bindAndAuthorize(
  service: VoxService,
  connectionId: string,
  providerAccountId: string,
  status: HTMLElement,
  refresh: () => Promise<void>,
): Promise<void> {
  const accountId = `vox-account:${globalThis.crypto.randomUUID()}`;
  const bound = await service.bindAccount(connectionId, providerAccountId, accountId);
  if (!bound.ok) {
    status.textContent = `${bound.error.status}: ${bound.error.body.message}`;
    return;
  }
  const details = await service.connectionDetails(connectionId);
  if (!details.ok) {
    status.textContent = `${details.error.status}: ${details.error.body.message}`;
    return;
  }
  const current = details.value.execution_authorizations.find(
    (item) => item.provider_account_id === providerAccountId,
  );
  if (current === undefined) {
    status.textContent = "Execution authorization record missing after binding.";
    return;
  }
  const authorized = await service.authorizeExecution(connectionId, {
    provider_account_id: providerAccountId,
    mode: "MANUAL_ALLOWED",
    expected_authorization_revision: current.authorization_revision,
  });
  if (!authorized.ok) {
    status.textContent = `${authorized.error.status}: ${authorized.error.body.message}`;
    return;
  }
  status.textContent = "Binding active. Starting account runtime…";
  await refresh();
}

async function tradingWorkspace(
  service: VoxService,
  snapshot: PlatformSnapshot,
  account: PlatformAccount,
  capabilities: CapabilitySet,
  runtime: PlatformSnapshot["processRuntime"],
  refreshPlatform: () => Promise<void>,
): Promise<HTMLElement> {
  let selected: InstrumentSummaryDto | undefined;
  let command = newCommand(account);
  let lastEntry: MutationReceiptDto | undefined;
  let lastEntryInstrumentUid: string | undefined;
  let quoteLast: string | undefined;
  let selectionVersion = 0;
  let submitting = false;
  const ticketHost = el("div");
  const pickerBody = el("div", "vox-widget__body vox-stack vox-gap-1");
  const status = statusNode();
  const quantity = numberInput("Quantity, lots", "1");
  quantity.min = "1";
  quantity.step = "1";
  const orderType = document.createElement("select");
  orderType.className = "vox-input vox-input__field";
  for (const value of ["MARKET", "LIMIT"] as const) {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = value;
    orderType.append(option);
  }
  const price = input("Limit price (exact decimal)");

  const submit = async (side: OrderSideDto): Promise<void> => {
    if (selected === undefined || submitting) return;
    const submittedInstrument = selected;
    const lots = Number(quantity.value);
    if (!Number.isSafeInteger(lots) || lots <= 0) {
      status.textContent = "Quantity must be positive integer lots.";
      return;
    }
    if (orderType.value === "LIMIT" && price.value.trim() === "") {
      status.textContent = "Limit order requires price.";
      return;
    }
    if (!window.confirm(`Submit ${side} ${lots} lot(s) ${submittedInstrument.identity.ticker} in ${account.scope.environment}?`)) return;
    submitting = true;
    status.textContent = "Server risk check and broker dispatch…";
    command = newCommand(account);
    paintTicket();
    const result = await service.submitOrder(command, {
      instrument_id: submittedInstrument.identity.uid,
      side,
      order_type: orderType.value === "LIMIT" ? "LIMIT" : "MARKET",
      quantity_lots: lots,
      ...(orderType.value === "LIMIT" ? { price: price.value.trim() } : {}),
      price_convention: "SETTLEMENT_CURRENCY",
      time_in_force: "DAY",
      confirm_margin_trade: false,
    });
    if (!result.ok) {
      submitting = false;
      status.textContent = `${result.error.status}: ${result.error.body.message}`;
      paintTicket();
      return;
    }
    submitting = false;
    command = result.handle;
    lastEntry = result.handle.receipt;
    lastEntryInstrumentUid = submittedInstrument.identity.uid;
    status.textContent = receiptText(lastEntry);
    paintTicket();
    await refreshBrokerPanels();
  };

  const paintTicket = (): void => {
    clear(ticketHost);
    const controls = el("div", "vox-stack vox-gap-1");
    append(controls, quantity, orderType, price, status);
    const ticket = createOrderTicket({
      account,
      session: snapshot.session,
      capabilities,
      runtime,
      command,
      ...(selected === undefined ? {} : { instrument: selected }),
      onAction: (side) => void submit(side),
    });
    if (submitting) {
      for (const action of Array.from(ticket.querySelectorAll<HTMLButtonElement>(".vox-ticket__action"))) {
        action.disabled = true;
      }
    }
    ticket.querySelector(".vox-ticket__body")?.prepend(controls);
    append(ticketHost, ticket);
  };
  paintTicket();

  const picker = el("section", "vox-widget");
  const search = input("Ticker or instrument name");
  const searchButton = button("Search broker catalogue", "vox-btn vox-btn--primary");
  const searchStatus = statusNode();
  searchButton.addEventListener("click", () => {
    const query = search.value.trim();
    if (query === "") return;
    searchButton.disabled = true;
    searchStatus.textContent = "Searching T-Invest…";
    void service.instruments(account.scope.provider, query).then((result) => {
      searchButton.disabled = false;
      clear(pickerBody);
      append(pickerBody, search, searchButton, searchStatus);
      if (!result.ok) {
        searchStatus.textContent = `${result.error.status}: ${result.error.body.message}`;
        return;
      }
      searchStatus.textContent = `${result.value.length} broker result(s)`;
      append(pickerBody, createInstrumentPicker({
        instruments: result.value,
        onSelect: (instrument) => {
          const version = ++selectionVersion;
          selected = instrument;
          command = newCommand(account);
          quoteLast = undefined;
          lastEntry = undefined;
          lastEntryInstrumentUid = undefined;
          searchStatus.textContent = `Selected ${instrument.identity.ticker}; loading broker quote…`;
          paintTicket();
          void service.quote(account.scope.provider, instrument.identity.uid).then((quote) => {
            if (version !== selectionVersion || selected?.identity.uid !== instrument.identity.uid) return;
            if (!quote.ok) {
              searchStatus.textContent = `${quote.error.status}: quote unavailable (${quote.error.body.code})`;
              return;
            }
            quoteLast = quote.value.last ?? undefined;
            searchStatus.textContent = `${instrument.identity.ticker} last ${quote.value.last ?? "unavailable"} · ${quote.value.freshness.stream} · age ${quote.value.freshness.age_ms} ms`;
          });
        },
      }));
    });
  });
  append(pickerBody, search, searchButton, searchStatus);
  append(picker, el("div", "vox-widget__header", "Instrument / market data"), pickerBody);

  const risk = await service.riskStatus();
  const riskWidget = risk.ok
    ? riskControl(service, risk.value, refreshPlatform)
    : errorWidget("Risk", scopedError(risk));

  const brokerWidget = el("section", "vox-widget");
  const brokerBody = el("div", "vox-widget__body vox-stack vox-gap-1");
  const refreshButton = button("Refresh broker evidence", "vox-btn vox-btn--ghost");
  append(brokerWidget, el("div", "vox-widget__header", "Broker / reconciliation"), brokerBody);

  const refreshBrokerPanels = async (): Promise<void> => {
    refreshButton.disabled = true;
    clear(brokerBody);
    append(brokerBody, refreshButton, el("span", undefined, "Reading T-Invest and runtime journal…"));
    const [portfolio, positions, orders, stops, mutations, reconciliation] = await Promise.all([
      service.portfolio(), service.positions(), service.orders(), service.stopOrders(),
      service.mutations(), service.reconciliation(),
    ]);
    clear(brokerBody);
    refreshButton.disabled = false;
    append(brokerBody, refreshButton);
    append(brokerBody, jsonEvidence("Portfolio", portfolio));
    append(brokerBody, positionsEvidence(service, account, positions, () => selected, refreshBrokerPanels));
    append(brokerBody, jsonEvidence("Orders", orders));
    append(brokerBody, jsonEvidence("Broker stops / protection", stops));
    append(brokerBody, jsonEvidence("Mutation ACK/FILL/REJECT/UNKNOWN", mutations));
    append(brokerBody, jsonEvidence("Reconciliation", reconciliation));
  };
  refreshButton.addEventListener("click", () => void refreshBrokerPanels());

  const protectionWidget = protectionControl(
    service,
    account,
    () => selected,
    () => quoteLast,
    () => selected?.identity.uid === lastEntryInstrumentUid ? lastEntry : undefined,
    refreshBrokerPanels,
  );
  await refreshBrokerPanels();

  return createWorkspaceGrid({
    workspaceId: "platform",
    layoutStore: new LayoutStore(sessionStorage),
    items: [
      { id: "connections", col: 0, row: 0, colSpan: 4, rowSpan: 5, element: connectionTable(snapshot) },
      { id: "risk", col: 0, row: 5, colSpan: 4, rowSpan: 4, element: riskWidget },
      { id: "instrument-picker", col: 4, row: 0, colSpan: 4, rowSpan: 5, element: picker },
      { id: "order-ticket", col: 8, row: 0, colSpan: 4, rowSpan: 5, element: ticketHost },
      { id: "protection", col: 4, row: 5, colSpan: 4, rowSpan: 4, element: protectionWidget },
      { id: "broker-evidence", col: 8, row: 5, colSpan: 4, rowSpan: 8, element: brokerWidget },
    ],
  });
}

function riskControl(service: VoxService, risk: RiskStatusDto, refresh: () => Promise<void>): HTMLElement {
  const widget = el("section", "vox-widget");
  const body = el("div", "vox-widget__body vox-stack vox-gap-1");
  const status = statusNode(`${risk.state} · policy revision ${risk.policy_revision}`);
  const normal = button("Set risk state NORMAL", "vox-btn vox-btn--primary");
  normal.disabled = risk.state === "NORMAL";
  normal.addEventListener("click", () => {
    if (!window.confirm("Allow new exposure for selected Sandbox account?")) return;
    normal.disabled = true;
    void service.changeRiskState({
      scope: risk.scope,
      state: "NORMAL",
      expected_policy_revision: risk.policy_revision,
      reason: "manual Sandbox RC1 session",
    }).then(async (result) => {
      if (!result.ok) {
        status.textContent = `${result.error.status}: ${result.error.body.message}`;
        normal.disabled = false;
        return;
      }
      status.textContent = `${result.value.state} · policy revision ${result.value.policy_revision}`;
      await refresh();
    });
  });
  append(widget, el("div", "vox-widget__header", "Server risk"), body);
  append(body, status, normal);
  return widget;
}

function protectionControl(
  service: VoxService,
  account: PlatformAccount,
  selected: () => InstrumentSummaryDto | undefined,
  quoteLast: () => string | undefined,
  lastEntry: () => MutationReceiptDto | undefined,
  refresh: () => Promise<void>,
): HTMLElement {
  const widget = el("section", "vox-widget");
  const body = el("div", "vox-widget__body vox-stack vox-gap-1");
  const side = document.createElement("select");
  side.className = "vox-input vox-input__field";
  for (const value of ["LONG", "SHORT"] as const) {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = value;
    side.append(option);
  }
  const lots = numberInput("Protected quantity, lots", "1");
  const reference = input("Current/reference price");
  const trigger = input("Stop-loss trigger price");
  const status = statusNode();
  const submit = button("Create broker stop-loss", "vox-btn vox-btn--primary");
  submit.addEventListener("click", () => {
    const instrument = selected();
    if (instrument === undefined) {
      status.textContent = "Select instrument first.";
      return;
    }
    const ref = reference.value.trim() || quoteLast();
    if (ref === undefined || trigger.value.trim() === "") {
      status.textContent = "Reference and stop-loss trigger are required.";
      return;
    }
    if (!window.confirm(`Submit ${side.value} stop-loss for ${instrument.identity.ticker}?`)) return;
    submit.disabled = true;
    const entryReservationId = lastEntry()?.risk_decision?.reservation_id;
    void service.submitProtection({
      scope: account.scope,
      instrument_id: instrument.identity.uid,
      client_request_id: globalThis.crypto.randomUUID(),
      quantity_lots: Number(lots.value),
      position_side: side.value === "SHORT" ? "SHORT" : "LONG",
      reference_price: ref,
      price_convention: "SETTLEMENT_CURRENCY",
      confirm_margin_trade: false,
      plan: { stop_loss_trigger_price: trigger.value.trim() },
      ...(entryReservationId === undefined || entryReservationId === null
        ? {}
        : { entry_reservation_id: entryReservationId }),
    }).then(async (result) => {
      submit.disabled = false;
      if (!result.ok) {
        status.textContent = `${result.error.status}: ${result.error.body.message}`;
        return;
      }
      status.textContent = receiptText(result.value);
      await refresh();
    });
  });
  append(widget, el("div", "vox-widget__header", "Broker-native protection"), body);
  append(body, side, lots, reference, trigger, submit, status);
  return widget;
}

function positionsEvidence(
  service: VoxService,
  account: PlatformAccount,
  result: Awaited<ReturnType<VoxService["positions"]>>,
  selected: () => InstrumentSummaryDto | undefined,
  refresh: () => Promise<void>,
): HTMLElement {
  const section = el("details");
  append(section, el("summary", undefined, "Positions / close-reduce"));
  if (!result.ok) {
    append(section, el("pre", "vox-num", scopedError(result)));
    return section;
  }
  append(section, el("pre", "vox-num", JSON.stringify(result.value, null, 2)));
  for (const position of result.value.filter((item) => item.quantity_units !== 0)) {
    const close = button(`Close ${position.instrument_uid}`, "vox-btn vox-btn--danger");
    close.addEventListener("click", () => {
      const instrument = selected();
      if (instrument?.identity.uid !== position.instrument_uid) {
        window.alert("Select matching instrument first so Vox can normalize units to broker lots.");
        return;
      }
      const quantityLots = Math.abs(position.quantity_units) / instrument.lot_size;
      if (!Number.isSafeInteger(quantityLots) || quantityLots <= 0) {
        window.alert("Broker position is not an exact whole-lot quantity.");
        return;
      }
      if (!window.confirm(`Close ${quantityLots} lot(s) ${instrument.identity.ticker}?`)) return;
      close.disabled = true;
      void service.submitOrder(newCommand(account), {
        instrument_id: instrument.identity.uid,
        side: position.quantity_units > 0 ? "SELL" : "BUY",
        order_type: "MARKET",
        quantity_lots: quantityLots,
        price_convention: "SETTLEMENT_CURRENCY",
        time_in_force: "DAY",
        confirm_margin_trade: false,
      }).then(async (submitted) => {
        close.disabled = false;
        if (!submitted.ok) {
          window.alert(`${submitted.error.status}: ${submitted.error.body.message}`);
          return;
        }
        await refresh();
      });
    });
    append(section, close);
  }
  return section;
}

function jsonEvidence<T>(title: string, result: { ok: true; value: T } | object): HTMLElement {
  const details = el("details");
  append(details, el("summary", undefined, title));
  if ("ok" in result && result.ok === true && "value" in result) {
    append(details, el("pre", "vox-num", JSON.stringify(result.value, null, 2)));
  } else {
    append(details, el("pre", "vox-num", scopedError(result)));
  }
  return details;
}

function connectionTable(snapshot: PlatformSnapshot): HTMLElement {
  const widget = el("section", "vox-widget");
  append(
    widget,
    el("div", "vox-widget__header", "Broker connections"),
    createDenseTable({
      columns: [
        { id: "label", header: "Connection" },
        { id: "environment", header: "Environment" },
        { id: "health", header: "Health" },
      ],
      rows: snapshot.connections.map((connection) => ({
        id: connection.connection_id,
        cells: [connection.display_label, connection.environment, connection.health.state],
      })),
      caption: "Vox API metadata; credentials never enter DOM.",
    }),
  );
  return widget;
}

function createDensityControl(root: HTMLElement): HTMLElement {
  const control = el("div", "vox-density");
  control.setAttribute("aria-label", "Density");
  for (const density of DENSITIES) {
    const choice = button(density, "vox-btn vox-btn--ghost");
    choice.dataset.densityChoice = density;
    choice.addEventListener("click", () => { root.dataset.density = density; });
    append(control, choice);
  }
  return control;
}

function findCurrentAccount(store: AccountStore, snapshot: PlatformSnapshot): PlatformAccount | undefined {
  const current = store.current();
  return snapshot.accounts.find((account) =>
    current !== null &&
    account.scope.broker_connection_id === current.broker_connection_id &&
    account.scope.account_id === current.account_id &&
    account.scope.environment === current.environment &&
    account.scope.provider === current.provider
  );
}

function newCommand(account: PlatformAccount): CommandHandle {
  return bindCommand(account.scope, undefined, undefined, {
    providerAccountId: account.providerAccountId,
    accountDisplay: account.accountDisplay,
    connectionLabel: account.connectionLabel,
  });
}

function input(label: string, type = "text"): HTMLInputElement {
  const node = document.createElement("input");
  node.type = type;
  node.className = "vox-input vox-input__field";
  node.placeholder = label;
  node.setAttribute("aria-label", label);
  return node;
}

function numberInput(label: string, value: string): HTMLInputElement {
  const node = input(label, "number");
  node.value = value;
  return node;
}

function button(text: string, className: string): HTMLButtonElement {
  const node = el("button", className, text);
  node.type = "button";
  return node;
}

function statusNode(text = ""): HTMLElement {
  const node = el("div", "vox-text--caption", text);
  node.setAttribute("role", "status");
  return node;
}

function errorWidget(title: string, message: string): HTMLElement {
  const widget = el("section", "vox-widget");
  append(widget, el("div", "vox-widget__header", title), el("div", "vox-deferred", message));
  return widget;
}

function scopedError(result: object): string {
  if ("error" in result) {
    const error = result.error as { status: number; body: { message: string } };
    return `${error.status}: ${error.body.message}`;
  }
  if ("stale" in result) return "Account selection changed; refresh required.";
  if ("noContext" in result) return "No account selected.";
  return "Unavailable.";
}

function receiptText(receipt: MutationReceiptDto | undefined): string {
  return receipt === undefined
    ? "No receipt"
    : `${receipt.state} · ${receipt.decision} · ${receipt.broker_order_id ?? receipt.broker_stop_order_id ?? receipt.logical_request_id}`;
}

function maskId(id: string): string {
  return id.length <= 4 ? id : `****${id.slice(-4)}`;
}
