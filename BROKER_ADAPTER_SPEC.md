# Trader 2.0 — Broker adapter specification

## Purpose

Broker adapters isolate venue/provider semantics from the rest of Trader 2.0. The adapter is not a thin HTTP wrapper: it owns translation, identity mapping, rate limiting, connectivity, reconciliation inputs and broker-specific safety constraints.

## Core rule

Never change broker semantics merely to fit a generic internal abstraction. Preserve original broker metadata and expose unsupported/ambiguous cases explicitly.

## Adapter responsibilities

### Reference data

- discover instruments;
- map broker instrument identities to canonical identities;
- expose lot size, tick size, currency, expiry, multiplier/contract economics;
- maintain freshness/validity metadata;
- fail closed if required economics are unavailable.

### Market data

- unary/history requests;
- real-time stream lifecycle;
- desired-subscription registry;
- reconnect and resubscribe;
- timestamp preservation;
- lot/unit conversion;
- order-book consistency handling;
- stale/gap/quality flags.

### Execution

- submit, cancel, replace;
- exact price/quantity/order-type translation;
- idempotency keys;
- broker order identity mapping;
- transport failure classification;
- broker rejection mapping;
- execution reports/fills.

### Reconciliation

Provide authoritative reports sufficient to recover:

- active/recent orders;
- individual order state;
- fills where supported;
- positions;
- cash/balances;
- account state.

### Connectivity and rate limits

The adapter owns broker-specific limits and prioritization. Product modules must not independently call broker APIs around the adapter.

Suggested priority:

```text
EXECUTION
RECONCILIATION
ACCOUNT/POSITIONS
MARKET DATA
RESEARCH/HISTORY
UI AUXILIARY
```

## Identity model

Persist mappings explicitly:

```text
canonical instrument ID <-> broker UID/FIGI/class code
client_request_id
client_order_id
broker_order_id
exchange_order_id (when available)
```

IDs are never silently substituted for each other.

## Mutation outcome model

### Not dispatched

If failure is proven before transport dispatch, report local `NOT_DISPATCHED`/failure.

### Broker rejected

Only explicit broker evidence maps to `REJECTED`.

### Unknown

If the request may have left the process but no authoritative response exists:

```text
UNKNOWN
```

Required behavior:

1. keep risk reservation;
2. do not blind-retry;
3. reconcile using request/order identities and broker reports;
4. resolve to authoritative state.

## T-Invest-specific baseline

Qualification established the following requirements:

- REST and WebSocket/gRPC surfaces may use different field names/semantics; adapters test actual contracts;
- T-Invest quantities in market-data trades/order books/candles are lot-based where documented and must be normalized explicitly;
- futures quote prices may be in points;
- `min_price_increment_amount` from authoritative futures margin metadata is required for monetary point conversion;
- `TYPE_*` asset enums are normalized explicitly;
- replace uses a fresh idempotency key and may return a new broker order ID;
- restart/UNKNOWN recovery uses broker-authoritative order/account reports.

## Interface shape

Exact code interfaces will be defined during implementation, but the conceptual ports are:

```text
InstrumentProvider
MarketDataClient
ExecutionClient
AccountClient
ReconciliationProvider
BrokerHealth
BrokerRateLimiter
```

All ports expose canonical typed domain objects or explicit adapter report types; raw dictionaries do not cross the broker boundary into product modules.

## Reconnect contract

After transport loss:

1. mark connectivity degraded;
2. reconnect with bounded exponential backoff + jitter;
3. authenticate;
4. restore desired subscriptions;
5. validate subscription acknowledgements;
6. rebuild authoritative order-book/stream state where needed;
7. reconcile execution/account state;
8. only then return trading readiness to READY.

## Secrets

Broker tokens/credentials:

- never sent to browser clients;
- never logged;
- never stored in plaintext application tables;
- loaded into broker process via protected secret mechanism;
- support rotation without application redesign.

## Testing contract

Every broker adapter requires:

- synthetic unit tests for translation/math;
- contract tests against sandbox/test environment where available;
- reconnect tests;
- UNKNOWN/reconciliation tests;
- restart recovery tests;
- rate-limit behavior tests;
- instrument economics regression vectors;
- explicit unsupported-feature tests.

Production enablement is blocked until these pass for the target broker/environment.