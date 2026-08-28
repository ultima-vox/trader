# Vox Application API — architecture note

Issue #38 requires the transport choice to be verified against current official documentation
before implementation, not from memory. This note records what was read, when, and what it
decided.

## Sources consulted

| Source | Version found | What it decided |
| --- | --- | --- |
| [axum](https://docs.rs/axum/latest/axum/) | **0.8.9** | `Router::new().route(...)`, shared state via `State` + `.with_state()`, served with `axum::serve(listener, app)` over a `tokio::net::TcpListener`. WebSocket support lives in `axum::extract::ws` behind the `ws` feature. |
| [axum WebSocket](https://docs.rs/axum/latest/axum/extract/ws/) | 0.8.9 | `WebSocketUpgrade` extractor, `ws.on_upgrade(handler)`, messages as `Message::Text(Utf8Bytes)`. |
| [utoipa](https://docs.rs/utoipa/latest/utoipa/) | **5.5.0** | `ToSchema` derive, `#[utoipa::path]`, `#[derive(OpenApi)]` with `paths(..)`/`components(schemas(..))`, **OpenAPI 3.1**, document via `ApiDoc::openapi()`. |
| [OpenAPI Specification](https://spec.openapis.org/oas/latest.html) | 3.1 | The generated document targets 3.1; `oneOf` carries the tagged unions of the protection lifecycle and the stream envelopes. |
| [RFC 9110 — HTTP Semantics](https://www.rfc-editor.org/rfc/rfc9110) | — | Status mapping in `error.rs`: 400 validation, 401 authentication, 403 permission, 404 not found, 409 conflict and stale scope/epoch, 202 for an accepted-but-unresolved mutation *receipt*, 503 for a capability with no owner and for transient dependency failure. `UNKNOWN_AFTER_DISPATCH` is never an `ApiError`. |
| [RFC 6455 — WebSocket](https://www.rfc-editor.org/rfc/rfc6455) | — | One upgrade endpoint, text frames carrying JSON envelopes, server-initiated heartbeats, close on a slow consumer. |
| [T-Invest developer protocols](https://developer.tbank.ru/invest/intro/developer/protocols/) | — | Confirms the provider speaks gRPC/protobuf. That stays inside the adapter: no provider wire type appears in this API, and the browser never reaches the provider. |
| [T-Invest GetCandles](https://developer.tbank.ru/invest/api/market-data-service-get-candles) | 2026-08-27 | Historic `CandleInterval` is 1..=16 including 5s/10s/30s. |
| [T-Invest marketdata proto](https://developer.tbank.ru/invest/services/quotes/marketdata) | pinned `#8` proto | MarketDataStream `SubscriptionInterval` is 1..=13 (no 5s/10s/30s). Provenance stays split. |

Read on 2026-08-27. Versions are what `docs.rs/latest` served that day; the workspace pins
`axum = "0.8"` and `utoipa = "5.5"`.

## Shape

```text
browser / future native clients
        │  HTTPS REST/JSON        (commands, snapshots, history)
        │  WebSocket              (live subscriptions)
        ▼
vox-api          transport + public contracts + OpenAPI
        ▼        application ports (traits)
vox-runtime / vox-domain / future #21,#22,#23...
        ▼
provider adapters (T-Invest today)
```

`vox-core` composes the process and starts the server. `vox-api` owns no business rule: a
handler takes a typed request, calls a port and shapes a typed response.

## Decisions and why

**HTTP + WebSocket, not gRPC-web.** REST/JSON is a stable browser boundary that OpenAPI can
describe, and axum supports both halves in one router. Provider gRPC stays internal.

**Ports, not a broker client.** `RuntimeQueries`, `AccountQueries` and `ExecutionCommands`
are traits. A deployment attaches what it has. #11 has landed: `ProcessRuntime` maps
accepted `RuntimeHealth`. Account reads require an `AccountBindingResolver` that maps
canonical `account_id` + `broker_connection_id` to `broker_account_id` before a
`RuntimeScope` is built. Matching strings are never a binding. Until #17 persists those
bindings, the server attaches runtime health only and account routes answer
`CAPABILITY_UNAVAILABLE` naming `#17`. Execution stays gated by #10.

**Connection identity.** Public `broker_connection_id` is the same opaque application
identity as `RuntimeScope.connection_ref`. The only conversion is
`connection_ref_from_broker_connection_id`, which validates through `OpaqueRef`.

**Money is a string.** `Decimal` renders `FixedPoint` at nano scale as a fixed nine-decimal
string. A JSON number would silently lose precision in a JavaScript client, so the type
system forbids one: there is no numeric money type in the public schema.

**Environment and trading mode are separate axes.** `BrokerEnvironment` is `SANDBOX` or
`PRODUCTION`, exactly what a broker connection has. `TradingMode` has one value, `LIVE`.
`PAPER` and `BACKTEST` are trading modes owned by #23/#29 and are absent until those
runtimes exist; the server refuses to start in `Environment::Paper` rather than serve a
scope the contracts cannot express.

**Execution target identity is canonical Vox identity.** Public `ExecutionScope` is
`provider`, `environment`, `broker_connection_id`, `account_id`, `trading_mode`. Provider
broker-account identifiers remain read-side metadata. The scope key includes the connection
so two connections exposing the same account cannot collide. Public order commands name an
opaque `instrument_id`; provider UID/FIGI stay inside adapters. Market and account read
models may still carry `instrument_uid` as broker-fact metadata because they are already
inside a provider-named scope.

**Instrument catalogue identity remains the domain's.** `InstrumentIdentityDto` still
publishes `vox-domain::InstrumentIdentity` (`provider` + `uid`, with FIGI/ticker/class code
as aliases). That is lookup identity, not the capital-command target.

**Market data is a projection, not a second broker client.** `SnapshotMarketProjection`
stores facts already acquired by the #8 adapter and republishes them provider-neutrally:
quote, order book, tape, candles, session and the instrument catalogue. Mapping functions
(`quote_from_last_price`, `order_book_from_levels`, `trade_from_canonical`,
`trading_status_from_provider`, `candle_interval_from_provider`) take #8 field shapes
(`FixedPoint`, uid, historic `CandleInterval` wire numbers, wire status) without importing
protobuf. Candle intervals are the official GetCandles set accepted by
`vox_tinvest::market_data::candle_request_constraint` (1..=16), including **5s/10s/30s**.
Those second-resolution bars are request/history only: MarketDataStream
`SubscriptionInterval` stops at month (1..=13) and is not treated as the same enum.
`GET /api/v1/market/candle-intervals` returns `CandleIntervalCapability` so a client can
tell historic-only from stream-capable without guessing. Unknown provider integers are
`UNSUPPORTED_CANDLE_INTERVAL`, never a silent drop.
`CandleDto.state` is `OPEN` / `CLOSED` / `CORRECTED` with a per-bar `revision: u64`; a boolean
`closed` flag is not enough. Every record carries `MarketFreshness`. An inconsistent book
is refused. A missing instrument is `MARKET_FACT_NOT_FOUND`, never a zero price. Default
`vox-server` does not attach an empty projection, so unattached `MARKET_DATA` stays
`CAPABILITY_UNAVAILABLE`.

**Live WebSocket is an application event bus.** `ApplicationEventBus` is a bounded
broadcast of already-projected facts. `SnapshotMarketProjection` publish methods emit
quote/book/tape events; a runtime-health watcher publishes `#11` health diffs after the
first snapshot baseline. The `/api/v1/stream` gateway fans matching events to per-socket
bounded queues as `UPDATE` with a monotonic per-subscription sequence. Each socket has its
own writer task, so a slow consumer cannot block another client or upstream publish.
A lagging subscriber is `DROPPED_SLOW_CONSUMER`, not buffered without limit. Account topics stay
explicitly unavailable until #17 attaches an account projection. This is not a second
T-Invest stream client.

**One document, generated.** `docs/api/openapi.json` is produced by
`cargo run -p vox-api --bin openapi -- docs/api/openapi.json` and served at
`/api/v1/openapi.json`. The TypeScript client is generated from that file by
`python tools/api-client/generate.py`. CI regenerates both and fails on any diff, so a
hand-edited DTO cannot survive review.

## Authentication — deliberately deferred, not skipped

The browser authenticates to Vox, never to T-Invest. #17 owns credentials, RBAC and the
security boundary, and it has not landed. Choosing a session scheme now would mean choosing
it by convenience, which #38 forbids.

Until #17 lands the server binds `127.0.0.1:8080` by default, so an unauthenticated
deployment is not reachable from a network by accident, and TLS is terminated in front of
the process. When #17 lands, this note is updated with the selected scheme and its official
documentation before the code changes: same-origin cookie session with CSRF protection, or
bearer tokens with an explicit CORS policy — the decision belongs to #17.

No route returns a stored broker secret, and no secret-shaped field can appear in the
schema: a test fails the build if one does.

## What this slice does not do

No risk verdicts (#21), no valuation or P&L (#22), no strategy (#23), analytics (#24/#25),
models (#26), decisions (#27), backtests (#29), live broker market feed credentials (#17),
bulk protection migration (#10). Each is listed in the capability set with its owner, and
`docs/design/BACKEND_CONTRACTS.md` tracks the same dependencies for the design side.
