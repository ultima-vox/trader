# Trader 2.0

Trader 2.0 is a new-generation AI-assisted trading platform built around a qualified upstream trading runtime rather than a bespoke execution engine.

## Current status

**Architecture qualification only. No product implementation has started.**

The first decision is whether NautilusTrader can be used as the trading runtime for T-Invest/MOEX without distorting broker, futures, market-data, execution, reconciliation, or risk semantics.

## Preferred target architecture

- NautilusTrader as upstream trading runtime (dependency, not fork)
- Rust-native T-Invest adapter
- Python intelligence / ML layer
- React + TypeScript frontend
- Trader-owned advanced risk layer before runtime execution risk
- shared backtest/live domain logic
- deterministic safety path; AI/LLM never has direct order authority

## Qualification sequence

1. Q1 — T-Invest instrument semantics
2. Q2 — T-Invest market-data semantics
3. Q3 — execution and order lifecycle
4. Q4 — reconciliation, unknown outcomes, crash recovery
5. Q5 — futures PnL/margin/economic parity
6. Q6 — performance and resource qualification
7. Architecture Decision Record: accept or reject NautilusTrader

Product implementation starts only after the runtime passes qualification.

## Legacy reference

The previous `ultima-vox/ai-trader` repository remains a reference source for proven domain requirements, tests, broker semantics, safety rules, and UX lessons. Source code is not to be copied wholesale into Trader 2.0.
