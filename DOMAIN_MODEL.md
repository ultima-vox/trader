# Trader 2.0 — Domain model

## Core identity rules

Every domain entity has one canonical Trader identity. Broker-specific identifiers are aliases, not primary domain identity.

### Instrument

Canonical fields:

- `instrument_id`
- `symbol`
- `venue`
- `asset_class`
- `currency`
- `lot_size`
- `price_increment`
- `quantity_increment`
- `contract_multiplier`
- `expiry` where applicable
- `underlying` where applicable
- broker aliases: UID, FIGI, class code, venue symbol

Contract economics must be explicit and versioned by observation time.

### Account

Represents a trading account/environment and broker relationship.

Fields include canonical account identity, broker, environment (`sandbox|paper|live`), base currency, capabilities and reconciliation status.

### Order identity

Do not collapse identities.

```text
client_request_id != client_order_id != broker_order_id != exchange_order_id
```

Not every broker supplies every identity. Missing values remain absent; they are not fabricated.

### Order lifecycle

Minimum normalized states:

```text
CREATED
PENDING_SUBMIT
OPEN
PARTIALLY_FILLED
FILLED
PENDING_CANCEL
CANCELED
PENDING_REPLACE
REJECTED
EXPIRED
UNKNOWN
```

`UNKNOWN` means a mutation may have reached the broker but the local process lacks authoritative confirmation.

### Fill

A fill is immutable execution evidence. Deduplication uses broker/exchange trade identity when available; otherwise adapter-specific deterministic compatibility identity must be explicitly marked as derived.

### Position

A position is a broker-authoritative exposure projection built from fills and reconciliation reports. Application projections must converge to broker state after reconnect/restart.

## Decision domain

### Signal

Raw strategy/analysis output. Contains direction, strength, horizon, source and evidence references. A signal has no authority to trade.

### AnalysisEvidence

Typed evidence produced by technical, statistical, ML, macro/news or AI analysis.

Required attributes:

- source
- timestamp
- instrument/universe scope
- confidence if the source supports it
- feature/model/version references
- expiry/staleness rule

### TradeCandidate

A normalized candidate opportunity formed from one or more signals/evidence items.

### TradeIntent

Explicit desired economic action before risk/execution:

```text
instrument
side
exposure target or quantity intent
entry preference
horizon
strategy/source
reason/evidence
```

It must not contain broker order IDs or transport details.

### RiskDecision

Result of advanced risk evaluation:

```text
ALLOW
DENY
RESIZE
REQUIRE_PROTECTION
REQUIRE_MANUAL_APPROVAL
```

Includes machine-readable reason codes and calculated risk metrics.

### ExecutionPlan

Broker/runtime-neutral approved plan that may contain one or more executable orders, dependencies and protection requirements.

### PositionPolicy

Defines lifecycle after exposure: stop, take-profit, trailing, time exit, invalidation, scale-in/out and emergency handling.

## Strategy domain

A strategy has:

- immutable strategy ID;
- version;
- parameters/schema;
- supported instruments/timeframes;
- required data inputs;
- lifecycle state (`draft|validated|approved|disabled`);
- automation permission reference;
- risk budget reference.

Backtest and live instantiate the same strategy-domain logic with different runtime/data adapters.

## ML domain

### DatasetDefinition

Reproducible description of source instruments, date range, features, labels, preprocessing and version.

### TrainingRun

Immutable record of code/model/dataset versions, parameters, metrics and artifacts.

### ModelVersion

Lifecycle:

```text
EXPERIMENTAL -> VALIDATED -> APPROVED -> ACTIVE -> RETIRED
```

Only approved/active versions may participate in production decisions.

### Prediction

Contains model version, input snapshot identity, output, timestamp, horizon and quality metadata. Predictions are evidence, not direct orders.

## Automation domain

### AutomationGrant

Explicit permission linking actor/strategy/model to:

- account/environment;
- allowed instruments/universe;
- max order/position/exposure;
- time window;
- risk budget;
- allowed order types;
- expiry/revocation.

Absence of a grant means advisory-only.

## Reconciliation domain

### ReconciliationSnapshot

Broker-authoritative snapshot of orders, fills, positions, cash and account state with observation timestamp.

### ReconciliationResult

Classifies differences:

```text
MATCH
LOCAL_MISSING
BROKER_MISSING
VALUE_MISMATCH
IDENTITY_MISMATCH
UNKNOWN_RESOLVED
MANUAL_REVIEW_REQUIRED
```

Trading readiness depends on reconciliation health.

## Audit domain

Every critical flow carries a `correlation_id`. Durable audit events must connect:

```text
market snapshot
-> evidence
-> candidate
-> intent
-> risk decision
-> execution plan
-> client request
-> broker order
-> fills
-> position outcome
```

This lineage is required for debugging, compliance-style review and strategy evaluation.