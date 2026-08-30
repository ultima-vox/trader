// Generated from docs/api/openapi.json by tools/api-client/generate.py.
// Do not edit: run `python tools/api-client/generate.py` after changing the Rust contracts.


/** The public error envelope. Never carries provider payloads, credentials or stack traces. */
export type ApiError = {
  /** Stable machine code, screaming snake case. */
  code: string;
  /** Human sentence for the operator. */
  message: string;
  /** Correlates this response with server logs and with a mutation, when there is one. */
  correlation_id: string;
  category: ErrorCategory;
  retryable: boolean;
  field_errors?: Array<FieldError>;
  /** Safe, typed extra context. Never provider metadata. */
  details?: unknown;
};

export type BindBrokerAccountRequest = {
  provider_account_id: string;
  account_id: string;
};

export type BrokerAccountBindingDto = {
  binding_id: string;
  connection_id: string;
  provider: ProviderDto;
  environment: BrokerEnvironment;
  provider_account_id: string;
  account_id: string;
  enabled: boolean;
  created_at_unix_ms: number;
  updated_at_unix_ms: number;
};

/** A broker account discovered through a connection. */
export type BrokerAccountDto = {
  /** Canonical Vox account/binding identity. */
  account_id: string;
  /** Provider broker-account identifier. Metadata, not the capital-command target key. */
  broker_account_id: string;
  open: boolean;
  /** Whether the runtime can currently read this account. */
  accessible: boolean;
};

export type BrokerConnectionMetadataDto = {
  connection_id: string;
  provider: ProviderDto;
  environment: BrokerEnvironment;
  display_label: string;
  enabled: boolean;
  credential_status: CredentialStatusDto;
  credential_class: CredentialClassDto;
  credential_scope: CredentialScopeDto;
  capabilities: Array<ConnectionCapabilityDto>;
  health: ConnectionHealthDto;
  created_at_unix_ms: number;
  updated_at_unix_ms: number;
};

/** The broker-side environment of a connection. Exactly the runtime contract's two values. */
export type BrokerEnvironment = "SANDBOX" | "PRODUCTION";

/** Cancel a regular order. Exactly one of `broker_order_id` or `logical_request_id` names the command being cancelled. `client_request_id` is the identity of *this* cancel. */
export type CancelOrderRequest = {
  scope: ExecutionScope;
  client_request_id: string;
  broker_order_id?: string | null;
  logical_request_id?: string | null;
};

/** One candle. `state` is explicit: forming, settled, or a post-close correction. */
export type CandleDto = {
  instrument_uid: string;
  interval: CandleIntervalDto;
  /** Opening time of the bar, milliseconds since the Unix epoch, UTC. */
  opened_at_unix_ms: number;
  open: Decimal;
  high: Decimal;
  low: Decimal;
  close: Decimal;
  volume_units: number;
  state: CandleStateDto;
  /** Zero at first publication of this open time. Increments on each later publish. */
  revision: number;
};

/** Whether one interval is historic-only, stream-capable, or (by absence) unsupported. Unsupported provider integers never appear here. An unknown integer fails as `UNSUPPORTED_CANDLE_INTERVAL`. */
export type CandleIntervalCapability = {
  interval: CandleIntervalDto;
  /** Accepted by historic GetCandles / `#8` `CanonicalCandle.interval`. */
  historical_supported: boolean;
  /** Accepted by MarketDataStream `SubscriptionInterval`. False for 5s/10s/30s. */
  streaming_supported: boolean;
};

