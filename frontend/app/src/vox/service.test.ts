import { describe, expect, it } from "vitest";
import type {
  ExecutionScope,
  MutationReceiptDto,
  PortfolioDto,
  ReconciliationDto,
  RuntimeHealthDto,
} from "@vox/api-client";
import { VoxApiError } from "@vox/api-client";
import { freezeAccountContext } from "../account/context";
import { AccountStore } from "../account/store";
import { VoxService } from "./service";

function scope(account_id: string): ExecutionScope {
  return {
    provider: "T_INVEST",
    environment: "SANDBOX",
    broker_connection_id: "connection:primary",
    account_id,
    trading_mode: "LIVE",
  };
}

function portfolio(account_id: string): PortfolioDto {
  return { account_id, balances: [] };
}

function reconciliation(runtime_epoch: number): ReconciliationDto {
  return {
    scope_key: "account:a",
    reconciliation_id: "recon-1",
    snapshot_observed_at_unix_ms: 0,
    completed_at_unix_ms: 0,
    runtime_epoch,
    accounts_complete: true,
    portfolio_complete: true,
    positions_complete: true,
    orders_complete: true,
    stops_complete: true,
    operations_complete: true,
    complete: true,
  };
}

function health(runtime_epoch: number, account_display: string): RuntimeHealthDto {
  return {
    state: "READY",
    reason_code: "RECONCILIATION_COMPLETE",
    reason: "ready",
    provider: "T_INVEST",
    environment: "SANDBOX",
    account_display,
    runtime_epoch,
    connected: true,
    unresolved_unknown_count: 0,
    open_order_count: 0,
    active_stop_count: 0,
    stream_states: [],
    persistence_healthy: true,
    execution_authorized: false,
    new_exposure_allowed: true,
  };
}

type Pending = {
  url: URL;
  resolve: (body: unknown, status?: number) => void;
};

/** Fetch that ignores abort so a late body can still arrive after switchTo. */
function deferredFetch(): { fetchImpl: typeof fetch; pending: Pending[] } {
  const pending: Pending[] = [];
  const fetchImpl: typeof fetch = (input) => {
    const url = new URL(String(input), "http://localhost");
    return new Promise<Response>((resolve) => {
      pending.push({
        url,
        resolve: (body, status = 200) => {
          resolve(
            new Response(JSON.stringify(body), {
              status,
              headers: { "content-type": "application/json" },
            }),
          );
        },
      });
    });
  };
  return { fetchImpl, pending };
}

function take(pending: Pending[], pathname: string, accountId?: string): Pending {
  const index = pending.findIndex((item) => {
    if (item.url.pathname !== pathname) return false;
    if (accountId !== undefined && item.url.searchParams.get("account_id") !== accountId) {
      return false;
    }
    return true;
  });
  if (index < 0) {
    throw new Error(`missing pending ${pathname} ${accountId ?? ""}`);
  }
  const [item] = pending.splice(index, 1);
  if (!item) {
    throw new Error(`missing pending ${pathname} ${accountId ?? ""}`);
  }
  return item;
}

