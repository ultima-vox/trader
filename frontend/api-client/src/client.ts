// Generated from docs/api/openapi.json by tools/api-client/generate.py.
// Do not edit: run `python tools/api-client/generate.py` after changing the Rust contracts.

import type * as T from "./types";

/** Everything this client can fail with, as the server described it. */
export class VoxApiError extends Error {
  readonly status: number;
  readonly body: T.ApiError;
  constructor(status: number, body: T.ApiError) {
    super(body.message);
    this.status = status;
    this.body = body;
    this.name = "VoxApiError";
  }
}

export interface VoxClientOptions {
  /** Base URL of the Vox API. Same-origin by default. */
  baseUrl?: string;
  /** Passed through to fetch, for credentials and abort signals. */
  fetch?: typeof fetch;
  /** Cookie policy. Same-origin sends server-issued HttpOnly session cookie. */
  credentials?: RequestCredentials;
  /** Restored CSRF state from an earlier session bootstrap response. */
  csrfToken?: string;
}

/** The generated transport client. Wrap it in a service; do not call fetch directly. */
export class VoxClient {
  private readonly baseUrl: string;
  private readonly doFetch: typeof fetch;
  private readonly credentials: RequestCredentials;
  private csrfToken: string | undefined;

  constructor(options: VoxClientOptions = {}) {
    this.baseUrl = options.baseUrl ?? "";
    this.doFetch = options.fetch ?? fetch;
    this.credentials = options.credentials ?? "same-origin";
    this.csrfToken = options.csrfToken;
  }