/** Candle interval for the Vox read model. `#8` `CanonicalCandle.interval` is the already-accepted historic GetCandles integer (`candle_request_constraint`, 1..=16). This enum names that integer; it does not invent a second wire table. Stream support is the separate MarketDataStream `SubscriptionInterval` surface (1..=13). 5s/10s/30s exist on GetCandles only; they are not stream-subscribable. */
export type CandleIntervalDto = "FIVE_SECONDS" | "TEN_SECONDS" | "THIRTY_SECONDS" | "ONE_MINUTE" | "TWO_MINUTES" | "THREE_MINUTES" | "FIVE_MINUTES" | "TEN_MINUTES" | "FIFTEEN_MINUTES" | "THIRTY_MINUTES" | "ONE_HOUR" | "TWO_HOURS" | "FOUR_HOURS" | "ONE_DAY" | "ONE_WEEK" | "ONE_MONTH";

/** Lifecycle of one bar. Replaces a boolean `closed` so OPEN / CLOSED / CORRECTED do not have to be inferred. */
export type CandleStateDto = "OPEN" | "CLOSED" | "CORRECTED";

/** A page of candles for one instrument and interval. */
export type CandlesDto = {
  instrument_uid: string;
  interval: CandleIntervalDto;
  candles: Array<CandleDto>;
  freshness: MarketFreshness;
};

/** A capability the frontend may gate on. */
export type Capability = "RUNTIME_HEALTH" | "ACCOUNT_READ_SIDE" | "ORDER_EXECUTION" | "PROTECTION_EXECUTION" | "PROTECTION_DEFAULTS" | "BULK_PROTECTION_MIGRATION" | "CONNECTION_MANAGEMENT" | "RBAC" | "RISK_VERDICT" | "PORTFOLIO_VALUATION" | "MARKET_DATA" | "STRATEGY" | "DECISION" | "MACHINE_LEARNING" | "RESEARCH" | "AGGREGATE_ACCOUNTS" | "MULTI_PROVIDER" | "NON_LIVE_TRADING_MODE";

/** What this deployment can actually do, for one scope. */
export type CapabilitySet = {
  provider: ProviderDto;
  environment: BrokerEnvironment;
  /** Present when the capability set is account-scoped. */
  account_id?: string | null;
  supported: Array<Capability>;
  unavailable: Array<UnavailableCapability>;
};

export type ChangeExecutionAuthorizationRequest = {
  provider_account_id: string;
  mode: ExecutionAuthorizationModeDto;
};

/** What a client may send. */
export type ClientMessage = {
  /** Client-generated id, echoed on every event of this subscription. */
  subscription_id: string;
  topic: Topic;
  scope?: null | ExecutionScope;
  /** Required for market topics. Provider uid inside a provider-named feed. */
  instrument_uid?: string | null;
  interval?: null | CandleIntervalDto;
  type: "SUBSCRIBE";
} | {
  subscription_id: string;
  type: "UNSUBSCRIBE";
} | {
  nonce?: string | null;
  type: "PING";
};

export type ConnectionCapabilityDto = "ACCOUNT_DISCOVERY" | "PORTFOLIO_READ" | "POSITIONS_READ" | "OPERATIONS_READ" | "STREAM_HEALTH" | "SANDBOX_ORDERS" | "PRODUCTION_ORDERS_PROVIDER_ALLOWED";

export type ConnectionDetailsDto = {
  connection: BrokerConnectionMetadataDto;
  accounts: Array<DiscoveredBrokerAccountDto>;
  bindings: Array<BrokerAccountBindingDto>;
  execution_authorizations: Array<ExecutionAuthorizationDto>;
};

export type ConnectionHealthDto = {
  state: ConnectionHealthStateDto;
  checked_at_unix_ms?: number | null;
  reason_code: ConnectionHealthReasonDto;
  safe_detail?: string | null;
  retryable: boolean;
};

export type ConnectionHealthReasonDto = "NONE" | "INVALID_CREDENTIAL" | "EXPIRED_OR_INACTIVE" | "PERMISSION_DENIED" | "WRONG_ENVIRONMENT" | "PROVIDER_UNAVAILABLE" | "ACCOUNT_ACCESS_CHANGED" | "DISABLED_BY_OPERATOR";