describe("VoxService stale suppression", () => {
  it("discards account A after switch to B: A resolves later, store stays B", async () => {
    const store = new AccountStore();
    const { fetchImpl, pending } = deferredFetch();
    const service = new VoxService(store, { fetch: fetchImpl });
    const applied: PortfolioDto[] = [];

    store.switchTo(scope("account:a"));
    const fromA = service.portfolio().then((result) => {
      if (result.ok) applied.push(result.value);
      return result;
    });

    store.switchTo(scope("account:b"));
    const fromB = service.portfolio().then((result) => {
      if (result.ok) applied.push(result.value);
      return result;
    });

    take(pending, "/api/v1/portfolio", "account:a").resolve(portfolio("account:a"));
    take(pending, "/api/v1/portfolio", "account:b").resolve(portfolio("account:b"));

    const resultA = await fromA;
    const resultB = await fromB;

    expect(resultA).toEqual({ ok: false, stale: true });
    expect(resultB).toEqual({
      ok: true,
      value: portfolio("account:b"),
      generation: 2,
      context: freezeAccountContext(scope("account:b")),
    });
    expect(applied).toHaveLength(1);
    expect(applied[0]?.account_id).toBe("account:b");
    expect(store.current()?.account_id).toBe("account:b");
    expect(store.generation()).toBe(2);
  });

  it("rapid A→B→C: only C applies; A and B late responses are ignored", async () => {
    const store = new AccountStore();
    const { fetchImpl, pending } = deferredFetch();
    const service = new VoxService(store, { fetch: fetchImpl });
    const applied: string[] = [];

    store.switchTo(scope("account:a"));
    const fromA = service.portfolio().then((result) => {
      if (result.ok) applied.push(result.context.account_id);
      return result;
    });
    store.switchTo(scope("account:b"));
    const fromB = service.portfolio().then((result) => {
      if (result.ok) applied.push(result.context.account_id);
      return result;
    });
    store.switchTo(scope("account:c"));
    const fromC = service.portfolio().then((result) => {
      if (result.ok) applied.push(result.context.account_id);
      return result;
    });

    take(pending, "/api/v1/portfolio", "account:a").resolve(portfolio("account:a"));
    take(pending, "/api/v1/portfolio", "account:b").resolve(portfolio("account:b"));
    take(pending, "/api/v1/portfolio", "account:c").resolve(portfolio("account:c"));

    expect(await fromA).toEqual({ ok: false, stale: true });
    expect(await fromB).toEqual({ ok: false, stale: true });
    expect(await fromC).toMatchObject({
      ok: true,
      value: portfolio("account:c"),
      generation: 3,
    });
    expect(applied).toEqual(["account:c"]);
    expect(store.current()?.account_id).toBe("account:c");
  });

  it("discards a scoped snapshot from a previous runtime_epoch", async () => {
    const store = new AccountStore();
    const { fetchImpl, pending } = deferredFetch();
    const service = new VoxService(store, { fetch: fetchImpl });
    store.switchTo(scope("account:a"));

    const first = service.reconciliation();
    const second = service.reconciliation();
    take(pending, "/api/v1/reconciliation", "account:a").resolve(reconciliation(7));
    expect(await first).toMatchObject({ ok: true, value: { runtime_epoch: 7 } });
    expect(store.runtimeEpoch()).toBe(7);

    take(pending, "/api/v1/reconciliation", "account:a").resolve(reconciliation(4));
    expect(await second).toEqual({ ok: false, stale: true });
    expect(store.runtimeEpoch()).toBe(7);
  });

  it("does not abort or discard unscoped runtime() when the account switches", async () => {
    const store = new AccountStore();
    const { fetchImpl, pending } = deferredFetch();
    const service = new VoxService(store, { fetch: fetchImpl });

    store.switchTo(scope("account:a"));
    const pendingRuntime = service.runtime();
    store.switchTo(scope("account:b"));
    take(pending, "/api/v1/runtime").resolve(health(7, "A"));

    expect(await pendingRuntime).toEqual({ ok: true, value: health(7, "A") });
    expect(store.current()?.account_id).toBe("account:b");
  });

  it("does not surface a VoxApiError from a previous generation", async () => {
    const store = new AccountStore();
    const { fetchImpl, pending } = deferredFetch();
    const service = new VoxService(store, { fetch: fetchImpl });

    store.switchTo(scope("account:a"));
    const fromA = service.portfolio();
    store.switchTo(scope("account:b"));

    take(pending, "/api/v1/portfolio", "account:a").resolve(
      { code: "GONE", message: "old", correlation_id: "c", category: "NOT_FOUND", retryable: false },
      404,
    );

    const result = await fromA;
    expect(result).toEqual({ ok: false, stale: true });
    expect(result).not.toBeInstanceOf(VoxApiError);
    expect(store.current()?.account_id).toBe("account:b");
  });

  it("surfaces a current-generation 404 as VoxApiError", async () => {
    const store = new AccountStore();
    const { fetchImpl, pending } = deferredFetch();
    const service = new VoxService(store, { fetch: fetchImpl });
    store.switchTo(scope("account:a"));
    const pendingResult = service.portfolio();
    take(pending, "/api/v1/portfolio", "account:a").resolve(
      {
        code: "NOT_FOUND",
        message: "gone",
        correlation_id: "c",
        category: "NOT_FOUND",
        retryable: false,
      },
      404,
    );
    const result = await pendingResult;
    expect(result.ok).toBe(false);
    if (!result.ok && "error" in result) {
      expect(result.error).toBeInstanceOf(VoxApiError);
      expect(result.error.body.category).toBe("NOT_FOUND");
    } else {
      expect.fail("expected error result");
    }
  });

  it("treats abort after switchTo as stale", async () => {
    const store = new AccountStore();
    const fetchImpl: typeof fetch = (_input, init) => {
      return new Promise((_resolve, reject) => {
        const signal = init?.signal;
        if (signal?.aborted) {
          reject(new DOMException("Aborted", "AbortError"));
          return;
        }
        signal?.addEventListener("abort", () => {
          reject(new DOMException("Aborted", "AbortError"));
        });
      });
    };
    const service = new VoxService(store, { fetch: fetchImpl });
    store.switchTo(scope("account:a"));
    const pendingResult = service.portfolio();
    store.switchTo(scope("account:b"));
    expect(await pendingResult).toEqual({ ok: false, stale: true });
  });

  it("distinguishes no current context from stale", async () => {
    const store = new AccountStore();
    const { fetchImpl } = deferredFetch();
    const service = new VoxService(store, { fetch: fetchImpl });
    expect(await service.portfolio()).toEqual({ ok: false, noContext: true });
  });

  it("loads unscoped runtime without an account", async () => {
    const store = new AccountStore();
    const { fetchImpl, pending } = deferredFetch();
    const service = new VoxService(store, { fetch: fetchImpl });
    const pendingResult = service.runtime();
    take(pending, "/api/v1/runtime").resolve(health(1, "—"));
    expect(await pendingResult).toEqual({ ok: true, value: health(1, "—") });
  });

  it("rejects a provider baseUrl", () => {
    const store = new AccountStore();
    expect(
      () => new VoxService(store, { baseUrl: "https://invest-public-api.tinkoff.ru" }),
    ).toThrow();
  });

  it("discards a lying portfolio DTO on the current generation", async () => {
    const store = new AccountStore();
    const { fetchImpl, pending } = deferredFetch();
    const service = new VoxService(store, { fetch: fetchImpl });
    store.switchTo(scope("account:a"));
    const pendingResult = service.portfolio();
    take(pending, "/api/v1/portfolio", "account:a").resolve(portfolio("account:b"));
    expect(await pendingResult).toEqual({ ok: false, stale: true });
  });

  it("discards mutations whose nested scope belongs to another account", async () => {
    const store = new AccountStore();
    const { fetchImpl, pending } = deferredFetch();
    const service = new VoxService(store, { fetch: fetchImpl });
    store.switchTo(scope("account:a"));
    const pendingResult = service.mutations();
    const lying: MutationReceiptDto = {
      logical_request_id: "req-1",
      scope: scope("account:b"),
      kind: "POST_ORDER",
      state: "UNKNOWN_AFTER_DISPATCH",
      decision: "RECONCILE",
      correlation_id: "corr-1",
      runtime_epoch: 1,
      created_at_unix_ms: 0,
      updated_at_unix_ms: 0,
    };
    take(pending, "/api/v1/mutations", "account:a").resolve([lying]);
    expect(await pendingResult).toEqual({ ok: false, stale: true });
  });
});
