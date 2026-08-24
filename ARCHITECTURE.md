# Vox Trader — Architecture

## Architectural style

Vox Trader starts as a **modular monolith with isolated runtimes and replaceable adapters**, not as a traditional all-in-one monolith and not as a microservice fleet.

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
|                 Vox Core (Rust)                  |
|--------------------------------------------------|
| API / Auth / RBAC / User preferences            |
| Application services / orchestration            |
|--------------------------------------------------|
| Market       | Decision    | Strategy            |
| Portfolio    | Risk        | Position Mgmt       |
| Audit        | Automation  | System              |
| Broker Gateway / persistence / read models       |
+---------------------|----------------------------+
                      |
               Runtime boundary
                      v
+--------------------------------------------------+
|       NautilusTrader Rust runtime / crates       |
| Instrument model / Data / Execution / Portfolio |
| Backtest/live runtime / runtime-level risk       |
+---------------------|----------------------------+
                      |
               Broker boundary
                      v
+--------------------------------------------------+
| Broker adapters                                  |
| T-Invest first; future IBKR/MOEX/etc.            |
+--------------------------------------------------+

Separate computational plane:

+--------------------------------------------------+
| Vox Research / ML / AI workers (Python)          |
| training / inference / datasets / notebooks      |
+--------------------------------------------------+
```

## Deployment baseline

The first production topology should be small and explicit:

```text
reverse-proxy
  -> vox-web
  -> vox-core (Rust)
  -> vox-ml-worker(s) (Python, only when needed)

shared infrastructure:
  PostgreSQL
  Redis (optional cache/ephemeral coordination only)
  object storage / filesystem for model artifacts
  metrics + logs
```

Do not use NATS/Kafka/RabbitMQ by default. Introduce a durable message broker only when a concrete cross-process reliability or throughput requirement proves it necessary.

## Process isolation

### 1. Vox Core process

Owns HTTP/WebSocket API, auth, broker sessions, market data, execution, reconciliation, safety-critical runtime state, application orchestration and durable broker/application projections.

The codebase is internally modular. A module is split into a process only when fault isolation or scaling justifies the operational cost.

### 2. Web frontend

TypeScript frontend is independently deployable/static and communicates only with Vox Core APIs.

### 3. Research/ML workers

CPU/GPU-intensive training, feature generation, model inference where appropriate and bulk historical computations run outside Vox Core. They may scale horizontally.

A Python crash, CUDA OOM or model failure must not terminate or stall live trading.

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

Historical data, replay, backtests, feature computation, experiment metadata. Heavy/offline computation may be delegated to Python workers.

### ML / Model Registry

Dataset definitions, training jobs, model metadata, metrics, promotion state and inference interfaces. Python implementations are isolated behind explicit contracts.

### AI Agents

Analysis agents consume immutable snapshots and emit typed evidence. They do not call broker APIs.

### Automation Policy

Defines which strategy/agent is allowed to progress from advisory to automatic execution and with what risk budget.

### Audit / Observability

Durable decision/execution audit trail, correlation IDs, structured logs, metrics and health.

## T-Invest -> Vox -> Nautilus data ownership

All T-Invest data enters through the Vox T-Invest adapter and is normalized into typed Vox provider/domain representations first.

```text
T-Invest
   |
   v
Vox typed adapter
   |---------------------> Vox storage / read models / API / analytics / ML
   |
   +---------------------> Nautilus mapper/runtime (only faithful runtime data)
```

Nautilus is not the owner of all broker data. News, fundamentals, reports, issuer metadata, capability state and similar provider information remain Vox data. Trading instruments, market events, execution/account reports and other faithful runtime concepts are also mapped into Nautilus where required.

Do not implement a second competing order/position engine in Vox. Vox keeps durable identities, broker evidence, audit/reconciliation state and product read models; Nautilus owns the in-runtime trading-engine lifecycle where its semantics are canonical.

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

Rust/Tokio is the baseline for Vox Core asynchronous I/O and concurrency.

Rules:

- live runtime: event-driven, low-blocking, bounded work per event;
- async tasks must be cancellation-safe and supervised;
- no unbounded channels/queues;
- CPU-heavy analytics never execute on latency-sensitive async tasks;
- ML training/inference with material CPU/GPU load uses isolated Python workers/processes;
- explicit backpressure/drop/fail policy for every high-volume stream;
- no idle busy-loop polling.

## Scaling rule

A module is extracted into a service only if at least one is true:

1. independent scaling is needed;
2. fault isolation materially improves safety;
3. independent deployment cadence is required;
4. a different runtime/language is justified by measured or ecosystem requirements;
5. regulatory/security isolation requires it.

Until then it remains an in-process module behind the same interface.

## Language/runtime strategy

- **Rust** is the primary language for Vox Core: application backend, broker adapters, market data, execution, reconciliation, risk/runtime, orchestration and persistence integration;
- **NautilusTrader pure Rust/v2** is the preferred trading-runtime integration path where required capabilities are available and qualified;
- **TypeScript** is used for the web frontend;
- **Python** is restricted to isolated research/ML/AI workers and experimentation where its ecosystem materially helps;
- **Go is not part of the baseline stack**. It may be introduced only for a later independently justified service.

The existing Python production seed under `src/trader2/` is transitional and must not receive new product functionality. It is replaced by the Rust foundation under ADR-0002.

Current Rust dependency direction is one-way:

```text
vox-core -> vox-tinvest -> vox-domain
         -> vox-nautilus -> vox-domain
                         -> nautilus-model
```

`vox-domain` contains no provider or Nautilus dependency. `vox-tinvest` owns wire DTOs and never exports raw provider JSON. `vox-nautilus` is the only mapping boundary allowed to construct Nautilus instrument types from Vox representations.

## Precision policy

Financial economics must never depend on binary floating-point approximation.

- use exact fixed-point/decimal representations for broker money, price, quantity and contract economics;
- enable/qualify Nautilus high-precision support where the domain requires it;
- preserve broker integer/nano semantics exactly at the adapter boundary;
- fail closed if a critical conversion cannot be represented faithfully.

## Safety invariants

- no execution without current broker connectivity and reconciled account state;
- no autonomous execution without explicit automation permission;
- no financial inference from missing contract economics;
- no timeout-to-rejection conversion after dispatch;
- no direct AI-to-broker path;
- no direct UI-to-broker path;
- all mutations have correlation/request identities;
- restart must converge from durable local identities plus broker-authoritative reports;
- live mutations are disabled by default;
- Python worker availability is never a prerequisite for safe core runtime recovery.

## Architectural decision records

Cross-cutting decisions belong in `architecture/adr/`.

- ADR-0001 accepts NautilusTrader as the trading-runtime foundation.
- ADR-0002 makes Vox Trader Rust-first and moves Python outside the capital-critical core.
