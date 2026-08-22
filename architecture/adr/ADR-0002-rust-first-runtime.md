# ADR-0002 — Rust-first core for Vox Trader

Status: **ACCEPTED**

Date: 2026-08-22

## Context

Vox Trader is still at the beginning of production implementation. The current foundation proved T-Invest/Nautilus compatibility, but the application/runtime code was initially started in Python around Nautilus v1 bindings.

The product target is a long-lived, resource-efficient, highly concurrent trading platform. The live trading path must remain isolated from ML/research workloads, avoid interpreter-level contention, keep memory behavior bounded, and preserve strict financial-state semantics.

NautilusTrader is Rust-native and now exposes a pure-Rust v2 path capable of actors, strategies, backtests and live trading without a Python runtime. The Rust API is still evolving, so Nautilus upgrades remain a qualified dependency change rather than an automatic upgrade.

## Decision

Vox Trader adopts a **Rust-first application and trading-runtime core**.

The target language/runtime split is:

- **Rust** — Vox Core: application backend, broker adapters, market-data handling, execution, reconciliation, portfolio/risk runtime, orchestration, persistence integration and Nautilus runtime integration;
- **TypeScript** — web UI;
- **Python** — isolated research/ML/AI workers only where the Python ecosystem provides material value.

Go is not part of the baseline architecture. It may be introduced later only for a demonstrated independent service need that Rust does not satisfy economically.

## Process model

Initial deployment remains small:

```text
vox-core          Rust
vox-web           TypeScript static/frontend assets
vox-ml-worker     Python, started/scaled when needed
PostgreSQL
optional Redis/object storage/metrics infrastructure
```

Vox Core may be internally modular and may later split selected modules into processes, but process extraction is driven by fault isolation/scaling requirements, not by language boundaries alone.

## Nautilus boundary

NautilusTrader remains the accepted trading-runtime dependency from ADR-0001, but integration moves to the pure-Rust/v2 path where the required capability is available and qualified.

T-Invest data enters Vox Trader through the Vox T-Invest adapter first. Vox owns the provider-normalized DTO/event representation and then maps only faithful trading-runtime data into Nautilus.

```text
T-Invest
   |
   v
Vox typed provider adapter
   |---------------------> Vox storage/read models/API/analytics
   |
   +---------------------> Nautilus mapper/runtime
```

Nautilus is not the system-of-record for all provider data and does not receive data that has no faithful trading-runtime representation.

## Python policy

Python is explicitly removed from the live capital-critical core.

Python is allowed for:

- ML training and inference workers;
- notebooks/research;
- experimental analytics;
- AI integrations where isolation from live trading is maintained.

Python workers communicate with Vox Core through explicit IPC/API/job contracts. A Python crash, CUDA OOM or model failure must not terminate the trading runtime.

## Migration policy

The existing Python production seed in `src/trader2/` is transitional and must not be expanded. It remains as historical/qualification evidence until the Rust foundation replaces it safely.

A dedicated migration issue must establish the Rust workspace, Nautilus v2 dependency strategy, exact numeric policy, async/runtime conventions, T-Invest transport seed, tests and CI before feature issue #7 continues.

No new broker/reference/market/execution feature implementation should be added to the Python production package after this ADR.

## Precision and safety

- broker economics remain exact/fixed-point/Decimal-equivalent; no binary-float approximation for financial values;
- use Nautilus high-precision mode where required and explicitly qualified;
- all async queues are bounded;
- no busy-loop polling;
- broker mutation ambiguity remains `UNKNOWN` until authoritative reconciliation;
- live mutations remain fail-closed by default;
- broker, client and exchange identities remain distinct.

## Consequences

### Positive

- lower runtime overhead and more predictable memory/CPU behavior;
- strong compile-time ownership/concurrency guarantees for capital-critical code;
- direct integration with Nautilus Rust crates without a Python boundary;
- one primary backend language instead of Rust + Go duplication;
- Python ecosystem remains available without coupling ML failures to live trading.

### Costs / risks

- Nautilus pure-Rust/v2 API is still under active development and may introduce breaking changes;
- Rust development has a higher implementation/compile-time complexity than Python;
- some high-level Nautilus v1 functionality may need temporary compatibility work or explicit deferral until v2 parity exists;
- current Python production seed must be migrated rather than incrementally extended.

## Acceptance gate

Before resuming broker feature development, the Rust foundation must prove:

1. reproducible Cargo workspace/build/test flow;
2. successful consumption of the required Nautilus Rust crates/version;
3. exact instrument/futures economics equivalent to Q1;
4. T-Invest REST + WebSocket connectivity in Rust;
5. async reconnect/subscription skeleton without unbounded queues;
6. live mutation fail-closed configuration;
7. no Python runtime dependency for Vox Core;
8. documented fallback for any Nautilus v2 capability gap discovered during migration.
