# Trader 2.0 — Trading pipeline

## Principle

No component before Execution may mutate broker state. The pipeline is intentionally staged so analysis, decision, risk and execution can be inspected and tested independently.

## Canonical path

```text
Market / Reference Data
        |
        v
Feature / Analysis layer
        |
        v
Signals + AnalysisEvidence
        |
        v
TradeCandidate
        |
        v
Decision Engine
        |
        v
TradeIntent
        |
        v
Portfolio Construction
        |
        v
Advanced Risk
        |
        +---- DENY / MANUAL APPROVAL / RESIZE
        |
        v
ExecutionPlan
        |
        v
Nautilus runtime safeguards
        |
        v
Broker Adapter
        |
        v
Broker / Venue
        |
        v
Orders / Fills / Positions
        |
        v
Reconciliation
        |
        v
Portfolio + Position Management + Audit
```

## Stage contracts

### 1. Market snapshot

The decision path consumes a timestamped immutable snapshot/reference to data, never mutable UI state.

Required checks:

- instrument is known and tradeable;
- data is not stale beyond policy;
- contract economics are current;
- market/broker status permits the intended action.

### 2. Analysis

Each analysis producer emits typed evidence. Producers may run independently and at different cadences.

Examples:

- technical/indicator agent;
- statistical regime detector;
- ML forecast;
- order-book/liquidity analysis;
- volatility analysis;
- event/news analysis.

No analysis output is itself executable.

### 3. Decision

Decision aggregates evidence and current portfolio context. It produces a `TradeIntent` or no-action outcome.

A decision must retain evidence lineage and an explicit reason.

### 4. Portfolio construction

Transforms desired trade into account-aware target exposure. It considers existing positions, available capital, strategy budgets and concentration.

Output is still broker-neutral.

### 5. Advanced risk

Risk evaluates projected post-trade state, not only the single order.

Possible outcomes:

- allow as requested;
- resize;
- require protection;
- require manual approval;
- deny.

### 6. Execution planning

Converts an approved economic action into executable order instructions:

- quantity/lots;
- order type;
- limit/stop parameters;
- time-in-force;
- dependency/order grouping;
- protection sequencing;
- execution urgency/slippage policy.

The plan still uses canonical instrument/account identities. Broker translation happens later.

### 7. Runtime execution

NautilusTrader owns runtime order state, execution events, cache/portfolio integration and reconciliation mechanics.

Trader 2.0 advanced risk runs before this layer; Nautilus runtime-level checks remain defense in depth.

### 8. Broker boundary

The broker adapter translates canonical/runtime semantics into exact broker API semantics. It owns:

- broker IDs and idempotency keys;
- rate limits;
- protocol transport;
- reconnect/session behavior;
- broker-specific price/quantity representation;
- authoritative reports.

### 9. Reconciliation

Reconciliation is not an exceptional repair tool. It is part of normal execution lifecycle.

Triggers include:

- startup;
- reconnect;
- UNKNOWN mutation;
- periodic verification;
- broker stream gap/inconsistency;
- operator request.

### 10. Position management

After exposure exists, position policies can create new intents for:

- protective stop;
- take profit;
- trailing exit;
- time stop;
- thesis invalidation;
- scale in/out;
- emergency flatten.

Position management goes through the same risk/execution pipeline; it does not bypass controls.

## Readiness gates

The live runtime has explicit readiness states:

```text
STARTING
CONNECTING
RECONCILING
READY
DEGRADED
HALTED
```

`READY` requires at minimum:

- broker session healthy;
- instrument/reference data valid;
- account snapshot loaded;
- orders/positions reconciled;
- risk configuration loaded;
- no unresolved blocking UNKNOWN state.

## Failure semantics

### Before dispatch

Failure before broker dispatch may safely be classified as `NOT_DISPATCHED` / local failure.

### After dispatch

If dispatch may have occurred but response is unavailable:

```text
LOCAL = UNKNOWN
-> do not blindly resend
-> query/reconcile by identities
-> resolve from broker evidence
```

### Broker rejection

Only explicit broker evidence creates `REJECTED`.

## Manual order path

Manual UI orders do not bypass the architecture:

```text
UI OrderTicket
-> Manual TradeIntent
-> Risk
-> ExecutionPlan
-> Nautilus
-> Broker
```

The UI may expose why risk resized/denied the order.

## Autonomous path

Autonomous execution additionally requires a valid `AutomationGrant`. If absent/expired, the pipeline stops at advisory/manual approval regardless of signal confidence.