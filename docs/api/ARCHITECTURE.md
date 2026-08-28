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

**Market data is a projection, not a second broker client.** `MarketDataQueries` reads what
the accepted #8 adapter layer already acquired and republishes it provider-neutrally: quote,
order book, tape, candles, session and the instrument catalogue. Three facts are contract, not
decoration. Every record carries `MarketFreshness` (`stream`, `observed_at_unix_ms`, `age_ms`),
because a price without an age is a claim the operator cannot check, and a stale quote stays
visible with its age instead of vanishing. Every optional price is absent when the provider did
not supply it, which is a different thing from zero and must never render as one. A candle
states `closed`, so a still-forming bar is never read as settled. `InstrumentSummaryDto`
carries `lot_size` and `min_price_increment` so the order ticket validates against provider
metadata instead of guessing. No projection is attached in this slice: the six routes and the
`QUOTES`/`ORDER_BOOK`/`TRADES` stream topics answer `CAPABILITY_UNAVAILABLE` naming their
owner.

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
models (#26), decisions (#27), backtests (#29), the live market-data projection that feeds
the read model published here (#38, next slice),
connection or credential lifecycle (#17), bulk protection migration (#10). Each is listed in
the capability set with its owner, and `docs/design/BACKEND_CONTRACTS.md` tracks the same
dependencies for the design side.
