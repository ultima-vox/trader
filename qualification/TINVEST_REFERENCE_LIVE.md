# T-Invest reference-data live qualification

Status: **RERUN PENDING CREDENTIAL**

Runner covers all 39 current, non-deprecated `InstrumentsService` safe reads through generated tonic gRPC client. It derives by-ID requests from returned catalogues, exercises every response family and cursor continuation, and reports permission/environment gates through `CapabilityRegistry`. It never calls `EditFavorites`, `CreateFavoriteGroup`, or `DeleteFavoriteGroup`.

Method-by-method audit: [TINVEST_REFERENCE_METHODS.md](TINVEST_REFERENCE_METHODS.md).

```powershell
$env:TINVEST_TOKEN = '<read-only-capable token>'
cargo test --locked -p vox-tinvest --test reference_live -- --ignored --nocapture
```

Expected output contains one `QUALIFIED <method>` or `GATED <method> <state>` line for every applicable safe-read method. Any transport, request-shape, response-decode, timestamp, exact-number, or unclassified provider error fails the test.

Earlier REST qualification reached `GetFuturesMargin` and observed omitted `initialMarginOnBuy`. Generated protobuf regression coverage preserves missing message presence as `None`; Vox canonical margin validation fails closed before trading use.

Second credentialed run reached `TradingSchedules` and proved provider code `30002`: requested period must not exceed 14 days. A subsequent run proved code `30003`: `from` cannot precede provider current date. Production client now rejects historical UTC dates before dispatch and chunks longer future ranges without truncation; live qualification derives an exact seven-day future range from current UTC date.

Latest earlier REST run reached `GetForecastBy` and disproved reuse of consensus record `uid`. gRPC runner now resolves consensus `asset_uid` through `GetAssetBy`, probes only exact provider-issued `AssetFull.instruments[].uid` candidates, bounds selection to 8 assets x 8 sorted candidates, and fails typed inconsistency after candidate exhaustion.

Implementation environment recheck on 2026-08-24: `TINVEST_TOKEN` absent. Complete rerun cannot execute in this process; no credentialed result is claimed. Head of Development must supply token and attach full command output before accepting live evidence.
