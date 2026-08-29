import type { AccountContext, AccountStore } from "../account";

export function readAccountCurrent(store: AccountStore): AccountContext | null {
  return store.current();
}

export function subscribeStore(store: AccountStore, listener: () => void): () => void {
  return store.subscribe(listener);
}

export function accountLabelOf(context: AccountContext): string {
  return context.account_id;
}
