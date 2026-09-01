import { describe, expect, it } from "vitest";
import type { ExecutionScope, MutationReceiptDto } from "@vox/api-client";
import { AccountStore } from "../account/store";
import { CommandHandle, freezeExecutionTarget } from "./target";

function scope(account_id: string): ExecutionScope {
  return {
    provider: "T_INVEST",
    environment: "SANDBOX",
    broker_connection_id: "connection:primary",
    account_id,
    trading_mode: "LIVE",
  };
}

function receipt(account_id: string): MutationReceiptDto {
  return {
    logical_request_id: "req-1",
    scope: scope(account_id),
    kind: "POST_ORDER",
    state: "UNKNOWN_AFTER_DISPATCH",
    decision: "RECONCILE",
    correlation_id: "corr-1",
    runtime_epoch: 1,
    created_at_unix_ms: 0,
    updated_at_unix_ms: 0,
  };
}

describe("FrozenExecutionTarget", () => {
  it("survives a UI account switch: freeze under A, switch to B, handle.scope stays A", () => {
    const store = new AccountStore();
    store.switchTo(scope("account:a"));
    const handle = new CommandHandle(store.current()!, "req-1");
    expect(handle.scope.frozen).toBe(true);
    expect(handle.scope.account_id).toBe("account:a");

    store.switchTo(scope("account:b"));
    expect(store.current()?.account_id).toBe("account:b");
    expect(handle.scope.account_id).toBe("account:a");
    expect(handle.scope).toEqual(freezeExecutionTarget(scope("account:a")));
  });

  it("throws on mutation after freeze and does not retarget on bind", () => {
    const handle = new CommandHandle(scope("account:a"), "req-1");
    expect(() => {
      (handle.scope as { account_id: string }).account_id = "account:mutated";
    }).toThrow(TypeError);
    expect(() => {
      (handle as { receipt: MutationReceiptDto }).receipt = receipt("account:a");
    }).toThrow(TypeError);

    const bound = handle.bind(receipt("account:a"));
    expect(bound).not.toBe(handle);
    expect(bound.scope.account_id).toBe("account:a");
    expect(bound.receipt?.logical_request_id).toBe("req-1");
    expect(bound.receipt?.state).toBe("UNKNOWN_AFTER_DISPATCH");
    expect(handle.receipt).toBeUndefined();
    expect(() => {
      (bound.receipt!.scope as { account_id: string }).account_id = "account:mutated";
    }).toThrow(TypeError);

    expect(() => handle.bind(receipt("account:b"))).toThrow(
      /receipt scope does not match frozen execution target/,
    );
    expect(handle.scope.account_id).toBe("account:a");
    expect(handle.logicalRequestId).toBe("req-1");
  });

  it("rejects receipt with another logical request identity", () => {
    const handle = new CommandHandle(scope("account:a"), "req-1");
    expect(() => handle.bind({ ...receipt("account:a"), logical_request_id: "req-2" })).toThrow(
      /receipt identity does not match frozen logical request/,
    );
  });

  it("freezes human broker-account metadata through receipt binding", () => {
    const handle = new CommandHandle(scope("account:a"), "req-1", undefined, {
      providerAccountId: "provider-4417",
      accountDisplay: "Capital",
      connectionLabel: "Primary",
    });
    const bound = handle.bind(receipt("account:a"));
    expect(bound.targetDisplay).toEqual({
      providerAccountId: "provider-4417",
      accountDisplay: "Capital",
      connectionLabel: "Primary",
    });
    expect(() => {
      (bound.targetDisplay as { accountDisplay: string }).accountDisplay = "Other";
    }).toThrow(TypeError);
  });
});
