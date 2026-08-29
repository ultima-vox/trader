import {
  VoxApiError,
  VoxClient,
  type BrokerAccountDto,
  type CapabilitySet,
  type ExecutionScope,
  type MutationReceiptDto,
  type OperationsPageDto,
  type OrderDto,
  type PortfolioDto,
  type PositionDto,
  type ReconciliationDto,
  type RuntimeHealthDto,
  type StopOrderDto,
  type SystemHealthDto,
  type VoxClientOptions,
} from "@vox/api-client";
import {
  accountContextKey,
  type AccountContext,
} from "../account/context";
import type { AccountStore } from "../account/store";
import { assertSafeBaseUrl } from "../security/provider";

export type ScopedResult<T> =
  | { ok: true; value: T; generation: number; context: AccountContext }
  | { ok: false; stale: true }
  | { ok: false; noContext: true }
  | { ok: false; error: VoxApiError };

export type UnscopedResult<T> =
  | { ok: true; value: T }
  | { ok: false; error: VoxApiError };

export type VoxServiceOptions = {
  baseUrl?: string;
  fetch?: typeof fetch;
};

type ScopeQuery = Pick<
  ExecutionScope,
  "account_id" | "broker_connection_id" | "environment" | "provider"
>;

function scopeQuery(ctx: AccountContext): ScopeQuery {
  return {
    account_id: ctx.account_id,
    broker_connection_id: ctx.broker_connection_id,
    environment: ctx.environment,
    provider: ctx.provider,
  };
}

function isAbortError(error: unknown): boolean {
  return error instanceof Error && error.name === "AbortError";
}

function readRuntimeEpoch(value: unknown): number | undefined {
  if (typeof value !== "object" || value === null) return undefined;
  if (!("runtime_epoch" in value)) return undefined;
  const epoch = (value as { runtime_epoch: unknown }).runtime_epoch;
  if (typeof epoch !== "number" || !Number.isSafeInteger(epoch) || epoch < 0) {
    return undefined;
  }
  return epoch;
}

function isExecutionScope(value: unknown): value is ExecutionScope {
  if (typeof value !== "object" || value === null) return false;
  const record = value as Record<string, unknown>;
  return (
    typeof record.provider === "string" &&
    typeof record.environment === "string" &&
    typeof record.broker_connection_id === "string" &&
    typeof record.account_id === "string" &&
    typeof record.trading_mode === "string"
  );
}

function scopeMismatch(
  value: unknown,
  ctx: AccountContext,
  checkAccountId: boolean,
): boolean {
  if (Array.isArray(value)) {
    return value.some((item) => scopeMismatch(item, ctx, checkAccountId));
  }
  if (typeof value !== "object" || value === null) return false;
  const record = value as Record<string, unknown>;
  if (checkAccountId && typeof record.account_id === "string" && record.account_id !== ctx.account_id) {
    return true;
  }
  if (
    typeof record.broker_connection_id === "string" &&
    record.broker_connection_id !== ctx.broker_connection_id
  ) {
    return true;
  }
  if (typeof record.environment === "string" && record.environment !== ctx.environment) {
    return true;
  }
  if (isExecutionScope(record.scope) && accountContextKey(record.scope) !== accountContextKey(ctx)) {
    return true;
  }
  if ("items" in record) {
    return scopeMismatch(record.items, ctx, checkAccountId);
  }
  return false;
}

function abortedUnscoped(): VoxApiError {
  return new VoxApiError(499, {
    code: "ABORTED",
    message: "запрос прерван",
    correlation_id: "",
    category: "TRANSIENT",
    retryable: true,
  });
}

export class VoxService {
  private readonly client: VoxClient;
  private readonly baseFetch: typeof fetch;
  private readonly baseUrl: string | undefined;

  constructor(
    private readonly store: AccountStore,
    options: VoxServiceOptions = {},
  ) {
    if (options.baseUrl !== undefined) {
      assertSafeBaseUrl(options.baseUrl);
    }
    this.baseFetch = options.fetch ?? fetch.bind(globalThis);
    this.baseUrl = options.baseUrl;
    const clientOptions: VoxClientOptions = { fetch: this.baseFetch };
    if (this.baseUrl !== undefined) clientOptions.baseUrl = this.baseUrl;
    this.client = new VoxClient(clientOptions);
  }

  private scopedClient(signal: AbortSignal): VoxClient {
    const clientOptions: VoxClientOptions = {
      fetch: (input, init) => this.baseFetch(input, { ...init, signal }),
    };
    if (this.baseUrl !== undefined) clientOptions.baseUrl = this.baseUrl;
    return new VoxClient(clientOptions);
  }

