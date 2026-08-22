# Rust foundation

Issue #12 establishes Rust as the production path for Vox Core. The Python package under
`src/trader2/` is frozen qualification evidence; Vox Core has no CPython or PyO3 dependency.

## Toolchain and gates

Rust is pinned to `1.97.1`. Run from repository root:

```bash
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace
cargo run --locked -p vox-core --bin vox-core
```

`vox-core` defaults to `sandbox`. `VOX_ENV` accepts `sandbox`, `paper`, or `live`.
`VOX_LIVE_MUTATIONS_ENABLED` defaults to `false`; live reads remain allowed while every live
mutation must pass an explicit authorization guard.

## Dependency boundaries

```text
vox-core -> vox-tinvest -> vox-domain
         -> vox-nautilus -> vox-domain
                         -> nautilus-model
```

- `vox-domain` owns broker-neutral exact values, provider-normalized instrument aliases, readiness, order identity, mutation outcome, and
  restart/reconciliation contracts.
- `vox-tinvest` owns credentials, wire DTOs, HTTPS/WebSocket transport, provider errors,
  correlation, retry, reconnect, and subscription state. Raw provider JSON does not escape it.
- `vox-nautilus` is the only boundary that constructs Nautilus instrument types from Vox values.
- `vox-core` owns composition and executable qualification entry points.

Every async queue is bounded. Retry sleeps use Tokio timers; no idle polling loop exists.

## Nautilus Rust/v2 decision

Registry and official nightly Rust documentation were checked on 2026-08-22. The workspace pins
`nautilus-model = 0.62.0` exactly, with default features disabled and `high-precision` enabled.
This yields the pure-Rust domain model with 128-bit fixed-point `Price`, `Quantity`, and `Money`
values and no Python feature.

High precision is selected despite T-Invest's current nine-decimal `units`/`nano` wire scale. It
matches official Python-wheel precision, adds range/precision headroom, and prevents a later
precision-mode data migration. Broker economics stay in Vox exact fixed-point values until a
checked conversion at the Nautilus boundary.

Only `nautilus-model` owns behavior in this foundation. Later runtime work will need matching
exact pins of `nautilus-data`, `nautilus-execution`, `nautilus-live`, and `nautilus-trading`; those
crates are intentionally not empty dependencies here. All Nautilus crates must come from one
release/source to avoid cross-version type mismatches.

Current Rust/v2 gaps versus the accepted Python/v1 Q1-Q4 path:

- Nautilus has no T-Invest adapter; Vox must provide data/execution clients and explicit mappings.
- v2 Rust lacks controller, tearsheet, and config-serialization parity. None blocks #12.
- full T-Invest `DataEngine` and `ExecutionEngine` wiring belongs to #8 and #10; #12 proves typed
  transport, exact model conversion, bounded reconnect, and mutation/restart semantics first.
- Nautilus documents floating-point intermediates in some position/PnL calculations. Vox exact
  broker economics and authoritative reconciliation remain independent; live-capital acceptance
  requires parity checks against broker values before those calculations become risk inputs.

Nautilus upgrades require a dedicated compatibility change: update every Nautilus crate together,
review changelogs/API differences, then rerun workspace gates and RQ1-RQ4 evidence. No wildcard,
caret, or automatic production upgrade is allowed.

Official references:

- <https://nautilustrader.io/docs/nightly/concepts/rust/>
- <https://nautilustrader.io/docs/nightly/getting_started/installation/>
- <https://crates.io/crates/nautilus-model/0.62.0>

## T-Invest transport policy

REST uses HTTPS, Bearer authentication, rustls certificate verification, explicit timeouts, and
typed request/response DTOs. Provider response body plus `x-tracking-id` are retained on errors.
Only classified safe reads may use bounded retry. Mutations are attempted once; a failure after
dispatch remains `UNKNOWN` until broker-authoritative reconciliation. HTTP 429/rate-limit metadata
feeds bounded backoff without assuming mutation safety.

Mutation calls require a one-shot `MutationAuthorization`. REST method grammar is canonicalized
before dispatch: safe-read entry points reject mutation methods, sandbox authorization accepts only
the exact `SandboxService`, paper authorization cannot mutate T-Invest, and live authorization
requires explicit live opt-in. New-exposure authorization additionally requires typed connectivity
and reconciliation evidence with zero unresolved `UNKNOWN` mutations.

WebSocket uses `wss://invest-public-api.tbank.ru/ws/...`, Bearer authentication, and the
`json-proto` subprotocol. Desired subscriptions are adapter-owned, ACK order is independent, and
reconnect restores desired subscriptions into a bounded event channel.

Official references:

- <https://developer.tbank.ru/invest/intro/developer/protocols/restapi>
- <https://developer.tbank.ru/invest/intro/developer/protocols/ws>
- <https://developer.tbank.ru/invest/intro/intro/limits>
- <https://developer.tbank.ru/invest/intro/intro/faq_custom_types>

## Qualification evidence

Workspace tests prove:

- RQ1: SBER identity/lot/tick and futures tick amount, money-per-point, and multiplier map exactly;
  inconsistent or unrepresentable metadata fails closed.
- RQ2: typed REST/WebSocket foundations, order-independent ACK tracking, forced reconnect state,
  automatic resubscription, post-reconnect event gating, and bounded-channel backpressure.
- RQ3/RQ4: mutation evidence is persisted as `UNKNOWN` before dispatch; restart chooses
  reconciliation, never blind resubmission; authoritative evidence resolves outcome.

Previously captured live T-Invest results remain under `qualification/`. No token or secret-bearing
trace belongs in repository. Full live reruns require `TINVEST_TOKEN` and must remain read-only for
RQ1/RQ2 or use T-Invest Sandbox for mutation evidence.

Run current Rust live/read-only gates with:

```bash
export TINVEST_TOKEN='...'
cargo run --locked -p vox-core --bin rq1_live
cargo run --locked -p vox-core --bin rq2_live
```

`rq1_live` fetches typed Shares/Futures/GetFuturesMargin responses, selects SBER plus one active
SPBFUT contract, preserves UID/FIGI/ticker/class-code aliases, validates catalogue/margin tick
equality, derives exact money-per-point, and constructs checked high-precision Nautilus instruments.
`rq2_live` first validates a typed REST
order-book snapshot, then authenticates WebSocket, validates all subscription ACK statuses,
binds ACKs/events to expected instrument UID, receives a market event, forces disconnect/reconnect,
replays desired subscriptions, and requires
a post-reconnect event through a bounded channel.

No `TINVEST_TOKEN` was available in the implementation environment, so this PR does not claim a
new Rust live capture. CI runs deterministic exact-value, reconnect/backpressure, sandbox-style
mutation/readback/cancel, and fresh-process recovery harnesses. Head of Development should run the
two commands above before accepting the live gate; outputs contain identities/economics but never
the token.

## Deferred scope

Issues #7-#11 remain blocked. Full provider catalogue, market-data surface, account/portfolio,
execution, persistence, API, UI, strategies, and ML/AI features are not part of this foundation.
