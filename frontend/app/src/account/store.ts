import type { AccountContext } from "./context";
import { accountContextKey, freezeAccountContext } from "./context";

export type AccountStoreListener = () => void;

export class AccountStore {
  private context: AccountContext | null = null;
  private gen = 0;
  private epoch = 0;
  private controller = new AbortController();
  private readonly listeners = new Set<AccountStoreListener>();

  current(): AccountContext | null {
    return this.context;
  }

  generation(): number {
    return this.gen;
  }

  runtimeEpoch(): number {
    return this.epoch;
  }

  signal(): AbortSignal {
    return this.controller.signal;
  }

  switchTo(next: AccountContext): void {
    const frozen = freezeAccountContext(next);
    const previous = this.controller;
    this.controller = new AbortController();
    this.context = frozen;
    this.gen += 1;
    this.epoch = 0;
    previous.abort();
    this.emit();
  }

  observeRuntimeEpoch(epoch: number, generation: number, key: string): boolean {
    if (!Number.isSafeInteger(epoch) || epoch < 0) return false;
    if (this.gen !== generation) return false;
    if (this.context === null || accountContextKey(this.context) !== key) return false;
    if (epoch < this.epoch) return false;
    this.epoch = epoch;
    return true;
  }

  subscribe(listener: AccountStoreListener): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  private emit(): void {
    for (const listener of [...this.listeners]) {
      listener();
    }
  }
}