export type ConnectionHealthStateDto = "UNKNOWN" | "VALIDATING" | "HEALTHY" | "INVALID_CREDENTIAL" | "INSUFFICIENT_PERMISSION" | "PROVIDER_UNAVAILABLE" | "ACCOUNT_ACCESS_CHANGED" | "DISABLED";

export type CreateBrokerConnectionRequest = {
  provider: ProviderDto;
  environment: BrokerEnvironment;
  display_label: string;
  credential: string;
};

export type CredentialClassDto = "UNKNOWN" | "READ_ONLY" | "FULL_ACCESS" | "TRANSFER_ACCESS" | "SANDBOX";

export type CredentialRotationResultDto = {
  connection: BrokerConnectionMetadataDto;
  reconnect_required: boolean;
};

export type CredentialScopeDto = "NOT_CONFIRMED" | "SINGLE_ACCOUNT_RESTRICTED" | "ALL_ACCESSIBLE_ACCOUNTS";

export type CredentialStatusDto = "PENDING_VALIDATION" | "VALID" | "INVALID" | "EXPIRED_OR_INACTIVE" | "PENDING_DISABLE" | "DISABLED" | "PENDING_DELETE";

/** One currency balance, exact. */
export type CurrencyBalanceDto = {
  currency: string;
  amount: Decimal;
};

/** Exact decimal value, serialized as a string (`"272.550000000"`, `"-3140.700000000"`). */
export type Decimal = string;

/** One level of the book. */
export type DepthLevelDto = {
  price: Decimal;
  /** Size at this level, in instrument units. */
  size_units: number;
  /** Cumulative size from the top of book down to this level. */
  cumulative_units: number;
};

export type DiscoveredBrokerAccountDto = {
  connection_id: string;
  provider: ProviderDto;
  environment: BrokerEnvironment;
  provider_account_id: string;
  display_name?: string | null;
  account_type: string;
  account_status: string;
  access_level: string;
  opened_at_unix_ms?: number | null;
  closed_at_unix_ms?: number | null;
  accessible: boolean;
  capabilities: Array<ConnectionCapabilityDto>;
  discovered_at_unix_ms: number;
};

/** The class of failure, which decides what the operator can do about it. */
export type ErrorCategory = "VALIDATION" | "AUTHENTICATION" | "PERMISSION" | "NOT_FOUND" | "CONFLICT" | "STALE" | "UNRESOLVED_UNKNOWN" | "CAPABILITY_UNAVAILABLE" | "TRANSIENT" | "INTERNAL";

/** The payload of an event, keyed by topic. */
export type EventPayload = {
  data: RuntimeHealthDto;
  topic: "RUNTIME_HEALTH";
} | {
  data: QuoteDto;
  topic: "QUOTE";
} | {
  data: OrderBookDto;
  topic: "ORDER_BOOK";
} | {
  data: Array<TradeTickDto>;
  topic: "TRADES";
} | {
  data: CandlesDto;
  topic: "CANDLES";
} | {
  data: SessionDto;
  topic: "SESSION";
} | {
  data: Array<PositionDto>;
  topic: "POSITIONS";
} | {
  data: Array<OrderDto>;
  topic: "ORDERS";
} | {
  data: Array<StopOrderDto>;
  topic: "STOPS";
} | {
  data: Array<OperationDto>;
  topic: "OPERATIONS";
} | {
  data: PortfolioDto;
  topic: "PORTFOLIO";
};

export type ExecutionAuthorizationDto = {
  connection_id: string;
  provider_account_id: string;
  mode: ExecutionAuthorizationModeDto;
  authorization_revision: number;
  changed_by: string;
  changed_at_unix_ms: number;
};

export type ExecutionAuthorizationModeDto = "DISABLED" | "MANUAL_ALLOWED" | "AUTOMATED_ALLOWED";

