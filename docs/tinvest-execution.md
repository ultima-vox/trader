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
as `STOP_ORDER_TYPE_TAKE_PROFIT` plus `TAKE_PROFIT_TYPE_TRAILING`, typed indent/spread fields and
currency price input. Omitted activation/spread remain absent. Vox policy requires
`instant_execution=true` when activation price is omitted. Complete wire invariants are audited
before dispatch, and redacted actual request shape is included in live failure evidence. Unsupported
protection fails with explicit capability error.

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
Sandbox mutations are paced below documented 200 unary requests/minute. Provider errors add a
cooldown; rate-limit errors wait a complete reset window, preventing follow-on
`RESOURCE_EXHAUSTED` noise. A correctly formed `PostSandboxOrder` returning documented generic
`INTERNAL/70001` is recorded as `BLOCKED/PROVIDER` with status, provider code, attempt, tracking ID
and redacted request shape. Mutation service is then latched off for qualification: no blind replay
or unrelated mutation cascade occurs. Cleanup readback still runs.

PowerShell command:

```powershell
$env:TINVEST_SANDBOX_TOKEN = '<sandbox-token>'
cargo test --locked -p vox-tinvest --test execution_live -- --ignored --nocapture
```

Runner reads no production-token variable and rejects any non-sandbox client. It selects open
sandbox account plus API-tradeable instrument from generated contracts, exercises unary order,
async order, replacement, cancellation, broker idempotency, controlled ambiguous-dispatch guard,
all supported stop/protection shapes, and both execution streams. Every row is printed as
`QUALIFIED`, `GATED/UNAVAILABLE`, `BLOCKED/PROVIDER`, or `FAILED`. Provider-blocked completion returns
a distinct qualification error, never an implementation-failure result. Cleanup always runs last,
cancels qualification-created active orders/stops once, flattens observed test exposure, and fails
unless authoritative readback confirms baseline resources plus zero net test lots.
