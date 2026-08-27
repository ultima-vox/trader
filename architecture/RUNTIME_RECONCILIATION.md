# Broker-authoritative runtime and reconciliation

Issue #11 adds `vox-runtime`, an isolated single-node runtime. It depends on narrow
ports and accepted `vox-tinvest` adapters. It does not own credentials and does not
reconstruct broker state.

## Authority boundary

T-Invest unary responses own account accessibility, portfolio/cash, positions,
orders, order state, stops, operations and fills. Streams provide low-latency
evidence only. SQLite owns Vox request intent, dispatch uncertainty, typed identity
links, epochs, checkpoints, bounded dedupe and audit evidence. A checkpoint speeds
recovery; it never overrides a broker snapshot.

Official audit is machine-readable in
`qualification/tinvest_runtime_reconciliation_contracts.json`. It pins provider
contract revision `762e720e27164213f41cac0b226c5698c2ae8199` and records known
documentation/proto differences.

## Startup and handoff

1. Open/migrate SQLite with `WAL`, `synchronous=FULL`, foreign keys and bounded busy
   timeout. Acquire process file lock and transactionally increment account epoch.
2. Resolve opaque credential reference through `CredentialResolverPort`. Runtime DB
   never receives token material.
3. Enter `RECONCILING`; normal mutation gate stays closed.
4. Read accounts, portfolio, positions, active orders, stops, point order states and
   cursor operations. Safe reads use bounded classified retries. Mutations receive no
   runtime retry layer.
5. Resolve durable uncertainty using exact typed identity precedence: point state,
   active/list state, operation/fill identity, accepted stream evidence, then local
   evidence only to preserve uncertainty.
6. Persist resolved records, identity links, dedupe facts, checkpoint and readiness
   decision in one transaction.
7. Connect streams, then reconcile again. Snapshot/event races converge through
   broker identities, dedupe and reconciliation generation.
8. Enter `READY` only after committed complete reconciliation. Execution remains
   separately gated by authorization.

Any capital-relevant stream gap or bounded-channel overflow closes admission and
runs authoritative reconciliation. Events carry runtime epoch; stale epoch work is
ignored. Production subscription ACK semantics remain strict. Sandbox TradesStream
missing ACK stays qualification-only `QUALIFIED_WITH_PROVIDER_DEVIATION`; pings never
become an ACK.

## Mutation invariant

Coordinator inserts unique logical mutation, then atomically changes it to
`UNKNOWN_AFTER_DISPATCH` before exactly one transport attempt. ACK/rejection updates
that durable row. Crash, transport loss or shutdown leaves UNKNOWN intact. Restart
never submits it again. Only exact broker readback can reconcile it.

Cancel absence is inconclusive. Multi-leg protection resolves each leg separately.
Unknown/manual broker orders and stops remain untouched; ambiguous provenance halts
execution for operator reconciliation.

## Resource model

- execution/control admission: 256
- each stream channel: 1024
- reconciliation read concurrency ceiling: 8, further rate-limit constrained
- SQLite connection ceiling: 4; current implementation uses one locked connection
- metrics: 256 series, four typed labels per series
- event/audit tables: time and count compaction
- schema v2 hard caps journal admission at 1,000,000 logical mutations and enforces
  per-scope event/audit caps (100,000/20,000) with SQLite triggers, so retention does
  not depend on a healthy reconciliation loop

All SQLite calls made by async runtime pass through `spawn_blocking`. Backoff uses
Tokio timers. No polling/busy-spin loop exists.