/** The immutable target of a read or a capital-affecting command. `broker_connection_id` is the application connection identity, never a credential. `account_id` is the canonical Vox account/binding identity. Provider broker-account identifiers are read-side metadata, not this key. */
export type ExecutionScope = {
  provider: ProviderDto;
  environment: BrokerEnvironment;
  /** Application connection identity. Opaque; never a token. */
  broker_connection_id: string;
  /** Canonical Vox account/binding identity. */
  account_id: string;
  /** How Vox executes for this scope. */
  trading_mode: TradingMode;
};

/** One field-level complaint. */
export type FieldError = {
  /** JSON pointer-ish path of the offending field. */
  field: string;
  message: string;
};

/** The canonical instrument identity, as the domain defines it. Identity is `provider` + `uid`. Everything else is an alias for humans and for lookup. */
export type InstrumentIdentityDto = {
  /** Provider whose namespace `uid` belongs to. */
  provider: string;
  /** The provider's stable instrument identifier. Diagnostics only in normal UI. */
  uid: string;
  /** FIGI where the provider supplies one. An alias, never the identity. */
  figi?: string | null;
  /** Exchange ticker. Shown to operators; not unique across venues or over time. */
  ticker: string;
  /** Venue or board code that qualifies the ticker. */
  class_code: string;
};

/** A catalogue entry: the canonical identity plus what the ticket needs to validate an order. Lot size and price step are metadata, not UI rules: without them a quantity field would be guessing, and this API refuses to make the browser guess. */
export type InstrumentSummaryDto = {
  identity: InstrumentIdentityDto;
  /** Instrument units in one lot. */
  lot_size: number;
  /** Minimum price increment. */
  min_price_increment: Decimal;
  /** Settlement currency code. */
  currency: string;
  /** Whether the provider currently lists the instrument as tradable. */
  tradable: boolean;
};

/** Where a dispatched command stands. `UNKNOWN_AFTER_DISPATCH` is an unfinished answer. */
export type JournalStateDto = "NOT_DISPATCHED" | "DISPATCHING" | "ACKNOWLEDGED" | "REJECTED" | "UNKNOWN_AFTER_DISPATCH" | "RECONCILED";

/** How current a market-data record is. Freshness is a property of the feed, not of the instrument, and it is never folded into the price: a stale quote stays visible with its age rather than disappearing or pretending. */
export type MarketFreshness = {
  /** Connectivity of the feed that produced this record. */
  stream: StreamStateDto;
  /** Event time of the record itself, milliseconds since the Unix epoch, UTC. */
  observed_at_unix_ms: number;
  /** Age of the record at the moment the API answered. */
  age_ms: number;
};

/** One aggregate broker valuation, exact. Not a spendable cash balance. */
export type MoneyValuationDto = {
  currency: string;
  amount: Decimal;
};

/** Whether the client may submit, must reconcile first, or must not submit at all. Decided by the backend; the browser never derives it. */
export type MutationDecisionDto = "SUBMIT" | "RECONCILE" | "DO_NOT_SUBMIT";

export type MutationKindDto = "POST_ORDER" | "POST_ORDER_ASYNC" | "REPLACE_ORDER" | "CANCEL_ORDER" | "POST_STOP_ORDER" | "CANCEL_STOP_ORDER" | "PROTECTION_LEG";

/** The receipt of a capital-affecting command. */
export type MutationReceiptDto = {
  /** Vox-side identity of the command, stable across retries and reconciliation. */
  logical_request_id: string;
  /** The frozen target this command was dispatched to. */
  scope: ExecutionScope;
  kind: MutationKindDto;
  state: JournalStateDto;
  /** What the backend says the client may do next. */
  decision: MutationDecisionDto;
  correlation_id: string;
  /** Set only once the broker has answered. */
  broker_order_id?: string | null;
  broker_stop_order_id?: string | null;
  /** Runtime disposition after reconciliation, when it has run. */
  reconciliation_disposition?: string | null;
  runtime_epoch: number;
  created_at_unix_ms: number;
  updated_at_unix_ms: number;
};

