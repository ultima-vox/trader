# Vox Trader — backend contract map for the frontend

Issue #18 forbids the frontend from inventing broker, risk, execution, reconciliation or
security semantics, and forbids exposing an option the backend cannot execute. This
document is the conformance map: what the repository's Rust contracts actually expose,
what a screen may therefore render, and which UI concepts have **no contract yet** and
must ship disabled/deferred with a tracked dependency.

Sources of truth (Rust, serde-serializable, `SCREAMING_SNAKE_CASE` on enums):

- `crates/vox-domain/src/{environment,execution,identity,instrument,money,mutation,readiness}.rs`
- `crates/vox-runtime/src/{model,policy,ports}.rs`

There is **no HTTP or gRPC server in the repository yet**: `vox-core` is a configuration
binary, and `reqwest`/`tonic` are used to reach the provider, not to serve Vox clients.
The transport for these contracts is therefore itself a tracked dependency (#11/#17); the
types below are already canonical and a typed frontend client must be generated from them
rather than from a hand-written API guess.

---

## 1. What the contracts expose

### 1.1 Execution target — `RuntimeScope`

```rust
RuntimeScope { provider, environment, broker_account_id, connection_ref, credential_ref }
```

`provider: Provider` = `T_INVEST` (single variant).
`environment: RuntimeEnvironment` = `SANDBOX | PRODUCTION`.
`connection_ref` / `credential_ref`: `OpaqueRef` — rejects anything resembling secret
material (`Bearer`, `token=`, `secret=`, `t.`), renders as `[opaque-ref]`.
`scope.key()` = `Provider:Environment:broker_account_id`; `redacted_account_id()` = `***1234`.

This **is** the frozen command target the UI must carry and display: provider, environment,
account id and the opaque connection/credential references, never the secret itself.

### 1.2 Runtime state — `RuntimeState`, `ReasonCode`, `RuntimeHealth`

`RuntimeState` = `STARTING | CONNECTING | RECONCILING | READY | DEGRADED | HALTED | STOPPING | STOPPED`.
`RuntimeState::new_exposure_allowed()` is true **only** in `READY`.

`ReasonCode` (23 canonical values, the only reason codes that exist):

```
STARTUP · CONNECTING · RECONCILIATION_STARTED · RECONCILIATION_COMPLETE ·
RECONCILIATION_INCOMPLETE · UNKNOWN_MUTATION · BROKER_POSITION_CONFLICT ·
BROKER_ORDER_CONFLICT · BROKER_STOP_CONFLICT · REQUIRED_READ_UNAVAILABLE ·
ACCOUNT_UNAVAILABLE · CREDENTIAL_REJECTED · EXECUTION_UNAUTHORIZED ·
STREAM_DISCONNECTED · STREAM_GAP · STREAM_QUEUE_OVERFLOW ·
OPTIONAL_CAPABILITY_UNAVAILABLE · CHECKPOINT_REBUILD · PERSISTENCE_FAILURE ·
OWNERSHIP_FAILURE · STALE_EPOCH · CORRUPT_MUTATION_EVIDENCE ·
SHUTDOWN_REQUESTED · SHUTDOWN_COMPLETE
```

`RuntimeHealth` gives the shell everything it needs: `state`, `reason_code`, `reason`,
`provider`, `environment`, `account_display`, `runtime_epoch`, `connected`,
`last_successful_reconciliation_at_unix_ms`, `reconciliation_age_ms`,
`unresolved_unknown_count`, `open_order_count`, `active_stop_count`, `stream_states`,
`persistence_healthy`, **`execution_authorized`**, **`new_exposure_allowed`**.

`SafetyCondition` (policy) explains *why* exposure is blocked: `STARTUP_BEFORE_RECONCILIATION`,
`UNRESOLVED_UNKNOWN_MUTATION`, `POSITION_CONFLICT`, `ORDER_IDENTITY_CONFLICT`,
`STOP_IDENTITY_CONFLICT`, `REQUIRED_READ_UNAVAILABLE`, `ACCOUNT_UNAVAILABLE`,
`CREDENTIAL_INVALID`, `EXECUTION_AUTHORIZATION_DISABLED`,
`PRODUCTION_STREAM_DISCONNECTED_AFTER_UNARY_RECOVERY`, `OPTIONAL_ANALYTICS_UNAVAILABLE`,
`CHECKPOINT_CORRUPT_SNAPSHOT_AVAILABLE`, `PERSISTENCE_FAILURE`, `OWNERSHIP_FAILURE`, `CLEAR`.

### 1.3 Dispatch and reconciliation — the real `UNKNOWN_AFTER_DISPATCH`

`JournalState` = `NOT_DISPATCHED | DISPATCHING | ACKNOWLEDGED | REJECTED | UNKNOWN_AFTER_DISPATCH | RECONCILED`
with `safety_unresolved()` = `DISPATCHING | UNKNOWN_AFTER_DISPATCH` and
`terminal()` = `ACKNOWLEDGED | REJECTED | RECONCILED`.

`MutationDecision` = `SUBMIT | RECONCILE | DO_NOT_SUBMIT` — the backend, not the browser,
decides whether a re-submit is allowed. `MutationOutcome` = `NOT_DISPATCHED | ACCEPTED | REJECTED | UNKNOWN`.
`MutationRecord` carries `logical_request_id`, `kind`, `state`, `redacted_request_evidence`
(validated to contain no `authorization`/`bearer`/`token=`/`secret=`), `correlation_id`,
`reconciliation_disposition`, `runtime_epoch`.

`ReconciliationCheckpoint` reports per-domain completeness (`accounts_complete`,
`portfolio_complete`, `positions_complete`, `orders_complete`, `stops_complete`,
`operations_complete`) plus `complete()`.

**Consequence for the UI:** the three outcome labels used in the current reference
(`RECON_CONFIRMED`, `RECON_NOT_FOUND`, `RECON_PENDING`) are **not** contract values. The
contract expresses the same three outcomes as `JournalState` + `MutationOutcome` +
`reconciliation_disposition`, and the reason code for the unresolved case is
`UNKNOWN_MUTATION`.

### 1.4 Read models per account

`BrokerSnapshot { accounts, portfolio, positions, active_orders, stop_orders, operations, stream_evidence, observed_at_unix_ms }`

- `BrokerAccount { account_id, open, accessible }`
- `PositionFact { account_id, instrument_uid, quantity_units, broker_observed_at_unix_ms }`
- `PortfolioFact { account_id, currencies: Map<String,String>, broker_observed_at_unix_ms }`
- `OrderFact { account_id, broker_order_id, logical_request_id, instrument_uid, active, terminal }`
- `StopFact { account_id, broker_stop_order_id, logical_request_id, instrument_uid, active, terminal }`
- `OperationFact { account_id, cursor, provider_operation_id, broker_order_id, logical_request_id, broker_fill_ids }`, paged via `OperationsPage`
- `BrokerEvent { account_id, event_class, stable_event_id, ... }`, `BrokerEventClass` = `ORDER | STOP | FILL | OPERATION | POSITION | PORTFOLIO`
- `StreamHealth { stream, state, queue_depth, last_event_at_unix_ms }`,
  `StreamKind` = `ORDER_STATE | TRADES | POSITIONS | PORTFOLIO | OPERATIONS`,
  `StreamState` = `DISCONNECTED | CONNECTING | ACTIVE | STALE | FAILED`

### 1.5 Commands and protection

`RegularOrderCommand { account_id, instrument_id, client_request_id, quantity_lots, price: Option<FixedPoint>, price_convention, side, order_type, time_in_force, ... }`
`ReplaceOrderCommand`, `CancelOrderCommand`, `CancelStopOrderCommand`.
`OrderSide` = `BUY | SELL`; `RegularOrderType` = `LIMIT | MARKET | BEST_PRICE`;
`TimeInForce` = `DAY | FILL_AND_KILL | FILL_OR_KILL`;
`ExecutionPriceConvention` = `SETTLEMENT_CURRENCY | POINTS`.

`ProtectionPlan { stop_loss: Option<StopLossProtection>, take_profit: Option<TakeProfitProtection> }` —
independent and optional, exactly as the design says.
`TrailingDistance { value: FixedPoint, mode }`, `TrailingDistanceMode` = `ABSOLUTE_PRICE | RELATIVE_PERCENT`.
`TrailingSemanticReference { side: LONG|SHORT, distance, favorable_extreme }` — the
high-water/low-water rule is a backend concept, and the browser only renders it.

`ProtectionCapability { fixed_stop, native_trailing_relative, native_trailing_absolute, take_profit, stop_limit }`
with `ProtectionCapabilityError` = `FIXED_STOP_UNSUPPORTED | STOP_LIMIT_UNSUPPORTED | TAKE_PROFIT_UNSUPPORTED | NATIVE_RELATIVE_TRAILING_UNSUPPORTED | NATIVE_ABSOLUTE_TRAILING_UNSUPPORTED`
— this is the capability gate the UI must obey.

`ProtectionEstablishmentState` = `AWAITING_ENTRY | ESTABLISHING | ACTIVE | UNKNOWN_AFTER_DISPATCH | RECONCILIATION_REQUIRED | CLOSING_POSITION | ORPHANED | TERMINAL`.
`ProtectionLifecycle { broker_stop_order_id, broker_child_order_id, provider_status, provider_trailing_status, broker_reported_extreme, broker_reported_execution_price }`.

### 1.6 Money — exact, never floating point

`FixedPoint(i128)` at `NANO_SCALE = 1_000_000_000`; `UnitsNano { units: i64, nano: i32 }`
with `nano ∈ [-999_999_999, 999_999_999]`. `PortfolioFact.currencies` maps a currency code
to a **string** amount. There is no float anywhere in the contract, and the frontend must
keep it that way: parse to integer nanos, format for display, never `Number` arithmetic on
a capital-affecting value.

### 1.7 Ports (transport shape the client must mirror)

`BrokerReadPort` methods: `GET_ACCOUNTS`, `GET_PORTFOLIO`, `GET_POSITIONS`, `GET_ORDERS`,
`GET_ORDER_STATE`, `GET_STOP_ORDERS`, `GET_OPERATIONS_BY_CURSOR`, `STREAM_CONNECT`, `MUTATION`.
`BrokerResultClass` = `SUCCESS | RATE_LIMITED | CREDENTIAL | PERMISSION | TRANSIENT | PERMANENT`.
`CredentialResolution { execution_authorized }`.

---

## 2. Conformance findings against the current reference

The rendered reference asserts the following, which the contracts do **not** support. Each
must be corrected or shown as an explicitly deferred capability — not simulated.

| # | Reference asserts | Contract reality | Action |
| --- | --- | --- | --- |
| C1 | `PAPER` and `BACKTEST` environment badges | `RuntimeEnvironment` = `SANDBOX \| PRODUCTION` only | Remove from product screens; keep `Environment::Paper` out of the UI until a runtime scope exposes it |
| C2 | Broker connections named Альфа / Финам | `Provider` = `T_INVEST` only | Multi-provider stays as a *shape* (multiple connections per provider is real), but a second provider must render as a deferred capability |
| C3 | Runtime chip shows 4 states | 8 states, incl. `STARTING`, `CONNECTING`, `STOPPING`, `STOPPED` | Extend the runtime vocabulary to the full enum |
| C4 | Risk verdicts `SAFE/WARNING/BLOCKED/UNKNOWN/RESIZE`, day-loss limit, concentration %, margin-after | **No risk contract exists** anywhere in the repo | Re-anchor on `new_exposure_allowed` + `SafetyCondition`; everything else becomes a deferred risk-engine dependency |
| C5 | Invented reason codes: `RISK_CONC_NEAR`, `RISK_DAY_LOSS_NEAR`, `RISK_TRAIL_MAX`, `ORD_LOT_STEP`, `MD_STREAM_SILENT`, `BRK_PROTECT_STALE`, `EXEC_TARGET_MISMATCH`, `RECON_*`, `STR_ACCOUNT_READ_ONLY`, `ML_METRICS_MISSING`, `RSH_HISTORY_GAP` | 23 canonical `ReasonCode` values, none of these | Replace with canonical codes where one fits; where none fits, the message carries no code and the gap is tracked |
| C6 | Protection runtime states `ACTIVE/STALE/RECONCILING/TRIGGERED/CANCELLED` | `ProtectionEstablishmentState` = `AWAITING_ENTRY/ESTABLISHING/ACTIVE/UNKNOWN_AFTER_DISPATCH/RECONCILIATION_REQUIRED/CLOSING_POSITION/ORPHANED/TERMINAL` | Adopt the contract enum; `STALE` becomes an age on the last broker response, not a state |
| C7 | Connection vocabulary `VALIDATING/RECONNECTING/REVOKED/ROTATE/PROVIDER_UNAVAILABLE/DISABLED` | `BrokerResultClass` + `SafetyCondition` (`CREDENTIAL_INVALID`, `EXECUTION_AUTHORIZATION_DISABLED`, `ACCOUNT_UNAVAILABLE`), `StreamState` | Map each chip to a contract value; drop the ones with no backing (`REVOKED`, `ROTATE` as distinct states) |
| C8 | Portfolio value, P&L, free funds, margin | `PortfolioFact.currencies: Map<code,string>` only | Render currency balances; P&L/margin become a deferred analytics dependency |
| C9 | Quotes, order book, tape, chart data | No market-data read model in `vox-runtime` | Deferred market-data dependency; the widgets keep their empty/deferred state |
| C10 | Strategy, Decision Center, Research, ML screens with live data | No contracts of any kind | Screens exist as operator workplaces but every data region is explicitly deferred |
| C11 | Bulk protection migration result | No bulk mutation contract; only per-mutation `MutationRecord` | Design stays; the action is gated on a tracked backend capability |
| C12 | `Все счета` aggregate | No aggregate read model | Already deferred — keep |

## 3. Tracked backend dependencies

| ID | Needed by | Missing contract | Owner |
| --- | --- | --- | --- |
| BD-1 | every screen | Vox-side transport (HTTP/gRPC) exposing the read models above | #11 |
| BD-2 | shell, ticket | risk verdict / guardrail read model (exposure, day loss, concentration, resize) | risk engine, unassigned |
| BD-3 | Markets, chart, book, tape, ticket price | market-data read model (quote, depth, trades, candles) | #8 |
| BD-4 | Portfolio | P&L, margin and valuation analytics | unassigned |
| BD-5 | Settings → Brokers | credential rotation/revocation lifecycle beyond `CredentialResolution` | #17 |
| BD-6 | Strategy, Decision | strategy binding, signal and approval contracts | unassigned |
| BD-7 | ML / Models | dataset, training, registry, promotion contracts | unassigned |
| BD-8 | Research | backtest run contracts | unassigned |
| BD-9 | Protection defaults | account-scoped default protection policy storage | #10 |
| BD-10 | Bulk migration | bulk re-application mutation | #10 |
| BD-11 | `Все счета` | aggregate read model | unassigned |
| BD-12 | Settings → Users | RBAC read model (roles, permissions, actor) | #17 |

Rule for every deferred region: render the widget, name the missing capability in words,
disable the control, and show the tracked dependency id. Never simulate the data.
