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
  It requires successful subscription ACK before events, checks exact account identities, detects
  staleness, reconnects with bounded jitter/backoff, and restores subscriptions. Duplicate or
  out-of-order provider events remain evidence; adapter neither invents order nor fill state.

Native T-Invest trailing fields are always used. No market-data polling or synthetic trailing engine
exists. Fixed stop-loss and take-profit are independent legs, each with distinct broker request ID.
Provider trailing state, favorable extreme, execution price, stop identity and raw status remain
available in canonical readback. Unsupported protection fails with explicit capability error.

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
