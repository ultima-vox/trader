import type { CapabilitySet, InstrumentSummaryDto } from "@vox/api-client";
import { AccountStore } from "../account";
import { bindCommand } from "../command";
import { establishAndLoadPlatform, type PlatformAccount, type PlatformSnapshot } from "../platform";
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
  const credential = document.createElement("input");
  credential.type = "password";
  credential.name = "bootstrap-credential";
  credential.autocomplete = "current-password";
  credential.required = true;
  credential.className = "vox-input vox-input__field";
  credential.setAttribute("aria-label", "Bootstrap credential");
  const submit = el("button", "vox-btn vox-btn--primary", "Открыть сессию");
  submit.type = "submit";
  const status = el("div", "vox-session__status");
  status.setAttribute("role", "status");
  append(
    form,
    el("h1", undefined, "Vox Trader"),
    el("p", "vox-text--caption", "Credential обменивается на HttpOnly session cookie и не сохраняется в browser storage."),
    credential,
    submit,
    status,
  );
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    submit.disabled = true;
    status.textContent = "Загрузка platform/session…";
    const secret = credential.value;
    credential.value = "";
    void establishAndLoadPlatform(service, secret).then((result) => {
      if (!result.ok) {
        status.textContent = `${result.error.status}: ${result.error.body.message}`;
        submit.disabled = false;
        credential.focus();
        return;
      }
      if (result.value.accounts[0] !== undefined) store.switchTo(result.value.accounts[0].scope);
      let renderVersion = 0;
      const render = (): void => {
        const version = ++renderVersion;
        void showPlatform(root, store, service, result.value, () => version === renderVersion);
      };
      store.subscribe(render);
      render();
    });
  });
  append(root, form);
  credential.focus();
}

async function showPlatform(
  root: HTMLElement,
  store: AccountStore,
  service: VoxService,
  snapshot: PlatformSnapshot,
  isCurrent: () => boolean,
): Promise<void> {
  const account = findCurrentAccount(store, snapshot);
  const capabilities = account === undefined ? undefined : await service.capabilities();
  if (!isCurrent()) return;
  clear(root);
  const body = el("main", "vox-platform");
  const density = createDensityControl(root);
  append(body, density);

  if (account === undefined) {
    append(body, connectionTable(snapshot), el("div", "vox-deferred", "Vox не вернул active account binding. Demo account не создан."));
  } else if (capabilities === undefined || !capabilities.ok) {
    const message = capabilities !== undefined && "error" in capabilities
      ? `${capabilities.error.status}: ${capabilities.error.body.message}`
      : "Capability contract unavailable";
    append(body, connectionTable(snapshot), el("div", "vox-deferred", message));
  } else {
    const workspace = await tradingWorkspace(service, snapshot, account, capabilities.value);
    if (!isCurrent()) return;
    append(body, workspace);
  }

  const shell = createAppShell({
    environment: account?.scope.environment ?? snapshot.runtime.environment,
    accountStore: store,
    runtime: snapshot.runtime,
    accounts: snapshot.accounts,
    body,
  });
  append(root, shell.element);
}

async function tradingWorkspace(
  service: VoxService,
  snapshot: PlatformSnapshot,
  account: PlatformAccount,
  capabilities: CapabilitySet,
): Promise<HTMLElement> {
  const instruments = await service.instruments(account.scope.provider, "S");
  let selected = instruments.ok ? instruments.value[0] : undefined;
  const command = bindCommand(account.scope, undefined, undefined, {
    providerAccountId: account.providerAccountId,
    accountDisplay: account.accountDisplay,
    connectionLabel: account.connectionLabel,
  });
  const ticketHost = el("div");
  const paintTicket = (): void => {
    clear(ticketHost);
    append(ticketHost, createOrderTicket({
      account,
      session: snapshot.session,
      capabilities,
      runtime: snapshot.runtime,
      command,
      ...(selected === undefined ? {} : { instrument: selected }),
    }));
  };
  paintTicket();

  const picker = el("section", "vox-widget");
  append(picker, el("div", "vox-widget__header", "Инструмент"));
  const pickerBody = el("div", "vox-widget__body");
  if (instruments.ok) {
    append(pickerBody, createInstrumentPicker({
      instruments: instruments.value,
      onSelect: (instrument: InstrumentSummaryDto) => {
        selected = instrument;
        paintTicket();
      },
    }));
  } else {
    append(pickerBody, el("div", "vox-deferred", `${instruments.error.status}: ${instruments.error.body.message}`));
  }
  append(picker, pickerBody);

  return createWorkspaceGrid({
    workspaceId: "platform",
    layoutStore: new LayoutStore(sessionStorage),
    items: [
      { id: "connections", col: 0, row: 0, colSpan: 4, rowSpan: 5, element: connectionTable(snapshot) },
      { id: "instrument-picker", col: 4, row: 0, colSpan: 4, rowSpan: 5, element: picker },
      { id: "order-ticket", col: 8, row: 0, colSpan: 4, rowSpan: 5, element: ticketHost },
    ],
  });
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
    const button = el("button", "vox-btn vox-btn--ghost", density);
    button.type = "button";
    button.dataset.densityChoice = density;
    button.addEventListener("click", () => {
      root.dataset.density = density;
    });
    append(control, button);
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