  private async request<R>(method: string, path: string, query?: Record<string, unknown>, body?: unknown): Promise<R> {
    let resolvedPath = path;
    const remainingQuery: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(query ?? {})) {
      if (value === undefined || value === null) continue;
      const marker = `{${key}}`;
      if (resolvedPath.includes(marker)) resolvedPath = resolvedPath.replaceAll(marker, encodeURIComponent(String(value)));
      else remainingQuery[key] = value;
    }
    const url = new URL(this.baseUrl + resolvedPath, this.baseUrl || "http://localhost");
    for (const [key, value] of Object.entries(remainingQuery)) url.searchParams.set(key, String(value));
    const headers: Record<string, string> = {};
    if (!["GET", "HEAD", "OPTIONS"].includes(method) && this.csrfToken) headers["x-vox-csrf"] = this.csrfToken;
    const init: RequestInit = { method, credentials: this.credentials, headers };
    if (body !== undefined) {
      headers["content-type"] = "application/json";
      init.body = JSON.stringify(body);
    }
    const target = this.baseUrl ? url.toString() : url.pathname + url.search;
    const response = await this.doFetch(target, init);
    const payload = response.status === 204 ? undefined : await response.json();
    if (!response.ok) throw new VoxApiError(response.status, payload as T.ApiError);
    if (path === "/api/v1/auth/session") this.csrfToken = (payload as T.AuthSessionDto).csrf_token;
    return payload as R;
  }

  /** Accounts discovered through the connection. */
  accounts(query: { account_id: string; broker_connection_id: string; environment: T.BrokerEnvironment; provider: T.ProviderDto }): Promise<Array<T.BrokerAccountDto>> {
    return this.request("GET", "/api/v1/accounts", query);
  }

  /** Exchanges trusted bootstrap credential for browser session cookie and CSRF state. */
  postAuthSession(body: T.CreateSessionRequest): Promise<T.AuthSessionDto> {
    return this.request("POST", "/api/v1/auth/session", undefined, body);
  }

  deleteBrokerBindingsBinding_id(query: { binding_id: string }): Promise<void> {
    return this.request("DELETE", "/api/v1/broker-bindings/{binding_id}", query);
  }

  brokerConnections(): Promise<Array<T.BrokerConnectionMetadataDto>> {
    return this.request("GET", "/api/v1/broker-connections", undefined);
  }

  postBrokerConnections(body: T.CreateBrokerConnectionRequest): Promise<T.BrokerConnectionMetadataDto> {
    return this.request("POST", "/api/v1/broker-connections", undefined, body);
  }

  deleteBrokerConnectionsConnection_id(query: { connection_id: string }): Promise<void> {
    return this.request("DELETE", "/api/v1/broker-connections/{connection_id}", query);
  }

  brokerConnectionsConnection_id(query: { connection_id: string }): Promise<T.ConnectionDetailsDto> {
    return this.request("GET", "/api/v1/broker-connections/{connection_id}", query);
  }

  brokerConnectionsConnection_idAccounts(query: { connection_id: string }): Promise<Array<T.DiscoveredBrokerAccountDto>> {
    return this.request("GET", "/api/v1/broker-connections/{connection_id}/accounts", query);
  }

  brokerConnectionsConnection_idBindings(query: { connection_id: string }): Promise<Array<T.BrokerAccountBindingDto>> {
    return this.request("GET", "/api/v1/broker-connections/{connection_id}/bindings", query);
  }

  postBrokerConnectionsConnection_idBindings(query: { connection_id: string }, body: T.BindBrokerAccountRequest): Promise<T.BrokerAccountBindingDto> {
    return this.request("POST", "/api/v1/broker-connections/{connection_id}/bindings", query, body);
  }

  putBrokerConnectionsConnection_idCredential(query: { connection_id: string }, body: T.RotateCredentialRequest): Promise<T.CredentialRotationResultDto> {
    return this.request("PUT", "/api/v1/broker-connections/{connection_id}/credential", query, body);
  }

  postBrokerConnectionsConnection_idDisable(query: { connection_id: string }): Promise<T.BrokerConnectionMetadataDto> {
    return this.request("POST", "/api/v1/broker-connections/{connection_id}/disable", query);
  }

  putBrokerConnectionsConnection_idExecutionAuthorization(query: { connection_id: string }, body: T.ChangeExecutionAuthorizationRequest): Promise<T.ExecutionAuthorizationDto> {
    return this.request("PUT", "/api/v1/broker-connections/{connection_id}/execution-authorization", query, body);
  }

  postBrokerConnectionsConnection_idValidate(query: { connection_id: string }): Promise<T.BrokerConnectionMetadataDto> {
    return this.request("POST", "/api/v1/broker-connections/{connection_id}/validate", query);
  }

  /** What this deployment can actually do. */
  capabilities(query: { account_id?: string }): Promise<T.CapabilitySet> {
    return this.request("GET", "/api/v1/capabilities", query);
  }

  /** Cancel a regular order. */
  postCommandsCancelOrder(body: T.CancelOrderRequest): Promise<T.MutationReceiptDto> {
    return this.request("POST", "/api/v1/commands/cancel-order", undefined, body);
  }

  /** Cancel a stop order. Exactly one target identity. */
  postCommandsCancelStopOrder(body: T.CancelOrderRequest): Promise<T.MutationReceiptDto> {
    return this.request("POST", "/api/v1/commands/cancel-stop-order", undefined, body);
  }

  /** Submit a regular order. The scope in the body is the frozen target of the command. */
  postCommandsOrder(body: T.SubmitOrderRequest): Promise<T.MutationReceiptDto> {
    return this.request("POST", "/api/v1/commands/order", undefined, body);
  }

  /** Establish protection legs on a position. Not a bulk migration. */
  postCommandsProtection(body: T.SubmitProtectionRequest): Promise<T.MutationReceiptDto> {
    return this.request("POST", "/api/v1/commands/protection", undefined, body);
  }

  /** Replace a live regular order. */
  postCommandsReplaceOrder(body: T.ReplaceOrderRequest): Promise<T.MutationReceiptDto> {
    return this.request("POST", "/api/v1/commands/replace-order", undefined, body);
  }

  /** Submit a stop order. */
  postCommandsStopOrder(body: T.SubmitStopOrderRequest): Promise<T.MutationReceiptDto> {
    return this.request("POST", "/api/v1/commands/stop-order", undefined, body);
  }

  /** The UI uses this list instead of guessing. Intervals the provider does not name never appear; an unknown integer on a candle query fails as `UNSUPPORTED_CANDLE_INTERVAL`. */
  marketCandleIntervals(): Promise<Array<T.CandleIntervalCapability>> {
    return this.request("GET", "/api/v1/market/candle-intervals", undefined);
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

  /** Mutation journal for one scope. Empty when nothing has been dispatched. */
  mutations(query: { account_id: string; broker_connection_id: string; environment: T.BrokerEnvironment; provider: T.ProviderDto }): Promise<Array<T.MutationReceiptDto>> {
    return this.request("GET", "/api/v1/mutations", query);
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

  /** Scopes the operator may select. Empty until #17 bindings exist. */
  runtimeScopes(): Promise<Array<T.ExecutionScope>> {
    return this.request("GET", "/api/v1/runtime/scopes", undefined);
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
