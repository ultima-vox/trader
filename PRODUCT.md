# Trader 2.0 — Product baseline

## Purpose

Trader 2.0 is a production-grade client-server platform for discretionary, semi-automated and autonomous trading with a primary focus on exchange-traded instruments and derivatives. The system combines broker connectivity, real-time market data, portfolio/risk control, strategy execution, backtesting, ML/AI analysis and an operator-grade web terminal.

The product is not an MVP and must not optimize for demo speed at the cost of operational safety, recoverability or architectural clarity.

## Product principles

1. **Broker truth is authoritative.** Orders, fills, positions and balances converge to broker-reported state.
2. **Unknown is a first-class state.** A transport failure after dispatch is not a broker rejection.
3. **AI is advisory until explicitly authorized.** AI/ML cannot bypass risk, permissions or execution controls.
4. **Live and backtest logic converge.** Strategy/domain logic is shared wherever venue mechanics allow.
5. **No UI-driven architecture.** UI consumes stable application contracts; backend internals do not leak into screens.
6. **No premature microservices.** Modular boundaries are designed first; processes are split only for scaling, isolation or fault-containment reasons.
7. **Fail closed on financial ambiguity.** Missing contract economics, stale reference data, broken reconciliation or invalid risk state blocks trading.
8. **Observable by default.** Every critical decision and mutation is traceable by correlation IDs and durable audit events.

## Primary capabilities

- multi-broker architecture, T-Invest first;
- real-time quotes, trades, candles, order books and instrument status;
- shares and futures first, extensible to options and other asset classes;
- manual, strategy-driven and autonomous order flows;
- portfolio state and PnL;
- advanced risk and exposure controls;
- position protection and lifecycle management;
- strategy registry and execution;
- historical data, replay and backtesting;
- ML datasets, training, inference and model registry;
- AI analysis agents and decision support;
- configurable automation permissions;
- operator web terminal;
- sandbox/paper/live environments;
- audit, diagnostics, health and recovery tooling.

## Non-goals for the first production baseline

- direct exchange membership / colocated HFT;
- sub-millisecond latency guarantees;
- fully autonomous capital allocation without explicit risk/permission policy;
- broker-specific assumptions inside product/domain layers;
- a distributed microservice fleet by default.

## Operator roles

### Trader

Views market state, creates/manual orders, supervises strategies and positions.

### Risk operator

Configures exposure, loss, concentration and automation limits; can halt trading.

### Administrator

Manages users, broker credentials, environments, system health and updates.

### Analyst / ML operator

Builds datasets, trains/evaluates models and publishes approved model versions.

## Product modes

- **Research** — historical data, analytics, backtests, model training.
- **Sandbox/Paper** — full application path with non-production execution.
- **Live supervised** — manual/semi-automated trading with operator oversight.
- **Live autonomous** — only for strategies explicitly granted automation permissions and risk budgets.

## Acceptance philosophy

A capability is not complete when a button or endpoint exists. It is complete when the full state transition, failure path, reconciliation path, observability and operator feedback are implemented and tested.