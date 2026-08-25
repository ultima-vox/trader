# T-Invest account/read-side adapter

Status: **OFFLINE QUALIFIED; LIVE CREDENTIAL PENDING**

## Contract and boundary

Provider source is official `invest-contracts` revision
`762e720e27164213f41cac0b226c5698c2ae8199`. `users.proto`, `operations.proto`,
`sandbox.proto` and required imports are vendored under `crates/vox-tinvest/proto/tinkoff/` and
compiled by `prost`/`tonic-build`. Generated messages and clients stay in
`vox_tinvest::generated`; account consumers use Vox-owned models in `account`, `operations`,
`reports`, and `operations_stream`.

Machine-readable inventory: [tinvest_account_contracts.json](tinvest_account_contracts.json).
Contract drift test compares every RPC in UsersService, OperationsService,
OperationsStreamService, and SandboxService with all 38 inventory rows. Production mutations and
execution reads remain generated/inventoried but deferred; issue #9 never calls them.

## Semantics

- Broker account, tariff, qualification, margin, portfolio, position, operation, report, and
  stream facts remain authoritative.
- `MoneyValue` and `Quotation` map to exact unit/nano values. Missing protobuf messages remain
  `None`; no financial zero is fabricated.
- Unknown enum wire numbers remain `i32`. Account, instrument UID, FIGI, position UID, asset UID,
  operation ID, parent operation ID, order ID, and trade ID remain separate identities.
- `GetOperationsByCursor` is production history path. Paginator defaults to 100, rejects values
  outside provider limit, accumulates provider order, detects repeated cursors and contradictory
  continuation state, and returns completed prefix on later failure.
- Provider documents operation IDs as mutable. History never deduplicates by operation ID.
  OperationsStream emits every duplicate/update. Reconciliation key uses account, parent,
  instrument, operation type, and provider timestamp; occurrence revision is observable.
- Report requests model generate/get oneofs. Page numbering is validated as zero-based.
  Foreign-issuer generation cannot cross a calendar year. Live qualification performs generation,
  bounded `30058` task polling, authoritative task readback, and every page through termination.
- Production and sandbox credentials have typed environment provenance. Client construction rejects
  credential/endpoint mismatch. `40003/UNAUTHENTICATED` is invalid/inactive credential;
  `40002/PERMISSION_DENIED` is insufficient scope. Only method-specific documented codes become
  gates; arbitrary provider errors fail qualification.
- Account selection requests `ACCOUNT_STATUS_OPEN`, rejects unknown/new/closed/no-access, bank and
  special account types, and uses deterministic account IDs. Live probes use brokerage/IIS only;
  multi-account values and OperationsStream receive only validated IDs.
- OperationsStream has no unary deadline. Request supports multiple unique accounts and
  `PingDelaySettings`; supervisor validates subscription ACK, detects stale streams, applies
  bounded jittered reconnect, restores full subscription, uses bounded channel backpressure, and
  supports forced reconnect plus graceful stop.
- `SANDBOX_ONLY` routes ordinary generated UsersService, OperationsService and all three
  OperationsStreamService RPCs to `sandbox-invest-public-api.tbank.ru:443`, then also qualifies
  SandboxService reads with same sandbox credential/account. Broker and foreign-income reports are
  explicitly environment-gated because official matrix excludes them from sandbox.
- Sandbox read parity covers accounts, portfolio, positions, withdraw limits, deprecated operations
  compatibility, and cursor operations. Sandbox mutations/execution reads remain deferred.

## Offline gates

Run contract tests first, then mandatory workspace gates:

```powershell
cargo test --locked -p vox-tinvest --test generated_contract
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets --all-features
```

## One final credentialed qualification

Runner is read-only. `TINVEST_TOKEN` is production-only; `TINVEST_SANDBOX_TOKEN` is sandbox-only.
When only sandbox token exists, runner automatically selects `SANDBOX_ONLY`; no production token is
required. Set `TINVEST_QUALIFICATION_ENV` to `SANDBOX_ONLY` or `PRODUCTION_READ_ONLY` for explicit
selection. If both tokens exist and no override exists, production mode is selected. Selected
credential preflight always targets selected endpoint. Runner never creates/funds sandbox account.

```powershell
$env:TINVEST_SANDBOX_TOKEN = '<sandbox token>'
cargo test --locked -p vox-tinvest --test account_live -- --ignored --nocapture
```

Optional production qualification:

```powershell
$env:TINVEST_TOKEN = '<read-only-capable production token>'
$env:TINVEST_QUALIFICATION_ENV = 'PRODUCTION_READ_ONLY'
cargo test --locked -p vox-tinvest --test account_live -- --ignored --nocapture
```

Credential preflight aborts once with `CREDENTIAL_INVALID_OR_INACTIVE` for provider `40003`; token
contents never enter diagnostics. Final output contains exactly one `QUALIFIED` or contract-justified
`GATED/UNAVAILABLE` result for each of 38 inventory rows. Sandbox-only acceptance qualifies every
sandbox-supported ordinary/SandboxService safe read plus Portfolio/Positions/Operations stream ACK
and ping; two prod-only reports emit `reason=ENVIRONMENT_UNSUPPORTED_SANDBOX`.

Audit sources: pinned official protobuf plus current T-Bank Dev Portal token, sandbox/prod,
UsersService, OperationsService, OperationsStream, report FAQ, and gRPC error-code pages.

Implementation environment had no `TINVEST_TOKEN` on 2026-08-25. No live result is claimed.
