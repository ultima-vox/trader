import {
  freezeAccountContext,
  sameAccountContext,
  type AccountContext,
  type AccountStore,
} from "../account";
import { append, clear, el, setClass } from "./dom";
import { createDeferred } from "./deferred";
import { createEnvBadge, providerLabel } from "./env-badge";
import { accountLabelOf, readAccountCurrent, subscribeStore } from "./stores";

export type AccountSelectorOptions = {
  store: AccountStore;
  /** Empty until #17. Do not invent rows. */
  scopes?: readonly AccountContext[];
};

export function createAccountSelector(options: AccountSelectorOptions): HTMLElement {
  const store = options.store;
  const scopes = options.scopes ?? [];
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
      el("span", "vox-account__label", accountLabelOf(current)),
      createEnvBadge(current.environment),
    );
    trigger.setAttribute(
      "aria-label",
      `${providerLabel(current.provider)} ${accountLabelOf(current)} ${current.environment}`,
    );
  };

  const paintPopover = (): void => {
    clear(popover);
    const current = readAccountCurrent(store);
    if (scopes.length === 0) {
      append(
        popover,
        createDeferred({
          title: "Счета не подключены",
          owner: "#17",
          body: "Список пуст, пока нет подключений брокера. Счета не выдумываются.",
        }),
      );
      return;
    }
    const list = el("div");
    list.setAttribute("role", "listbox");
    for (const scope of scopes) {
      append(
        list,
        rowFor(scope, current, () => {
          store.switchTo(freezeAccountContext(scope));
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
  scope: AccountContext,
  current: AccountContext | null,
  onSelect: () => void,
): HTMLElement {
  const row = el("div", "vox-account-row");
  row.setAttribute("role", "option");
  if (current !== null && sameAccountContext(scope, current)) row.classList.add("is-selected");
  const name = el("span", "vox-account-row__name", accountLabelOf(scope));
  append(
    name,
    el(
      "span",
      "vox-account-row__meta",
      `${providerLabel(scope.provider)} · ${maskId(scope.account_id)}`,
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
