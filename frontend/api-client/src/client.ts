// Generated from docs/api/openapi.json by tools/api-client/generate.py.
// Do not edit: run `python tools/api-client/generate.py` after changing the Rust contracts.

import type * as T from "./types";

/** Everything this client can fail with, as the server described it. */
export class VoxApiError extends Error {
  constructor(readonly status: number, readonly body: T.ApiError) {
    super(body.message);
    this.name = "VoxApiError";
  }
}

export interface VoxClientOptions {
  /** Base URL of the Vox API. Same-origin by default. */
  baseUrl?: string;
  /** Passed through to fetch, for credentials and abort signals. */
  fetch?: typeof fetch;
}

/** The generated transport client. Wrap it in a service; do not call fetch directly. */
export class VoxClient {
  private readonly baseUrl: string;
  private readonly doFetch: typeof fetch;

  constructor(options: VoxClientOptions = {}) {
    this.baseUrl = options.baseUrl ?? "";
    this.doFetch = options.fetch ?? fetch;
  }

  private async request<R>(method: string, path: string, query?: Record<string, unknown>, body?: unknown): Promise<R> {
    const url = new URL(this.baseUrl + path, this.baseUrl || "http://localhost");
    for (const [key, value] of Object.entries(query ?? {})) {
      if (value !== undefined && value !== null) url.searchParams.set(key, String(value));
    }
    const init: RequestInit = { method };
    if (body !== undefined) {
      init.headers = { "content-type": "application/json" };
      init.body = JSON.stringify(body);
    }
    const target = this.baseUrl ? url.toString() : url.pathname + url.search;
    const response = await this.doFetch(target, init);
    const payload = response.status === 204 ? undefined : await response.json();
    if (!response.ok) throw new VoxApiError(response.status, payload as T.ApiError);
    return payload as R;
  }

  /** Accounts discovered through the connection. */
  accounts(query: { account_id: string; broker_connection_id: string; environment: T.BrokerEnvironment; provider: T.ProviderDto }): Promise<Array<T.BrokerAccountDto>> {
    return this.request("GET", "/api/v1/accounts", query);
  }

  /** What this deployment can actually do. */
  capabilities(query: { account_id?: string }): Promise<T.CapabilitySet> {
    return this.request("GET", "/api/v1/capabilities", query);
  }

  /** Cancel a regular order. */
  postCommandsCancelOrder(body: T.CancelOrderRequest): Promise<T.MutationReceiptDto> {
    return this.request("POST", "/api/v1/commands/cancel-order", undefined, body);
  }

  /** Submit a regular order. The scope in the body is the frozen target of the command. */
  postCommandsOrder(body: T.SubmitOrderRequest): Promise<T.MutationReceiptDto> {
    return this.request("POST", "/api/v1/commands/order", undefined, body);
  }

  /** Candles for one interval and window. */
  marketCandles(query: { from_unix_ms: number; instrument_uid: string; interval: T.CandleIntervalDto; provider: T.ProviderDto; to_unix_ms: number }): Promise<T.CandlesDto> {
    return this.request("GET", "/api/v1/market/candles", query);
  }

  /** Search the instrument catalogue. */
  marketInstruments(query: { limit?: number | null; provider: T.ProviderDto; query: string }): Promise<Array<T.InstrumentSummaryDto>> {
    return this.request("GET", "/api/v1/market/instruments", query);
  }

  /** A book snapshot. */
  marketOrderBook(query: { depth?: number | null; instrument_uid: string; provider: T.ProviderDto }): Promise<T.OrderBookDto> {
    return this.request("GET", "/api/v1/market/order-book", query);
  }

  /** Last price and top of book, with the age of the record. */
  marketQuote(query: { instrument_uid: string; provider: T.ProviderDto }): Promise<T.QuoteDto> {
    return this.request("GET", "/api/v1/market/quote", query);
  }

  /** Venue session state for one instrument. */
  marketSession(query: { instrument_uid: string; provider: T.ProviderDto }): Promise<T.SessionDto> {
    return this.request("GET", "/api/v1/market/session", query);
  }

  /** The public tape. */
  marketTrades(query: { instrument_uid: string; limit?: number | null; provider: T.ProviderDto }): Promise<Array<T.TradeTickDto>> {
    return this.request("GET", "/api/v1/market/trades", query);
  }

  /** Operations history, paged by cursor. */
  operations(query: { account_id: string; broker_connection_id: string; cursor?: string | null; environment: T.BrokerEnvironment; limit?: number | null; provider: T.ProviderDto }): Promise<T.OperationsPageDto> {
    return this.request("GET", "/api/v1/operations", query);
  }

  /** Active orders. */
  orders(query: { account_id: string; broker_connection_id: string; environment: T.BrokerEnvironment; provider: T.ProviderDto }): Promise<Array<T.OrderDto>> {
    return this.request("GET", "/api/v1/orders", query);
  }

  /** Currency balances as the broker reports them. */
  portfolio(query: { account_id: string; broker_connection_id: string; environment: T.BrokerEnvironment; provider: T.ProviderDto }): Promise<T.PortfolioDto> {
    return this.request("GET", "/api/v1/portfolio", query);
  }

  /** Positions and their quantities. */
  positions(query: { account_id: string; broker_connection_id: string; environment: T.BrokerEnvironment; provider: T.ProviderDto }): Promise<Array<T.PositionDto>> {
    return this.request("GET", "/api/v1/positions", query);
  }

  /** How complete the last reconciliation was. */
  reconciliation(query: { account_id: string; broker_connection_id: string; environment: T.BrokerEnvironment; provider: T.ProviderDto }): Promise<T.ReconciliationDto> {
    return this.request("GET", "/api/v1/reconciliation", query);
  }

  /** Runtime state, readiness and stream health. */
  runtime(): Promise<T.RuntimeHealthDto> {
    return this.request("GET", "/api/v1/runtime", undefined);
  }

  /** Stop orders. */
  stopOrders(query: { account_id: string; broker_connection_id: string; environment: T.BrokerEnvironment; provider: T.ProviderDto }): Promise<Array<T.StopOrderDto>> {
    return this.request("GET", "/api/v1/stop-orders", query);
  }

  /** Liveness of the API process itself. */
  systemHealth(): Promise<T.SystemHealthDto> {
    return this.request("GET", "/api/v1/system/health", undefined);
  }

}
