import { describe, expect, it } from "vitest";
import type { ExecutionScope } from "@vox/api-client";
import { accountContextKey, freezeAccountContext } from "./context";
import { AccountStore } from "./store";

function scope(account_id: string): ExecutionScope {
  return {
    provider: "T_INVEST",
    environment: "SANDBOX",
    broker_connection_id: "connection:primary",
    account_id,
    trading_mode: "LIVE",
  };
}

describe("AccountStore", () => {
  it("switchTo is atomic: context, generation, abort and epoch change together", () => {
    const store = new AccountStore();
    const seen: Array<{
      account: string | null;
      generation: number;
      epoch: number;
      aborted: boolean;
    }> = [];
    store.subscribe(() => {
      seen.push({
        account: store.current()?.account_id ?? null,
        generation: store.generation(),
        epoch: store.runtimeEpoch(),
        aborted: store.signal().aborted,
      });
    });

    const firstSignal = store.signal();
    store.switchTo(scope("account:a"));
    expect(
      store.observeRuntimeEpoch(4, store.generation(), accountContextKey(store.current()!)),
    ).toBe(true);
    expect(store.runtimeEpoch()).toBe(4);

    const afterA = store.signal();
    store.switchTo(scope("account:b"));

    expect(store.current()).toEqual(freezeAccountContext(scope("account:b")));
    expect(store.generation()).toBe(2);
    expect(store.runtimeEpoch()).toBe(0);
    expect(firstSignal.aborted).toBe(true);
    expect(afterA.aborted).toBe(true);
    expect(store.signal().aborted).toBe(false);
    expect(seen).toEqual([
      { account: "account:a", generation: 1, epoch: 0, aborted: false },
      { account: "account:b", generation: 2, epoch: 0, aborted: false },
    ]);
  });

  it("discards a previous runtime_epoch for the current context", () => {
    const store = new AccountStore();
    store.switchTo(scope("account:a"));
    const generation = store.generation();
    const key = accountContextKey(store.current()!);
    expect(store.observeRuntimeEpoch(3, generation, key)).toBe(true);
    expect(store.observeRuntimeEpoch(2, generation, key)).toBe(false);
    expect(store.runtimeEpoch()).toBe(3);
    expect(store.observeRuntimeEpoch(3, generation, key)).toBe(true);
    expect(store.runtimeEpoch()).toBe(3);
  });

  it("ignores an observe from a previous generation after switchTo", () => {
    const store = new AccountStore();
    store.switchTo(scope("account:a"));
    const generationA = store.generation();
    const keyA = accountContextKey(store.current()!);
    store.switchTo(scope("account:b"));
    expect(store.observeRuntimeEpoch(9, generationA, keyA)).toBe(false);
    expect(store.runtimeEpoch()).toBe(0);
    expect(store.observeRuntimeEpoch(Number.NaN, store.generation(), accountContextKey(store.current()!))).toBe(
      false,
    );
  });

  it("unsubscribe stops further notifications", () => {
    const store = new AccountStore();
    let calls = 0;
    const stop = store.subscribe(() => {
      calls += 1;
    });
    store.switchTo(scope("account:a"));
    stop();
    store.switchTo(scope("account:b"));
    expect(calls).toBe(1);
    expect(accountContextKey(store.current()!)).toContain("account:b");
  });
});