/** An operation identity used for reconciliation. Amounts and kinds are owned by #22. */
export type OperationDto = {
  account_id: string;
  cursor: string;
  provider_operation_id?: string | null;
  broker_order_id?: string | null;
  logical_request_id?: string | null;
  broker_fill_ids: Array<string>;
};

/** A page of operations with the cursor to continue from. */
export type OperationsPageDto = {
  items: Array<OperationDto>;
  next_cursor?: string | null;
};

/** A book snapshot at one depth. */
export type OrderBookDto = {
  instrument_uid: string;
  /** Number of levels the provider returned per side. */
  depth: number;
  /** Best ask first. */
  asks: Array<DepthLevelDto>;
  /** Best bid first. */
  bids: Array<DepthLevelDto>;
  spread_absolute?: null | Decimal;
  freshness: MarketFreshness;
};

/** Order identity and exact provider execution status. */
export type OrderDto = {
  account_id: string;
  broker_order_id: string;
  /** Vox-side identity of the command that created it, when it was ours. */
  logical_request_id?: string | null;
  instrument_uid: string;
  status: OrderExecutionStatusDto;
  status_cause_code?: number | null;
};

export type OrderExecutionStatusDto = {
  status: "NEW";
} | {
  status: "PARTIALLY_FILLED";
} | {
  status: "FILLED";
} | {
  status: "CANCELLED";
} | {
  status: "REJECTED";
} | {
  wire_value: number;
  status: "UNKNOWN_PROVIDER_STATUS";
};

/** Which side of the market a command takes. The side is the action, never a mode. */
export type OrderSideDto = "BUY" | "SELL";

export type OrderTypeDto = "LIMIT" | "MARKET" | "BEST_PRICE";

/** Portfolio aggregates and authoritative cash balances remain separate. */
export type PortfolioDto = {
  account_id: string;
  total_portfolio_valuation?: null | MoneyValuationDto;
  total_currency_valuation?: null | MoneyValuationDto;
  /** Actual currency balances from GetPositions, never inferred from portfolio aggregates. */
  balances: Array<CurrencyBalanceDto>;
  broker_observed_at_unix_ms?: number | null;
};

/** A position as the broker reports it: instrument and quantity, nothing derived. */
export type PositionDto = {
  account_id: string;
  instrument_uid: string;
  /** Signed quantity in instrument units. */
  quantity_units: number;
  broker_observed_at_unix_ms?: number | null;
};

export type PriceConventionDto = "SETTLEMENT_CURRENCY" | "POINTS";

/** Stop loss and take profit are independent and both optional. */
export type ProtectionPlanDto = {
  stop_loss_trigger_price?: null | Decimal;
  stop_loss_trailing?: null | TrailingDistanceDto;
  take_profit_trigger_price?: null | Decimal;
  take_profit_limit_price?: null | Decimal;
};

/** The canonical protection lifecycle, all ten states. Two of them carry data and are the two an operator most needs to see: a position that filled only partly and is therefore only partly protected, and protection that failed after the position was already open. `STALE` is deliberately absent: staleness is the age of the last broker answer, carried by stream health, not a lifecycle state. */
export type ProtectionStateDto = "AWAITING_ENTRY" | {
  /** The entry filled in part, so only part of the position carries protection. */
  ENTRY_PARTIALLY_FILLED: {
  filled_lots: number;
  protected_lots: number;
};
} | "ESTABLISHING" | "ACTIVE" | {
  /** The position is open and its protection did not establish. It is unprotected. */
  FAILED_AFTER_ENTRY: {
  reason: string;
};
} | "UNKNOWN_AFTER_DISPATCH" | "RECONCILIATION_REQUIRED" | "CLOSING_POSITION" | "ORPHANED" | "TERMINAL";

