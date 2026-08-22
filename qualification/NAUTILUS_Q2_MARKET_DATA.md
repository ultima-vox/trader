# Q2 — T-Invest market-data semantics on NautilusTrader

Status: **IN PROGRESS**

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
| last price | explicit last-price representation | do not fabricate bid/ask from last price |
| trade | `TradeTick` | broker quantity is in lots; convert using authoritative lot size |
| order book | book snapshot/deltas supported by the chosen client | preserve bid/ask side, depth and consistency state |
| candle | `Bar` | preserve exchange timestamp, interval, completion and lot-volume semantics |
| trading status | `InstrumentStatus` or explicit adapter status event | preserve unavailable/unknown state explicitly |

## Important broker semantics

T-Invest states that market-data prices are per instrument, while order-book, trade and candle volumes are expressed in lots. Futures prices are expressed in points.

The stream exposes subscription responses plus candles, trades, order books, trading status, last-price updates and ping messages. The order-book payload includes an `is_consistent` flag; an inconsistent broker book must not be published as an authoritative clean book.

### Public trade identifier constraint

The public T-Invest `Trade` message exposes instrument identity, direction, price, quantity, time and trade source, but does not expose a venue trade identifier. Nautilus `TradeTick` requires a `TradeId`.

The PoC therefore derives an adapter compatibility ID deterministically from the fields T-Invest actually exposes. This follows the same general compatibility pattern used by adapters/data sources that lack a native trade ID, but it is not represented as a broker-supplied ID.

Known limitation: if T-Invest emits two distinct public trades whose complete exposed field tuple is identical, the provider contract gives the adapter no authoritative way to distinguish them. This remains an explicit provider limitation and must be evaluated during live stream testing rather than hidden.

## Timestamp contract

Where the broker provides an exchange/event timestamp and the adapter has a local receive/init time, keep them distinct:

- `ts_event` = broker/exchange event time;
- `ts_init` = local adapter/runtime receive/init time according to the Nautilus event contract.

No wall-clock timestamp may replace a broker event timestamp merely because a field is missing. Missing event time must fail closed or remain explicitly unavailable.

## Quantity contract

For market data:

```text
T-Invest lots * authoritative instrument lot size = Nautilus instrument units
```

This rule is implemented for trades, candles and order-book levels in `qualification/poc/q2_market_data.py` and covered by synthetic tests.

## Order-book contract

- do not return fabricated empty depth;
- do not silently treat `is_consistent=false` as a valid clean snapshot;
- preserve depth and side semantics;
- reconnect must rebuild book state before normal updates are accepted;
- duplicate/out-of-order handling must be idempotent where the T-Invest message contract makes this possible.

## Q2a — unary semantic probe

Before introducing stream/reconnect complexity, run a live unary probe to prove the raw field/unit mappings against T-Invest:

```bash
python -m qualification.live.q2_snapshot
```

It validates on live broker data:

- SBER instrument identity and lot size from Q1;
- a completed exchange candle -> Nautilus `Bar`;
- candle volume lots -> instrument units;
- `GetOrderBook` prices/quantities and event timestamp;
- non-empty depth without fabrication;
- `GetTradingStatus`;
- a public trade -> Nautilus `TradeTick` when a trade exists in the last hour;
- deterministic adapter trade ID when T-Invest provides no native trade ID.

The trade leg may legitimately be unavailable outside an active market session; that does not complete Q2.

## Q2b — stream/DataEngine qualification

After Q2a passes, implement the real adapter path and use Nautilus `DataTester` acceptance cases. Nautilus documents `DataTester` as the adapter acceptance harness and considers instruments/raw book/quotes/trades baseline data groups.

Required sequence:

1. InstrumentProvider from Q1;
2. T-Invest market-data stream transport;
3. adapter-owned desired subscription registry;
4. subscription acknowledgement validation;
5. trades;
6. order-book state;
7. candles;
8. trading status;
9. last price without pretending it is a two-sided quote;
10. forced disconnect;
11. bounded reconnect;
12. resubscribe;
13. rebuild authoritative book state;
14. verify events reach Nautilus `DataEngine` and test actor.

## Reconnect contract

The adapter must own desired subscription state. After a stream failure it must:

1. reconnect with bounded backoff;
2. re-authenticate;
3. restore requested subscriptions;
4. validate subscription acknowledgements;
5. re-establish book state before declaring depth authoritative;
6. resume event emission without manual operator action.

T-Invest `GetMySubscriptions` may be used as evidence/diagnostics but must not replace adapter-owned desired state.

## Current verdict

**NOT YET QUALIFIED.** Q1 live instrument mapping has passed. Q2 normalization primitives and the unary live probe now exist, but no T-Invest stream has yet been passed through Nautilus `DataEngine`, and forced reconnect/resubscription evidence is still required.
