# T-Invest market-data adapter

Status: implementation contract for Issue #8.

## Wire ownership

- Source: official `invest-contracts` revision `762e720e27164213f41cac0b226c5698c2ae8199`.
- Vendored contract: `crates/vox-tinvest/proto/tinkoff/marketdata.proto`.
- `prost`/`tonic-build` generate all provider messages and both service clients. Handwritten wire DTOs are forbidden.
- gRPC is production transport. REST/OpenAPI is compatibility evidence only.
- TLS always verifies against host native CA roots. No insecure mode exists.

Machine inventory: `qualification/tinvest_market_data_contracts.json`. Contract tests compare all 9 unary and 2 streaming RPCs plus every request/response `oneof` branch.

## Boundary

```text
generated T-Invest protobuf
  -> vox-tinvest exact normalization / validation / stream supervision
  -> broker-neutral market specs
  -> vox-nautilus checked mapping
  -> Nautilus DataEngine
```

Generated provider values never become fake economics. Missing protobuf messages remain `None`; consumers that require price/time/economics reject absence. Unknown enum wire numbers remain `i32` provider facts until policy explicitly recognizes them.

Prices use `vox_domain::FixedPoint`. Trade, candle, and order-book quantities remain lots inside `vox-tinvest`; Nautilus mapping multiplies by authoritative reference-data lot size with checked integer arithmetic. Last price stays last price and is never fabricated into bid/ask.

## History

`plan_candle_history` models every official `CandleInterval` range and `limit`. Long ranges split into adjacent bounded windows. `merge_historic_candles` sorts results, collapses exact shared-boundary duplicates, and fails on conflicting candles at one timestamp. No range is silently truncated. Historical response lacks identity, so caller must attach the authoritative requested instrument UID explicitly through `CanonicalCandle::from_historic`.

## Streaming

`MarketDataStreamSupervisor` uses generated bidirectional gRPC. Server-side streaming remains exposed as generated compatibility surface, but cannot support runtime subscription changes or `GetMySubscriptions`.

- bounded outbound and event queues provide backpressure;
- adapter-owned desired registry survives disconnects;
- ACK order is arbitrary and data may arrive before ACK;
- reconnect resets ACK and book-authority state, then replays desired subscriptions;
- delays use bounded exponential backoff; zero-delay busy loops are rejected;
- native ping setting accepts only official 5,000–180,000 ms range; stale timeout forces reconnect;
- official combined 300 limit applies only to candle, order-book, and public-trade subscriptions; Info and LastPrice are unlimited;
- startup rejects plans requiring more than official 100 subscription requests/minute;
- `is_consistent=false` remains visible but cannot become authoritative Nautilus book state;
- reconnect requires a new consistent snapshot before book authority returns;
- duplicate and older timestamped events are dropped with observable `Dropped` events. Distinct public trades at the same timestamp remain accepted by full protobuf fingerprint.

T-Invest public `Trade` has no venue trade ID or sequence. `derived_trade_compatibility_id` hashes the full exposed tuple into `TI-<fnv128>`; prefix proves adapter provenance. Identical provider tuples cannot be distinguished. Book sequence maps to Nautilus sentinel `0`; no broker sequence is fabricated.

## Runtime config

`MarketDataSupervisorConfig` controls bounded capacities, stale timeout, reconnect delays/attempts, and ping delay. Defaults:

- outbound queue: 64;
- event queue: 1024;
- stale timeout: 150 seconds;
- reconnect: 250 ms initial, 15 seconds maximum, 8 consecutive failed connects;
- provider ping: 120 seconds.

Credential stays only in `SecretToken`; logs/debug redact it. `GrpcConfig` keeps native-root certificate verification and request/message limits.

## Qualification order

1. generated contract inventory tests;
2. normalization, constraints, ACK/state, reconnect, dedup, book consistency unit tests;
3. Nautilus exact mapping tests;
4. workspace `fmt`, `clippy`, and tests;
5. one credentialed live qualification covering all unary methods plus stream/reconnect.

Missing credentials or closed-market data is reported as missing evidence, never as success.

Live command:

```bash
cargo test -p vox-tinvest --test market_data_live -- --ignored --nocapture
```

Required environment: `TINVEST_TOKEN`. Optional `TINVEST_MARKET_DATA_UID` overrides default SBER
UID. Test is read-only: all 9 unary methods plus five bidirectional subscription ACK families.