/** Providers with a registered adapter. A provider appears here only once it is real. */
export type ProviderDto = "T_INVEST";

/** Last price and the top of book, with the day's range. Absent fields mean the provider did not supply them for this instrument, which is a different thing from a zero and must not be rendered as one. */
export type QuoteDto = {
  instrument_uid: string;
  last?: null | Decimal;
  bid?: null | Decimal;
  ask?: null | Decimal;
  change_absolute?: null | Decimal;
  change_percent?: null | Decimal;
  day_high?: null | Decimal;
  day_low?: null | Decimal;
  /** Traded volume in instrument units. */
  volume_units?: number | null;
  freshness: MarketFreshness;
};

/** Why the runtime is in its current state. The only reason vocabulary that exists. */
export type ReasonCodeDto = "STARTUP" | "CONNECTING" | "RECONCILIATION_STARTED" | "RECONCILIATION_COMPLETE" | "RECONCILIATION_INCOMPLETE" | "UNKNOWN_MUTATION" | "BROKER_POSITION_CONFLICT" | "BROKER_ORDER_CONFLICT" | "BROKER_STOP_CONFLICT" | "REQUIRED_READ_UNAVAILABLE" | "ACCOUNT_UNAVAILABLE" | "CREDENTIAL_REJECTED" | "EXECUTION_UNAUTHORIZED" | "STREAM_DISCONNECTED" | "STREAM_GAP" | "STREAM_QUEUE_OVERFLOW" | "OPTIONAL_CAPABILITY_UNAVAILABLE" | "CHECKPOINT_REBUILD" | "PERSISTENCE_FAILURE" | "OWNERSHIP_FAILURE" | "STALE_EPOCH" | "CORRUPT_MUTATION_EVIDENCE" | "SHUTDOWN_REQUESTED" | "SHUTDOWN_COMPLETE";

/** How complete the last reconciliation was, per domain. */
export type ReconciliationDto = {
  scope_key: string;
  reconciliation_id: string;
  operations_cursor?: string | null;
  snapshot_observed_at_unix_ms: number;
  completed_at_unix_ms: number;
  runtime_epoch: number;
  accounts_complete: boolean;
  portfolio_complete: boolean;
  positions_complete: boolean;
  orders_complete: boolean;
  stops_complete: boolean;
  operations_complete: boolean;
  /** True only when every domain above is complete. */
  complete: boolean;
};

/** Replace a live regular order. Identifies the original with exactly one target. */
export type ReplaceOrderRequest = {
  scope: ExecutionScope;
  instrument_id: string;
  client_request_id: string;
  broker_order_id?: string | null;
  logical_request_id?: string | null;
  quantity_lots: number;
  price?: null | Decimal;
};

export type RotateCredentialRequest = {
  credential: string;
};

/** Everything the shell needs to answer "can I trade right now, and if not why". */
export type RuntimeHealthDto = {
  state: RuntimeStateDto;
  reason_code: ReasonCodeDto;
  /** Human sentence for the operator. The code is the diagnostic, this is the explanation. */
  reason: string;
  provider: ProviderDto;
  environment: BrokerEnvironment;
  /** Human account label. Raw identifiers stay in diagnostics. */
  account_display: string;
  /** Monotonic ownership epoch. A response from a previous epoch must be discarded. */
  runtime_epoch: number;
  connected: boolean;
  last_successful_reconciliation_at_unix_ms?: number | null;
  reconciliation_age_ms?: number | null;
  unresolved_unknown_count: number;
  open_order_count: number;
  active_stop_count: number;
  stream_states: Array<StreamHealthDto>;
  persistence_healthy: boolean;
  /** Whether Vox execution is authorized for this scope. Not the same as broker permission. */
  execution_authorized: boolean;
  /** Whether new exposure may be created right now. */
  new_exposure_allowed: boolean;
};

