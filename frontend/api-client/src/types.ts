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

/** A broker account discovered through a connection. */
export type BrokerAccountDto = {
  account_id: string;
  open: boolean;
  /** Whether the runtime can currently read this account. */
  accessible: boolean;
};

/** The broker-side environment of a connection. Exactly the runtime contract's two values. */
export type BrokerEnvironment = "SANDBOX" | "PRODUCTION";

/** Cancel a regular order by broker identity or by the logical request that created it. */
export type CancelOrderRequest = {
  scope: ExecutionScope;
  client_request_id: string;
  broker_order_id?: string | null;
  logical_request_id?: string | null;
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

/** What a client may send. */
export type ClientMessage = {
  /** Client-generated id, echoed on every event of this subscription. */
  subscription_id: string;
  topic: Topic;
  scope?: unknown | ExecutionScope;
  type: "SUBSCRIBE";
} | {
  subscription_id: string;
  type: "UNSUBSCRIBE";
} | {
  nonce?: string | null;
  type: "PING";
};

/** One currency balance, exact. */
export type CurrencyBalanceDto = {
  currency: string;
  amount: Decimal;
};

/** Exact decimal value, serialized as a string (`"272.55"`, `"-3140.70"`). */
export type Decimal = string;

/** The class of failure, which decides what the operator can do about it. */
export type ErrorCategory = "VALIDATION" | "AUTHENTICATION" | "PERMISSION" | "NOT_FOUND" | "CONFLICT" | "STALE" | "UNRESOLVED_UNKNOWN" | "CAPABILITY_UNAVAILABLE" | "TRANSIENT" | "INTERNAL";

/** The payload of an event, keyed by topic. */
export type EventPayload = {
  data: RuntimeHealthDto;
  topic: "RUNTIME_HEALTH";
};

/** The immutable target of a read or a capital-affecting command. `connection_ref` is an opaque reference, never a credential: the runtime rejects any value that resembles secret material before it can reach this boundary. */
export type ExecutionScope = {
  provider: ProviderDto;
  environment: BrokerEnvironment;
  /** Broker account identifier. Human labels live in the UI; identity stays explicit here. */
  broker_account_id: string;
  /** Opaque connection reference from the runtime. Never a token. */
  connection_ref: string;
  /** How Vox executes for this scope. */
  trading_mode: TradingMode;
};

/** One field-level complaint. */
export type FieldError = {
  /** JSON pointer-ish path of the offending field. */
  field: string;
  message: string;
};

/** Where a dispatched command stands. `UNKNOWN_AFTER_DISPATCH` is an unfinished answer. */
export type JournalStateDto = "NOT_DISPATCHED" | "DISPATCHING" | "ACKNOWLEDGED" | "REJECTED" | "UNKNOWN_AFTER_DISPATCH" | "RECONCILED";

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

/** An order identity and its liveness. Price and quantity are not in the read model yet. */
export type OrderDto = {
  account_id: string;
  broker_order_id: string;
  /** Vox-side identity of the command that created it, when it was ours. */
  logical_request_id?: string | null;
  instrument_uid: string;
  active: boolean;
  terminal: boolean;
};

/** Which side of the market a command takes. The side is the action, never a mode. */
export type OrderSideDto = "BUY" | "SELL";

export type OrderTypeDto = "LIMIT" | "MARKET" | "BEST_PRICE";

/** Portfolio as the broker reports it: currency balances and when they were observed. Valuation, P&L, exposure and margin are **not** here. #22 owns them. */
export type PortfolioDto = {
  account_id: string;
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
  stop_loss_trigger_price?: unknown | Decimal;
  stop_loss_trailing?: unknown | TrailingDistanceDto;
  take_profit_trigger_price?: unknown | Decimal;
  take_profit_limit_price?: unknown | Decimal;
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
  scope?: unknown | ExecutionScope;
  payload: EventPayload;
  type: "SNAPSHOT";
} | {
  schema_version: number;
  subscription_id: string;
  as_of_unix_ms: number;
  runtime_epoch: number;
  /** Monotonic per-subscription sequence, so a gap is detectable. */
  sequence: number;
  scope?: unknown | ExecutionScope;
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

/** A stop order identity and its liveness. Trigger levels are not in the read model yet. */
export type StopOrderDto = {
  account_id: string;
  broker_stop_order_id: string;
  logical_request_id?: string | null;
  instrument_uid: string;
  active: boolean;
  terminal: boolean;
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
  instrument_uid: string;
  /** Client-generated identity of this command, used for idempotency and reconciliation. */
  client_request_id: string;
  side: OrderSideDto;
  order_type: OrderTypeDto;
  quantity_lots: number;
  price?: unknown | Decimal;
  price_convention: PriceConventionDto;
  time_in_force: TimeInForceDto;
  protection?: unknown | ProtectionPlanDto;
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
export type Topic = "RUNTIME_HEALTH" | "POSITIONS" | "ORDERS" | "STOPS" | "OPERATIONS" | "PORTFOLIO";

/** How Vox executes, which is not where the broker lives. Only `LIVE` exists: orders go to the broker connection named by the scope. `PAPER` and `BACKTEST` are owned by #23 and #29 and will be added here when those runtimes exist, not before. */
export type TradingMode = "LIVE";

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
