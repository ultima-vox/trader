# T-Invest execution adapter

Issue #10 execution boundary uses generated `prost`/`tonic` contracts pinned at
`762e720e27164213f41cac0b226c5698c2ae8199`. Full RPC and stream inventory lives in
`qualification/tinvest_execution_contracts.json`; CI compares it with vendored proto files, so a
new RPC or omitted stream branch fails tests.

## Safety boundary

- `vox-domain::execution` owns broker-neutral order/protection commands, capability gates,
  lifecycle state and offline trailing semantic reference.
- `vox-tinvest::execution` validates commands, maps them to generated provider messages and maps
  provider responses into Vox-owned canonical types. Missing economics remain `None`; unknown enum
  numbers remain raw `i32` values.
- `vox-tinvest::execution_dispatch` persists `UNKNOWN_AFTER_DISPATCH` before every transport send.
  T-Invest mutation calls make one attempt. `DEADLINE_EXCEEDED`, `UNAVAILABLE`, `UNKNOWN` and other
  ambiguous transport outcomes remain UNKNOWN and block resubmit until authoritative readback or
  stream evidence resolves them. Production authorization remains off by default and differs from
  sandbox authorization.
- `vox-tinvest::execution_stream` owns bounded long-lived gRPC streams. No unary deadline is set.
  It requires successful subscription ACK within a separate bounded timeout, but valid account
  events and pings received before ACK do not close the stream. It checks exact account identities,
  detects staleness, reconnects with bounded jitter/backoff, and restores subscriptions. Duplicate
  or out-of-order provider events remain evidence; adapter neither invents order nor fill state.