/** Runtime lifecycle. New exposure is permitted in `READY` only. */
export type RuntimeStateDto = "STARTING" | "CONNECTING" | "RECONCILING" | "READY" | "DEGRADED" | "HALTED" | "STOPPING" | "STOPPED";

/** Why new exposure is blocked, when it is. Authoritative safety facts until #21 lands. */
export type SafetyConditionDto = "STARTUP_BEFORE_RECONCILIATION" | "UNRESOLVED_UNKNOWN_MUTATION" | "POSITION_CONFLICT" | "ORDER_IDENTITY_CONFLICT" | "STOP_IDENTITY_CONFLICT" | "REQUIRED_READ_UNAVAILABLE" | "ACCOUNT_UNAVAILABLE" | "CREDENTIAL_INVALID" | "EXECUTION_AUTHORIZATION_DISABLED" | "PRODUCTION_STREAM_DISCONNECTED_AFTER_UNARY_RECOVERY" | "OPTIONAL_ANALYTICS_UNAVAILABLE" | "CHECKPOINT_CORRUPT_SNAPSHOT_AVAILABLE" | "PERSISTENCE_FAILURE" | "OWNERSHIP_FAILURE" | "CLEAR";

/** What the server sends. */
export type ServerEvent = {
  schema_version: number;
  subscription_id: string;
  /** Event time in milliseconds since the Unix epoch, UTC. */
  as_of_unix_ms: number;
  /** Runtime ownership epoch this event belongs to. */
  runtime_epoch: number;
  scope?: null | ExecutionScope;
  payload: EventPayload;
  type: "SNAPSHOT";
} | {
  schema_version: number;
  subscription_id: string;
  as_of_unix_ms: number;
  runtime_epoch: number;
  /** Monotonic per-subscription sequence, so a gap is detectable. */
  sequence: number;
  scope?: null | ExecutionScope;
  payload: EventPayload;
  type: "UPDATE";
} | {
  schema_version: number;
  subscription_id: string;
  status: SubscriptionStatus;
  detail?: string | null;
  type: "STATUS";
} | {
  schema_version: number;
  subscription_id?: string | null;
  code: string;
  message: string;
  correlation_id: string;
  type: "ERROR";
} | {
  schema_version: number;
  server_time_unix_ms: number;
  nonce?: string | null;
  type: "HEARTBEAT";
};

/** The current session for one instrument. */
export type SessionDto = {
  instrument_uid: string;
  status: TradingStatusDto;
  /** Whether the venue accepts limit orders right now, when the provider says so. */
  limit_orders_available?: boolean | null;
  /** Whether the venue accepts market orders right now, when the provider says so. */
  market_orders_available?: boolean | null;
  freshness: MarketFreshness;
};

export type StopExecutionStatusDto = {
  status: "ACTIVE";
} | {
  status: "EXECUTED";
} | {
  status: "CANCELED";
} | {
  status: "EXPIRED";
} | {
  wire_value: number;
  status: "UNKNOWN_PROVIDER_STATUS";
};

/** Stop identity and exact provider status. Provider readback has no logical request identity. */
export type StopOrderDto = {
  account_id: string;
  broker_stop_order_id: string;
  instrument_uid: string;
  status: StopExecutionStatusDto;
  status_cause_code?: number | null;
};

/** Health of one broker stream, including how old its last event is. */
export type StreamHealthDto = {
  stream: StreamKindDto;
  state: StreamStateDto;
  queue_depth: number;
  /** Event time of the last message, in milliseconds since the Unix epoch, UTC. */
  last_event_at_unix_ms?: number | null;
};

/** Which broker stream a health record describes. */
export type StreamKindDto = "ORDER_STATE" | "TRADES" | "POSITIONS" | "PORTFOLIO" | "OPERATIONS";

/** Stream connectivity. `STALE` here is a stream fact, never a protection lifecycle state. */
export type StreamStateDto = "DISCONNECTED" | "CONNECTING" | "ACTIVE" | "STALE" | "FAILED";

