# ADR-0001 — Accept NautilusTrader as Trader 2.0 trading runtime

Status: **ACCEPTED**

Date: 2026-08-22

## Context

Trader 2.0 requires a production-grade trading runtime with consistent instrument, market-data, execution, recovery, and backtest/live semantics. The previous application accumulated significant complexity while implementing these concerns directly.

NautilusTrader was evaluated as a reusable trading-runtime foundation, with T-Invest/MOEX compatibility treated as a hard qualification gate rather than an assumption.

## Decision

Use NautilusTrader as the core trading runtime for Trader 2.0.

Trader 2.0 will keep its own application, advanced risk, AI/ML, decision, portfolio-construction, orchestration, API, and UI layers. NautilusTrader is not the product UI and is not the complete application architecture.

The T-Invest adapter must preserve broker semantics exactly and remain isolated behind a broker boundary.

## Qualification evidence

### Q1 — Instruments / futures economics

Live T-Invest evidence confirmed:

- SBER/TQBR maps into a Nautilus equity instrument;
- a live SPBFUT commodity future maps into `FuturesContract`;
- T-Invest `TYPE_COMMODITY` maps to `AssetClass.COMMODITY`;
- futures money-per-point is derived exactly from broker metadata as `min_price_increment_amount / min_price_increment`;
- broker tick value maps directly into Nautilus contract multiplier without approximation.

### Q2 — Market data

Live evidence confirmed:

- candles, order book, trading status, and public trades can be normalized without fabricating units or timestamps;
- lot-to-unit semantics are explicit;
- T-Invest MarketDataStream subscriptions restore after a forced disconnect;
- market data continues after reconnect;
- received trade data can be normalized into Nautilus `TradeTick` and passed through `DataEngine`/cache.

### Q3 — Sandbox execution

Live T-Invest Sandbox evidence confirmed:

- market BUY/SELL lifecycle;
- broker order ID and client request/idempotency ID remain distinct;
- authoritative `GetSandboxOrderState` confirms execution;
- portfolio/positions converge after fills;
- resting limit order submission;
- replace with a new idempotency key;
- cancel;
- authoritative final state reports `CANCELLED`;
- no active order remains after cleanup.

### Q4 — Reconciliation / ambiguous outcome

Live T-Invest Sandbox evidence confirmed:

- a fresh process can reconstruct an existing broker-side resting order from persisted identities and broker-authoritative reports without resubmitting the mutation;
- order/account/portfolio state can be recovered after local process restart;
- a simulated response loss after dispatch is represented locally as `UNKNOWN`, not `REJECTED`;
- broker state is then queried and resolves the ambiguity authoritatively;
- no blind retry is required;
- cleanup verifies final broker state.

## Architectural constraints

1. NautilusTrader is a dependency, not a fork.
2. T-Invest broker semantics must not be changed to fit a generic abstraction.
3. A timeout after dispatch must never be treated as broker rejection without broker evidence.
4. Client request IDs, broker order IDs, and exchange IDs remain distinct identities.
5. Broker reconciliation is authoritative for orders, fills, positions, and account state.
6. AI/ML must stay outside the execution safety path.
7. Trader 2.0 advanced risk executes before runtime-level execution safeguards.
8. UI communicates with Trader 2.0 application APIs; it does not bind directly to Nautilus internals.
9. Backtest and live trading must share the same domain strategy logic wherever technically possible.
10. Futures economics must use authoritative broker metadata and fail closed when required metadata is missing or inconsistent.

## Consequences

### Positive

- avoids reimplementing trading-engine primitives already provided by a mature runtime;
- reduces custom order-state, reconciliation, portfolio, and backtest/live infrastructure;
- gives Trader 2.0 a clear boundary between product logic and trading runtime;
- preserves room for multiple brokers and future market-data adapters.

### Costs / risks

- T-Invest adapter remains substantial engineering work;
- broker-specific futures semantics and market-data quirks must be maintained explicitly;
- Nautilus upgrades require compatibility qualification;
- advanced risk and product orchestration remain Trader 2.0 responsibilities.

## Rejection criteria going forward

This ADR must be revisited if production implementation proves any of the following unavoidable:

- broker-authoritative reconciliation cannot converge reliably;
- post-dispatch uncertainty must be misrepresented as rejection/failure;
- futures economics require approximation;
- live and backtest business logic diverge materially because of runtime constraints;
- Nautilus imposes a fundamental throughput, latency, or resource limit that cannot be mitigated within the target deployment model.