Native T-Invest trailing fields are always used. No market-data polling or synthetic trailing engine
exists. Fixed stop-loss and take-profit are independent legs, each with distinct broker request ID.
Provider trailing state, favorable extreme, execution price, stop identity and raw status remain
available in canonical readback. Generated proto and current provider schema encode trailing stops
as `STOP_ORDER_TYPE_TAKE_PROFIT` plus `TAKE_PROFIT_TYPE_TRAILING` and typed indent/spread fields.
Delayed activation requires an explicit `stop_price` that has not already been reached for the
position direction; immediate activation omits `stop_price` and sets `instant_execution=true`.
Combining explicit and immediate activation fails before dispatch. Provider `30035` is treated as
documented invalid `stop_price`, never an unknown outage. Complete wire invariants are audited before
dispatch, and redacted actual request shape is included in live failure evidence. Unsupported
protection fails with explicit capability error. Sources: [StopOrders contract](https://developer.tbank.ru/invest/services/stop-orders/stoporders/),
[error 30035](https://developer.tbank.ru/invest/intro/developer/error-codes/errors).

Execution price convention is explicit and broker-neutral in commands. Adapter derives it from
authoritative instrument kind, operation and environment. Production `PostOrder`, `PostOrderAsync`,
`ReplaceOrder` and `GetOrderState` use settlement currency for shares/ETF/currency/bonds and points
for futures sourced in points. Production `PostStopOrder` inputs and every Sandbox mutation input
use settlement currency. Unknown convention fails closed; generated requests never use
`PRICE_TYPE_UNSPECIFIED`. These rules follow T-Bank's current
[price-unit guide](https://developer.tbank.ru/invest/intro/useful-info/points) and generated
`PriceType` contract.

## Nautilus boundary

Regular LIMIT/MARKET command intent may cross Nautilus command boundary only with exact lot quantity,
side, order type and unit/nano price. Broker order/request IDs are attached only after provider
evidence. `CanonicalTradeBatch` may become Nautilus `OrderFilled` only when broker fill ID, order ID,
instrument ID, exact price, quantity and authoritative timestamp are all present. Missing facts fail
closed; no timestamp, fill, status or economics is fabricated.

T-Invest native trailing state, stop lifecycle, provider status causes, pre-trade estimates and
provider-specific flags have no faithful complete Nautilus representation. They stay Vox canonical
extensions. Exact routing and limitations are recorded per method in machine inventory.

## Qualification

Offline order:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
```

Only after these pass, set `TINVEST_SANDBOX_TOKEN` and run one ignored sandbox qualification test.
Production credentials and production mutation qualification are prohibited. Runner must report one
result for every inventory capability, clean every active test order/stop it created, then confirm
cleanup by broker readback. `GATED/UNAVAILABLE` is valid only for documented sandbox capability,
account, permission or deterministic-trigger limitations; arbitrary provider errors fail run.
All SandboxService qualification calls, including reads and reconciliation, share one quota pacer.
The gRPC adapter preserves provider `x-ratelimit-limit`, `x-ratelimit-remaining` and
`x-ratelimit-reset` metadata from successful responses and errors. Pacing uses that advertised
bucket state; absent metadata falls back to the documented 200 requests/minute SandboxService
limit, while `RESOURCE_EXHAUSTED` without reset metadata waits the documented minute window. A
correctly formed `PostSandboxOrder` returning documented generic
`INTERNAL/70001` is recorded as `BLOCKED/PROVIDER` with status, provider code, attempt, tracking ID
and redacted request shape. Mutation service is then latched off for qualification: no blind replay
or unrelated mutation cascade occurs. Cleanup readback still runs.

`PostSandboxOrderAsync` transport failure remains `UNKNOWN_AFTER_DISPATCH`. Runner never replays it;
it performs bounded `GetSandboxOrderState` reconciliation by request ID plus active-order list
readback. Authoritative discovery resolves persisted UNKNOWN to accepted broker identity and may
qualify the row; exhausted reconciliation reports unresolved UNKNOWN with quota evidence.

TradesStream sends generated `TradesStreamRequest { accounts, ping_delay_ms: Some(5000) }` to the
sandbox endpoint. Full runner and targeted credentialed test use one acceptance path. Evidence
records request shape, client request ID, provider tracking ID, bounded stream liveness, pings,
stream errors, exact account/instrument/order identities, matching broker order IDs, and
authoritative BUY/SELL fill readback. Sandbox currently keeps the stream alive and emits matching
trade events plus pings, but omits the formal subscription ACK. This exact case is
`QUALIFIED_WITH_PROVIDER_DEVIATION` only after a qualification trade executes, its broker order ID
matches a stream event, broker readback confirms both BUY and SELL fills, and cleanup succeeds.
Missing matching event is `BLOCKED/PROVIDER`; stream, identity, readback, or cleanup failure is
`FAILED`. Missing ACK is recorded, never fabricated. Runtime TradesStream ACK semantics remain
strict for production supervision. OrderStateStream qualification remains strict because sandbox
returns its valid exact-account ACK.

Observed 2026-08-27 sandbox evidence: targeted request
`47c2eca4-22e7-478b-8757-1e2da04a3a65` / tracking
`a2f218b4b71d699fb9f0cc436cf354a5`; full-runner request
`d246e2b0-fcae-41fe-a09f-b407ac374ec7` / tracking
`5f9e2a86dc0ab361c8cea9abd0bcf5d8`. Both observed five pings, matching BUY/SELL
trade events and authoritative fill readback, no stream errors, and qualified cleanup; only
subscription ACK was absent.

PowerShell command:

```powershell
$env:TINVEST_SANDBOX_TOKEN = '<sandbox-token>'
cargo test --locked -p vox-tinvest --test execution_live -- --ignored --nocapture
```

Runner reads no production-token variable and rejects any non-sandbox client. It selects open
sandbox account plus API-tradeable instrument from generated contracts, exercises unary order,
async order, replacement, cancellation, broker idempotency, controlled ambiguous-dispatch guard,
all supported stop/protection shapes, and both execution streams. Every row is printed as
`QUALIFIED`, `QUALIFIED_WITH_PROVIDER_DEVIATION`, `GATED/UNAVAILABLE`, `BLOCKED/PROVIDER`, or
`FAILED`. Provider-blocked completion returns a distinct qualification error, never an
implementation-failure result. TradesStream acceptance performs and verifies cleanup before
recording its row; final cleanup runs again after OrderStateStream, cancels any remaining
qualification-created active orders/stops, flattens observed test exposure, and fails unless
authoritative readback confirms baseline resources plus zero net test lots.
