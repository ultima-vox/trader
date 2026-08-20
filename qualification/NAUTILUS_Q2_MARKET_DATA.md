# Q2 — T-Invest market-data semantics on NautilusTrader

Status: **RESEARCHED / IMPLEMENTATION PENDING**

## Target path

```text
T-Invest MarketDataStream
  -> adapter parser
  -> Nautilus normalized event
  -> DataEngine
  -> test Actor/DataTester
```

## Required mappings

| T-Invest surface | Nautilus target | Qualification rule |
| --- | --- | --- |
| last price | quote/last-price representation chosen explicitly | do not fabricate bid/ask from last price |
| trade | `TradeTick` | broker quantity is in lots; convert using authoritative lot size |
| order book | book snapshot/deltas supported by the chosen client | preserve bid/ask side, depth and consistency state |
| candle | `Bar` | preserve exchange timestamp, interval and completion semantics |
| trading status | `InstrumentStatus` | preserve unavailable/unknown state explicitly |

## Important broker semantics

T-Invest states that market-data prices are per instrument, while order-book and trade quantities are expressed in lots. Futures prices are expressed in points.

The stream exposes subscription responses plus candles, trades, order books, trading status, last-price updates and ping messages. The order-book payload includes an `is_consistent` flag; an inconsistent broker book must not be published as an authoritative clean book.

## Reconnect contract

The adapter must own desired subscription state. After a stream failure it must:

1. reconnect with bounded backoff;
2. re-authenticate;
3. restore requested subscriptions;
4. validate subscription acknowledgements;
5. re-establish book state before declaring depth authoritative;
6. resume event emission without manual operator action.

T-Invest provides `GetMySubscriptions`; it can be used as evidence/diagnostics but must not replace adapter-owned desired state.

## Timestamp contract

Where the broker provides both an exchange/event timestamp and local receive time, keep them distinct:

- `ts_event` = broker/exchange event time;
- `ts_init` = adapter/runtime initialization or receive time according to Nautilus event contract.

No wall-clock timestamp may replace a broker event timestamp merely because a field is missing. Missing event time must be represented/documented explicitly.

## Order-book contract

- do not return fabricated empty depth;
- do not silently treat `is_consistent=false` as a valid snapshot;
- preserve depth and side semantics;
- reconnect must rebuild book state before normal updates are accepted;
- duplicate/out-of-order handling must be idempotent where the T-Invest message contract makes this possible.

## Current implementation plan

Use NautilusTrader adapter guidance and its `DataTester` acceptance matrix. Initial implementation order:

1. InstrumentProvider from Q1;
2. market-data stream transport;
3. subscription-state registry;
4. trades;
5. order-book snapshots/updates;
6. candles;
7. trading status;
8. last price without pretending it is a two-sided quote;
9. forced reconnect test.

## Current verdict

**NOT YET QUALIFIED.** Research shows no obvious model mismatch, but no T-Invest stream has yet been passed through Nautilus `DataEngine` in this repository.
