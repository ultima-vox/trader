# ADR-0004 — Share strategy-domain logic between backtest and live

Status: **ACCEPTED**

Date: 2026-08-22

## Decision

Backtest, replay, sandbox and live trading use the same strategy-domain implementation. Runtime/data/execution adapters may differ, but strategy business logic must not fork into separate live and backtest versions.

## Rationale

Divergent strategy implementations invalidate research results and create defects that appear only in production.

## Constraints

- venue-specific mechanics are modeled through execution/data adapters;
- deterministic clocks/data inputs are injected for research;
- strategy code does not directly call external brokers;
- any unavoidable mode-specific behavior must be explicit and covered by parity tests.

## Consequences

Research results remain materially comparable to production behavior and strategy maintenance is simplified.