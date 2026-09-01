import { sameAccountContext, type AccountContext, type AccountStore } from "../account";
import type { PlatformAccount } from "../platform";
import { append, clear, el, setClass } from "./dom";
import { createDeferred } from "./deferred";
import { createEnvBadge, providerLabel } from "./env-badge";
import { accountLabelOf, readAccountCurrent, subscribeStore } from "./stores";

export type AccountSelectorOptions = {
  store: AccountStore;
  accounts?: readonly PlatformAccount[];
};

export function createAccountSelector(options: AccountSelectorOptions): HTMLElement {
  const store = options.store;
  const accounts = options.accounts ?? [];
  const root = el("div");
  root.style.position = "relative";

  const trigger = el("button", "vox-account");
  trigger.type = "button";
  trigger.setAttribute("aria-haspopup", "listbox");
  trigger.setAttribute("aria-expanded", "false");
  trigger.title = "Счёт исполнения рабочего пространства";

  const popover = el("div", "vox-popover");
  popover.hidden = true;
  popover.style.position = "absolute";
  popover.style.top = "100%";
  popover.style.left = "0";
  popover.style.zIndex = "4";
  popover.style.minWidth = "280px";
  popover.style.marginTop = "4px";

  const paintTrigger = (): void => {
    clear(trigger);
    const current = readAccountCurrent(store);
    const selected = current === null
      ? undefined
      : accounts.find((item) => sameAccountContext(item.scope, current));
    setClass(trigger, "is-unknown", current === null);
    setClass(trigger, "is-live", current?.environment === "PRODUCTION");
    if (current === null) {
      append(trigger, el("span", "vox-account__label", "Нет счёта"));
      trigger.setAttribute("aria-label", "Нет счёта");
      return;
    }
    append(
      trigger,
      el("span", "vox-account__broker", providerLabel(current.provider)),
      el("span", "vox-account__sep", "/"),
      el("span", "vox-account__label", selected?.accountDisplay ?? accountLabelOf(current)),
      createEnvBadge(current.environment),
    );
    trigger.setAttribute(
      "aria-label",
      `${providerLabel(current.provider)} ${selected?.connectionLabel ?? ""} ${selected?.accountDisplay ?? accountLabelOf(current)} ${current.environment}`,
    );
  };

  const paintPopover = (): void => {
    clear(popover);
    const current = readAccountCurrent(store);
    if (accounts.length === 0) {
      append(
        popover,
        createDeferred({
          title: "Счета не подключены",
          body: "Vox не вернул активных привязок счетов. Строки не симулируются.",
        }),
      );
      return;
    }
    const list = el("div");
    list.setAttribute("role", "listbox");
    for (const account of accounts) {
      append(
        list,
        rowFor(account, current, () => {
          store.switchTo(account.scope);
          close();
        }),
      );
    }
    append(popover, list);
  };

  const open = (): void => {
    paintPopover();
    popover.hidden = false;
    trigger.setAttribute("aria-expanded", "true");
  };

  const close = (): void => {
    popover.hidden = true;
    trigger.setAttribute("aria-expanded", "false");
  };

  trigger.addEventListener("click", (event) => {
    event.stopPropagation();
    if (popover.hidden) open();
    else close();
  });

  document.addEventListener("click", (event) => {
    if (!root.contains(event.target as Node)) close();
  });

  paintTrigger();
  subscribeStore(store, paintTrigger);
  append(root, trigger, popover);
  return root;
}

function rowFor(
  account: PlatformAccount,
  current: AccountContext | null,
  onSelect: () => void,
): HTMLElement {
  const row = el("div", "vox-account-row");
  row.setAttribute("role", "option");
  const scope = account.scope;
  if (current !== null && sameAccountContext(scope, current)) row.classList.add("is-selected");
  const name = el("span", "vox-account-row__name", account.accountDisplay);
  const authorization = account.executionAuthorization?.mode ?? "DISABLED";
  const capabilities = account.connectionCapabilities.join(", ") || "NO_CAPABILITIES";
  append(
    name,
    el(
      "span",
      "vox-account-row__meta",
      `${providerLabel(scope.provider)} · ${account.connectionLabel} · ${maskId(account.providerAccountId)} · ${account.connectionHealth.state} · ${authorization} · ${capabilities}`,
    ),
  );
  append(row, name, createEnvBadge(scope.environment));
  row.addEventListener("click", (event) => {
    event.stopPropagation();
    onSelect();
  });
  return row;
}

function maskId(id: string): string {
  if (id.length <= 4) return id;
  return `****${id.slice(-4)}`;
}
