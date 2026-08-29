import type { AccountStore } from "../account";
import { append, clear, el } from "./dom";
import { createDeferred } from "./deferred";
import { createEnvBadge, providerLabel } from "./env-badge";
import { readAccountCurrent, subscribeStore } from "./stores";

export type AccountContextIndicatorOptions = {
  store: AccountStore;
};

export function createAccountContextIndicator(
  options: AccountContextIndicatorOptions,
): HTMLElement {
  const root = el("div", "vox-row vox-gap-2");
  const render = (): void => {
    clear(root);
    const current = readAccountCurrent(options.store);
    if (current === null) {
      append(
        root,
        createDeferred({
          title: "Счёт не выбран",
          owner: "#17",
          body: "Нет подключений брокера. Подпись счёта появится после привязки.",
        }),
      );
      return;
    }
    append(
      root,
      el("span", "vox-text--dense", providerLabel(current.provider)),
      createEnvBadge(current.environment),
    );
  };
  render();
  subscribeStore(options.store, render);
  return root;
}
