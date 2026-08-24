# T-Invest reference-data live qualification

Status: **PENDING CREDENTIAL**

Runner covers all 39 current, non-deprecated `InstrumentsService` safe reads. It derives by-ID requests from returned catalogues, exercises every response family, performs cursor/page reads, and reports permission/tariff/provider gates through `CapabilityRegistry`. It never calls `EditFavorites`, `CreateFavoriteGroup`, or `DeleteFavoriteGroup`.

```powershell
$env:TINVEST_TOKEN = '<read-only-capable token>'
cargo test --locked -p vox-tinvest --test reference_live -- --ignored --nocapture
```

Expected output contains one `QUALIFIED <method>` or `GATED <method> <state>` line for every applicable safe-read method. Any transport, request-shape, response-decode, timestamp, exact-number, or unclassified provider error fails the test.

Implementation environment check on 2026-08-24: `TINVEST_TOKEN` absent. No credentialed result is claimed. Head of Development must supply token and attach command output before accepting live evidence.
