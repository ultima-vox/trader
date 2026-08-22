# Trader 2.0

Trader 2.0 is an AI-assisted trading platform built around a qualified upstream trading runtime rather than a bespoke execution engine.

## Current status

**Runtime qualification complete. Architecture baseline in progress.**

NautilusTrader has been accepted as the Trader 2.0 trading-runtime foundation after live T-Invest qualification of instruments/futures economics, market data/reconnect, sandbox execution, reconciliation, restart recovery and UNKNOWN post-dispatch semantics.

See `architecture/adr/ADR-0001-nautilus-runtime.md`.

## Architecture direction

- NautilusTrader as upstream trading runtime (dependency, not fork);
- modular-monolith application with explicit bounded modules;
- isolated live trading runtime process;
- isolated research/ML workers for CPU/GPU-heavy work;
- T-Invest broker adapter first;
- Python for orchestration/strategy/research/ML;
- TypeScript frontend;
- Trader-owned advanced risk before runtime execution safeguards;
- shared backtest/live strategy-domain logic;
- deterministic execution safety path; AI/LLM has no direct broker authority;
- broker-authoritative reconciliation.

## Baseline documents

- `PRODUCT.md` — product scope and principles;
- `ARCHITECTURE.md` — modules, processes, deployment and scaling rules;
- `DOMAIN_MODEL.md` — canonical financial/application domain objects;
- `TRADING_PIPELINE.md` — signal-to-execution lifecycle;
- `RISK_MODEL.md` — advanced risk and reservation model;
- `BROKER_ADAPTER_SPEC.md` — broker boundary contract;
- `DESIGN_SYSTEM.md` — operator terminal UX/design rules;
- `IMPLEMENTATION_PLAN.md` — staged development sequence.

## Qualification

Qualification harnesses remain under `qualification/` as regression evidence for adapter/runtime compatibility.

## Legacy reference

The previous `ultima-vox/ai-trader` repository remains a reference source for proven domain requirements, tests, broker semantics, safety rules and UX lessons. Source code is not copied wholesale into Trader 2.0.