  async runScoped<T>(
    fn: (ctx: AccountContext, client: VoxClient) => Promise<T>,
    checkAccountId = true,
  ): Promise<ScopedResult<T>> {
    const ctx = this.store.current();
    if (ctx === null) {
      return { ok: false, noContext: true };
    }
    const generation = this.store.generation();
    const key = accountContextKey(ctx);
    const signal = this.store.signal();
    if (signal.aborted) {
      return { ok: false, stale: true };
    }

    try {
      const value = await fn(ctx, this.scopedClient(signal));
      if (this.isStale(generation, key, signal, value) || scopeMismatch(value, ctx, checkAccountId)) {
        return { ok: false, stale: true };
      }
      const epoch = readRuntimeEpoch(value);
      if (epoch !== undefined) {
        this.store.observeRuntimeEpoch(epoch, generation, key);
      }
      return { ok: true, value, generation, context: ctx };
    } catch (error) {
      if (this.isStale(generation, key, signal) || isAbortError(error)) {
        return { ok: false, stale: true };
      }
      if (error instanceof VoxApiError) {
        return { ok: false, error };
      }
      throw error;
    }
  }

  portfolio(): Promise<ScopedResult<PortfolioDto>> {
    return this.runScoped((ctx, client) => client.portfolio(scopeQuery(ctx)));
  }

  positions(): Promise<ScopedResult<Array<PositionDto>>> {
    return this.runScoped((ctx, client) => client.positions(scopeQuery(ctx)));
  }

  orders(): Promise<ScopedResult<Array<OrderDto>>> {
    return this.runScoped((ctx, client) => client.orders(scopeQuery(ctx)));
  }

  operations(extra?: {
    cursor?: string | null;
    limit?: number | null;
  }): Promise<ScopedResult<OperationsPageDto>> {
    return this.runScoped((ctx, client) => {
      const query: Parameters<VoxClient["operations"]>[0] = scopeQuery(ctx);
      if (extra?.cursor !== undefined) query.cursor = extra.cursor;
      if (extra?.limit !== undefined) query.limit = extra.limit;
      return client.operations(query);
    });
  }

  mutations(): Promise<ScopedResult<Array<MutationReceiptDto>>> {
    return this.runScoped((ctx, client) => client.mutations(scopeQuery(ctx)));
  }

  accounts(): Promise<ScopedResult<Array<BrokerAccountDto>>> {
    return this.runScoped((ctx, client) => client.accounts(scopeQuery(ctx)), false);
  }

  stopOrders(): Promise<ScopedResult<Array<StopOrderDto>>> {
    return this.runScoped((ctx, client) => client.stopOrders(scopeQuery(ctx)));
  }

  reconciliation(): Promise<ScopedResult<ReconciliationDto>> {
    return this.runScoped((ctx, client) => client.reconciliation(scopeQuery(ctx)));
  }

  capabilities(): Promise<ScopedResult<CapabilitySet>> {
    return this.runScoped((ctx, client) => client.capabilities({ account_id: ctx.account_id }));
  }

  runtime(): Promise<UnscopedResult<RuntimeHealthDto>> {
    return this.runUnscoped(() => this.client.runtime());
  }

  runtimeScopes(): Promise<UnscopedResult<Array<ExecutionScope>>> {
    return this.runUnscoped(() => this.client.runtimeScopes());
  }

  systemHealth(): Promise<UnscopedResult<SystemHealthDto>> {
    return this.runUnscoped(() => this.client.systemHealth());
  }

  private async runUnscoped<T>(fn: () => Promise<T>): Promise<UnscopedResult<T>> {
    try {
      return { ok: true, value: await fn() };
    } catch (error) {
      if (isAbortError(error)) {
        return { ok: false, error: abortedUnscoped() };
      }
      if (error instanceof VoxApiError) {
        return { ok: false, error };
      }
      throw error;
    }
  }

  private isStale(
    generation: number,
    key: string,
    signal: AbortSignal,
    value?: unknown,
  ): boolean {
    if (signal.aborted) return true;
    if (this.store.generation() !== generation) return true;
    const current = this.store.current();
    if (current === null || accountContextKey(current) !== key) return true;
    const epoch = readRuntimeEpoch(value);
    if (epoch !== undefined && epoch < this.store.runtimeEpoch()) return true;
    return false;
  }
}
