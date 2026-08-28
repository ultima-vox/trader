# @vox/api-client

The TypeScript client for the Vox application API, generated from `docs/api/openapi.json`,
which is itself generated from the Rust contracts in `crates/vox-api`.

```bash
cargo run -p vox-api --bin openapi -- docs/api/openapi.json   # 1. Rust -> OpenAPI
python tools/api-client/generate.py                            # 2. OpenAPI -> TypeScript
npm --prefix frontend/api-client run typecheck                 # 3. it still compiles
```

`src/types.ts` and `src/client.ts` are generated. Editing them by hand is how a frontend
starts disagreeing with its backend, so CI regenerates both and fails on any diff.

## What the types guarantee

- **Money is `string`.** `Decimal` is a decimal string with nine fractional digits. It is
  never a `number`, because a `number` cannot hold every value the backend can send.
- **Enums are exactly the backend spelling.** `BrokerEnvironment` is `"SANDBOX" | "PRODUCTION"`;
  `TradingMode` is `"LIVE"`. `PAPER` and `BACKTEST` do not exist here because they do not
  exist in a broker environment.
- **Errors are typed.** Every failure is an `ApiError` with a `category`, and `VoxApiError`
  carries it alongside the HTTP status.
- **A missing capability is visible.** `CAPABILITY_UNAVAILABLE` names the capability and the
  issue that owns it, so a screen can render a deferred state instead of a broken one.
