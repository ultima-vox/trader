# T-Invest reference-data live qualification

Status: **RERUN PENDING CREDENTIAL**

Runner covers all 39 current, non-deprecated `InstrumentsService` safe reads. It derives by-ID requests from returned catalogues, exercises every response family, performs cursor/page reads, and reports permission/tariff/provider gates through `CapabilityRegistry`. It never calls `EditFavorites`, `CreateFavoriteGroup`, or `DeleteFavoriteGroup`.

Method-by-method audit: [TINVEST_REFERENCE_METHODS.md](TINVEST_REFERENCE_METHODS.md).

```powershell
$env:TINVEST_TOKEN = '<read-only-capable token>'
cargo test --locked -p vox-tinvest --test reference_live -- --ignored --nocapture
```

Expected output contains one `QUALIFIED <method>` or `GATED <method> <state>` line for every applicable safe-read method. Any transport, request-shape, response-decode, timestamp, exact-number, or unclassified provider error fails the test.

First credentialed run reached `GetFuturesMargin` and proved that current REST protobuf JSON omits unset `initialMarginOnBuy`. Regression coverage now preserves that field as `None`, audits omission across the complete reference wire surface, and requires explicit fail-closed economics validation.

Second credentialed run reached `TradingSchedules` and proved provider code `30002`: requested period must not exceed 14 days. A subsequent run proved code `30003`: `from` cannot precede provider current date. Production client now rejects historical UTC dates before dispatch and chunks longer future ranges without truncation; live qualification derives an exact seven-day future range from current UTC date.

Latest credentialed run reached `GetForecastBy` and proved arbitrary catalogue instruments are not valid forecast samples. Runner now uses only authoritative UIDs returned by `GetConsensusForecasts`, reports explicit `UNAVAILABLE` when none exists, and treats 404 for a provider-sourced UID as inconsistency. Brands, consensus forecasts, insider deals, and news exercise a second page whenever provider metadata exposes one.

Implementation environment recheck on 2026-08-24: `TINVEST_TOKEN` absent. Complete rerun cannot execute in this process; no credentialed result is claimed. Head of Development must supply token and attach full command output before accepting live evidence.
