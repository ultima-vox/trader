# Runtime operations and recovery

## Start

Configure one DB path per canonical provider/environment/account scope. In async
startup code use `SqliteRuntimeStore::open_async`; it runs open/migrations off Tokio
core workers. Store only
opaque `connection_ref` and `credential_ref`; secret resolution belongs to #17.
Second process opening same DB fails before `READY` through OS file lock. Runtime
epoch also fences stale tasks.

Healthy sequence:

```text
STARTING -> CONNECTING -> RECONCILING -> READY
```

`RuntimeHealth` separates liveness from readiness. `READY` can still reject new
exposure when execution authorization is disabled. `DEGRADED` rejects exposure by
default. `HALTED` always rejects it.

READY requires exact provider ACKs for OrderStateStream, PositionsStream,
PortfolioStream and OperationsStream. Their `required_for_ready` health flags stay
explicit. TradesStream is optional for readiness: required streams already signal
order, position, portfolio and operation changes, while TradesStream remains strict
when enabled and never substitutes pings for ACK. Any required disconnect or any
capital-state event closes admission until broker-authoritative unary refresh
completes.

## Diagnose HALTED

Read typed `reason_code`, then inspect bounded audit records and provider tracking
metadata. Never paste token or authorization header into logs or DB.

- `UNKNOWN_MUTATION`: do not retry command. Query exact order/request/stop identity,
  then run reconciliation. Operations history never proves cancel/replace: cancel
  needs terminal non-active order state; replace needs exact replacement identity.
- `BROKER_POSITION_CONFLICT`, `BROKER_ORDER_CONFLICT`,
  `BROKER_STOP_CONFLICT`: compare direct broker snapshots. Preserve unfamiliar manual
  orders/stops. Operator resolves provenance; runtime never auto-cancels them.
- `REQUIRED_READ_UNAVAILABLE`, `CREDENTIAL_REJECTED`, `ACCOUNT_UNAVAILABLE`: restore
  provider access, then perform full reconciliation.
- `PERSISTENCE_FAILURE`, `OWNERSHIP_FAILURE`, `CORRUPT_MUTATION_EVIDENCE`: keep runtime
  stopped. Repair/restore under operator control, then full reconciliation.
- `STREAM_GAP`, `STREAM_QUEUE_OVERFLOW`: gate closes automatically; recovery requires
  successful unary reconciliation.

Timeout never converts UNKNOWN to failed. Restart never converts UNKNOWN to
not-dispatched.

## Shutdown

Call `RuntimeCoordinator::shutdown`. It enters `STOPPING`, closes admission, waits a
bounded interval for admitted work, preserves ambiguous work as UNKNOWN, disconnects
streams, records `STOPPED`/clean shutdown, releases ownership. It does not cancel
broker orders or stops.

## Backup and restore

Stop runtime cleanly before copying DB, or use SQLite online backup tooling aware of
WAL. Back up DB plus required WAL state atomically. Keep filesystem permissions
restricted because audit metadata can be sensitive even though tokens are forbidden.

After restore, broker snapshot still wins. Start with execution disabled, complete
full reconciliation, inspect UNKNOWN and identity conflicts, then restore execution
authorization. Never edit unresolved journal rows to force readiness.

Unsupported newer schema and migration failure fail closed. Corrupt checkpoint may
be discarded and rebuilt only when mutation journal and typed identity evidence are
valid. Corrupt unresolved evidence requires operator intervention.

## Resource qualification

Run from repository root in PowerShell:

```powershell
qualification/live/runtime_soak.ps1 -Minutes 60
```

Harness records OS/CPU/RAM baseline, process CPU normalized to one logical core, RSS
growth and queue/reconnect summary. Required limits: 10-minute idle CPU average at
most 2% of one logical core, steady RSS at most 150 MiB, and 60-minute post-warm-up
RSS growth at most 20 MiB.
