import { describe, expect, it } from "vitest";
import type { ExecutionScope } from "@vox/api-client";
import {
  accountContextKey,
  freezeAccountContext,
  sameAccountContext,
} from "./context";

function scope(account_id: string, broker_connection_id = "connection:primary"): ExecutionScope {
  return {
    provider: "T_INVEST",
    environment: "SANDBOX",
    broker_connection_id,
    account_id,
    trading_mode: "LIVE",
  };
}

describe("AccountContext", () => {
  it("freezes only the generated ExecutionScope identity fields", () => {
    const raw = {
      ...scope("account:a"),
      broker_account_id: "broker-meta-should-not-be-identity",
      display_name: "Основной",
    };
    const ctx = freezeAccountContext(raw as ExecutionScope);
    expect(ctx).toEqual(scope("account:a"));
    expect("broker_account_id" in ctx).toBe(false);
    expect("display_name" in ctx).toBe(false);
    expect(() => {
      (ctx as { account_id: string }).account_id = "account:mutated";
    }).toThrow(TypeError);
  });

  it("does not put broker_account_id into the context key", () => {
    const ctx = freezeAccountContext(scope("account:shared"));
    const key = accountContextKey(ctx);
    expect(key).toContain("account:shared");
    expect(key).toContain("connection:primary");
    expect(key).not.toContain("broker-meta");
    expect(key).not.toContain("broker_account_id");
  });

  it("treats connection identity as part of the key so two connections cannot collide", () => {
    const left = freezeAccountContext(scope("account:shared", "connection:one"));
    const right = freezeAccountContext(scope("account:shared", "connection:two"));
    expect(sameAccountContext(left, right)).toBe(false);
    expect(accountContextKey(left)).not.toBe(accountContextKey(right));
    expect(sameAccountContext(left, freezeAccountContext(left))).toBe(true);
  });
});
