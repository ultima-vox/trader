# T-Invest runtime qualification

Issue: #11
Environment: T-Invest Sandbox only
Observed: 2026-08-27
OS: Microsoft Windows 11 Pro `10.0.26100`
CPU: Intel Xeon E5-2680 v4, 28 logical cores
Pinned provider contracts: `762e720e27164213f41cac0b226c5698c2ae8199`

## Full runtime runner

Command:

```text
cargo test --locked -p vox-runtime --test runtime_live complete_runtime_qualification_in_sandbox -- --ignored --nocapture
```

Result: `1 passed; 0 failed`; 16/16 rows `QUALIFIED`.

Evidence:

- ownership epoch 1 acquired; second starts used fenced higher epochs;
- exact OrderStateStream, PositionsStream, PortfolioStream and OperationsStream
  subscription ACKs received before runtime connection became active; TradesStream
  remains optional for READY and strict when configured;
- initial broker snapshot/stream/snapshot handoff committed reconciliation
  `51bbab1b-c3cf-4d8c-90a8-e16b892542a8` before READY;
- one durable `UNKNOWN_AFTER_DISPATCH` fence preceded each single transport attempt;
- acknowledged sandbox order retained typed logical-request/broker-order link;
- restart observed broker-visible open order without replay;
- controlled post-dispatch UNKNOWN survived shutdown and resolved through
  `GetSandboxOrderState` with `ORDER_ID_TYPE_REQUEST`; execution adapter received no
  replay call;
- authoritative position snapshot returned 12 position facts; injected external-position
  notification closed admission and forced unary refresh before READY;
- duplicate stable broker evidence inserted once;
- forced required-stream gap closed admission; four exact ACKs plus unary reconciliation
  restored READY;
- unresolved UNKNOWN count finished at zero;
- cleanup canceled two qualification orders; active-order readback returned zero.

No token, account ID, broker order ID or raw provider payload is recorded here.

## Execution/protection and sandbox stream evidence

Issue #10 full execution runner remains required alongside runtime runner. It proves
real BUY/SELL fills, point readback, regular order lifecycle, relative and absolute
native trailing stop identities, cleanup, and strict OrderStateStream ACK.

TradesStream sandbox omits formal subscription ACK while connection remains alive,
pings arrive, exact broker order identities match real trade events, authoritative
readback confirms BUY/SELL fills, and cleanup succeeds. Classification remains
`QUALIFIED_WITH_PROVIDER_DEVIATION`. No ACK is fabricated. Production TradesStream
and OrderStateStream ACK semantics remain strict.

## Resource soak

Reproduce:

```powershell
qualification/live/runtime_soak.ps1 -Minutes 60
```

Acceptance: average idle CPU at most 2% of one logical core, max RSS at most 150 MiB,
post-warm-up RSS growth at most 20 MiB, runtime stays READY, queue bounds hold, no
reconnect storm.

Observed result after four required account streams and bounded stale detection:
`QUALIFIED`; 717 samples over 60 minutes, average idle CPU 0.02% of one logical
core, max RSS 18.4 MiB, post-warm-up RSS growth 0.0 MiB. Runtime stayed READY,
bounded queues held, and no reconnect storm occurred.
