# Trader 2.0 — Architecture

## Architectural style

Trader 2.0 starts as a **modular monolith with isolated runtimes and replaceable adapters**, not as a traditional all-in-one monolith and not as a microservice fleet.

The important unit is the **bounded module**, not the process. Modules communicate through explicit application interfaces and domain events. A module may later be moved into its own process without changing business semantics.

This avoids two failure modes:

- one giant tightly coupled application where every feature shares global state;
- premature microservices with network, deployment and consistency complexity before there is a scaling need.

## Top-level architecture

```text
Clients
  Web UI / future desktop-mobile clients
                    |
             HTTPS / WebSocket
                    v
+--------------------------------------------------+
|              Trader Application                 |
|--------------------------------------------------|
| API Gateway / Auth / RBAC / User preferences    |
| Application services / orchestration            |
|--------------------------------------------------|
| Market       | Decision    | Strategy            |
| Portfolio    | Risk        | Position Mgmt       |
| Research     | ML/Models   | AI Agents           |
| Audit        | Automation  | System              |
+---------------------|----------------------------+
                      |
               Runtime boundary
                      v
+--------------------------------------------------+
|               NautilusTrader                    |
| Instrument model / DataEngine / Cache           |
| ExecutionEngine / Portfolio / Backtest runtime  |
| Runtime-level risk / event model                |
+---------------------|----------------------------+
                      |
               Broker boundary
                      v
+--------------------------------------------------+
| Broker adapters                                   |
| T-Invest first; future IBKR/MOEX/etc.            |
+--------------------------------------------------+
```

## Deployment baseline

The first production topology should be small and explicit:

```text
reverse-proxy
  -> web-ui
  -> application-api
  -> trading-runtime worker
  -> research/ML worker(s)

shared infrastructure:
  PostgreSQL
  Redis (optional cache/ephemeral coordination only)
  object storage / filesystem for model artifacts
  metrics + logs
```

Do not use NATS/Kafka/RabbitMQ by default. Introduce a durable message broker only when a concrete cross-process reliability or throughput requirement proves it necessary.

## Process isolation

The following boundaries are process-worthy from the beginning:

### 1. Application/API process

Owns HTTP/WebSocket API, auth, user/session state, read models and orchestration.

### 2. Trading runtime process

Owns live Nautilus runtime, broker sessions, execution, reconciliation and safety-critical trading state. A UI or ML failure must not crash this process.

### 3. Research/ML workers

CPU/GPU-intensive training, feature generation and bulk historical computations run outside the live trading process. They may scale horizontally.

This is the primary answer to the prior single-core/memory problem: heavy analytical workloads and latency-sensitive trading do not share one interpreter/process/memory lifecycle.

## Modules and responsibilities

### Identity & Access

Authentication, RBAC, sessions, API tokens, user preferences. Never owns broker secrets directly in UI state.

### Broker Gateway

Broker adapter lifecycle, credentials, rate limiting, connectivity, reference-data normalization, environment selection, broker health.

### Market Data

Subscriptions, current state, historical retrieval, normalization, quality flags, stale-data detection.

### Instrument Registry

Canonical instrument identity plus broker aliases/UIDs/FIGIs, contract economics, expiry, trading status and metadata freshness.

### Portfolio

Broker-authoritative positions/balances plus normalized valuation and PnL read models.

### Decision

Combines strategy signals, AI/ML evidence and portfolio context into an explicit `TradeIntent`. Decision does not send orders.

### Strategy

Owns deterministic strategy logic and lifecycle. Strategy output is an intent/signal, not direct broker mutation.

### Advanced Risk

Applies portfolio, strategy, instrument and account constraints before execution. Returns allow/deny/resize plus reasons.

### Position Management

Protection, exits, trailing logic, time stops and lifecycle policies after exposure exists.

### Execution

Converts approved plans into runtime orders and tracks lifecycle through Nautilus/broker reconciliation.

### Research

Historical data, replay, backtests, feature computation, experiment metadata.

### ML / Model Registry

Dataset definitions, training jobs, model metadata, metrics, promotion state and inference interfaces.

### AI Agents

Analysis agents consume immutable snapshots and emit typed evidence. They do not call broker APIs.

### Automation Policy

Defines which strategy/agent is allowed to progress from advisory to automatic execution and with what risk budget.

### Audit / Observability

Durable decision/execution audit trail, correlation IDs, structured logs, metrics and health.

## Data ownership

PostgreSQL is the system of record for application state, configuration, audit metadata, model registry and durable identity mappings.

Broker state remains externally authoritative for:

- open orders;
- fills;
- positions;
- cash/balances;
- venue execution status.

Local projections must reconcile to broker truth.

Redis, if used, is never the only source of durable financial state.

## Concurrency model

Use concurrency deliberately:

- live runtime: event-driven, low-blocking, bounded work per event;
- I/O: async where the underlying client benefits from it;
- CPU-heavy analytics: separate worker process pool;
- ML training: isolated process/container, optional GPU;
- no long CPU work on API/event-loop threads;
- bounded queues with explicit backpressure and drop/fail policy.

## Scaling rule

A module is extracted into a service only if at least one is true:

1. independent scaling is needed;
2. fault isolation materially improves safety;
3. independent deployment cadence is required;
4. a different runtime/language is justified by measured performance;
5. regulatory/security isolation requires it.

Until then it remains an in-process module behind the same interface.

## Language/runtime strategy

- **Python** for application orchestration, strategies, research, ML and integration with NautilusTrader;
- **Rust-backed NautilusTrader internals** provide the performance-sensitive trading runtime foundation;
- frontend uses TypeScript;
- additional Rust components are allowed only when profiling demonstrates a real hot path or when a broker adapter benefits materially from native implementation.

Do not rewrite modules in a lower-level language merely because the previous application consumed too much CPU. First isolate workloads and measure.

## Safety invariants

- no execution without current broker connectivity and reconciled account state;
- no autonomous execution without explicit automation permission;
- no financial inference from missing contract economics;
- no timeout-to-rejection conversion after dispatch;
- no direct AI-to-broker path;
- no direct UI-to-broker path;
- all mutations have correlation/request identities;
- restart must converge from durable local identities plus broker-authoritative reports.

## Architectural decision records

All irreversible or cross-cutting decisions belong in `architecture/adr/`. ADR-0001 accepts NautilusTrader as the trading-runtime foundation.