/** Submit a regular order. Quantity is in lots; price is exact and optional for market orders. */
export type SubmitOrderRequest = {
  /** The immutable target. A submitted command is never retargeted by a later UI change. */
  scope: ExecutionScope;
  /** Opaque canonical instrument identity. Provider UID/FIGI mapping stays inside adapters. */
  instrument_id: string;
  /** Client-generated identity of this command, used for idempotency and reconciliation. */
  client_request_id: string;
  side: OrderSideDto;
  order_type: OrderTypeDto;
  quantity_lots: number;
  price?: null | Decimal;
  price_convention: PriceConventionDto;
  time_in_force: TimeInForceDto;
  protection?: null | ProtectionPlanDto;
};

/** Establish or replace protection legs on an existing position. Not a bulk migration. */
export type SubmitProtectionRequest = {
  scope: ExecutionScope;
  instrument_id: string;
  client_request_id: string;
  plan: ProtectionPlanDto;
};

/** Submit a stop order. Trigger is exact; limit price is optional. */
export type SubmitStopOrderRequest = {
  scope: ExecutionScope;
  instrument_id: string;
  client_request_id: string;
  side: OrderSideDto;
  quantity_lots: number;
  trigger_price: Decimal;
  limit_price?: null | Decimal;
};

/** Why a subscription is in its current status. */
export type SubscriptionStatus = "ACTIVE" | "CANCELLED" | "DROPPED_SLOW_CONSUMER" | "UNAVAILABLE";

/** Liveness of the API process itself, independent of any broker connection. */
export type SystemHealthDto = {
  /** Always `"ok"` when the process can serve requests. */
  status: string;
  /** Public API version this process serves. */
  api_version: string;
  /** Server time in milliseconds since the Unix epoch, UTC. */
  server_time_unix_ms: number;
};

export type TimeInForceDto = "DAY" | "FILL_AND_KILL" | "FILL_OR_KILL";

/** Topics a client may subscribe to. A topic exists only when its read model does. */
export type Topic = "RUNTIME_HEALTH" | "POSITIONS" | "ORDERS" | "STOPS" | "OPERATIONS" | "PORTFOLIO" | "QUOTES" | "ORDER_BOOK" | "TRADES" | "CANDLES" | "SESSION";

/** Which side initiated a public trade, where the provider reports it. */
export type TradeDirectionDto = "BUY" | "SELL" | "UNKNOWN";

/** One public trade on the tape. */
export type TradeTickDto = {
  instrument_uid: string;
  price: Decimal;
  size_units: number;
  direction: TradeDirectionDto;
  /** Exchange time of the trade, milliseconds since the Unix epoch, UTC. */
  traded_at_unix_ms: number;
  /** True when this trade is one of the operator's own, matched by broker fill identity. */
  own: boolean;
};

/** How Vox executes, which is not where the broker lives. Only `LIVE` exists: orders go to the broker connection named by the scope. `PAPER` and `BACKTEST` are owned by #23 and #29 and will be added here when those runtimes exist, not before. */
export type TradingMode = "LIVE";

/** Whether the venue is currently accepting orders for an instrument. This is the venue's session state, not a Vox permission: execution authorization is a separate fact carried by runtime health. */
export type TradingStatusDto = "CLOSED" | "AUCTION" | "OPEN" | "SUSPENDED" | "UNKNOWN";

/** A trailing distance: an exact value plus the mode it is measured in. */
export type TrailingDistanceDto = {
  value: Decimal;
  mode: TrailingModeDto;
};

export type TrailingModeDto = "ABSOLUTE_PRICE" | "RELATIVE_PERCENT";

/** Why a capability is not available, and who owns making it so. */
export type UnavailableCapability = {
  capability: Capability;
  /** Human sentence naming what is missing. */
  reason: string;
  /** The issue that owns the missing contract, for example `#21`. */
  owner: string;
};
