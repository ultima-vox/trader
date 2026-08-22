# Q3/Q4 — T-Invest sandbox execution, UNKNOWN outcome, and reconciliation

Status: **Q3 PASS / Q4 LIVE EVIDENCE PENDING**

## Objective

Qualify the execution boundary before accepting NautilusTrader as the Trader 2.0 runtime.

The test must establish that T-Invest order semantics can be represented without unsafe inference and that broker-authoritative state can reconcile local state after ambiguous or interrupted mutations.

## Safety boundary

All mutation tests run only against T-Invest Sandbox. The live runners require an explicit `--execute` flag and never send a production order.

The token must be supplied only through `TINVEST_TOKEN`.

## Q3 — execution lifecycle

### Live evidence

Q3 market execution passed against sandbox account `c32df791-4fc1-4414-a2cf-fab025accdff`:

- 1-lot SBER market BUY returned `EXECUTION_REPORT_STATUS_FILL`;
- authoritative `GetSandboxOrderState` confirmed fill;
- position appeared in broker snapshot;
- 1-lot market SELL flattened the position;
- active orders returned to zero.

Q3 replace/cancel also passed:

- non-marketable SBER LIMIT BUY returned `EXECUTION_REPORT_STATUS_NEW`;
- replace used a distinct idempotency/request ID and returned a new broker order ID;
- cancel targeted the replacement order ID;
- authoritative final status was `EXECUTION_REPORT_STATUS_CANCELLED` with zero executed lots;
- active orders returned to zero and no SBER position remained.

### Q3 verdict

**PASS.** Submit/fill/replace/cancel semantics map without identity loss or blind retries.

## Q3 invariants

- client request IDs are UUIDs and are never reused for distinct mutations;
- broker order ID and client request ID are stored as distinct identities;
- a transport timeout after dispatch is never translated into `REJECTED`;
- no blind retry of an ambiguous mutation is allowed;
- cancel/replace use authoritative broker state to resolve races;
- quantity is expressed in broker lots at the T-Invest boundary and converted only at the domain adapter boundary.

## Q4 — reconciliation / recovery

`qualification/live/q4_reconciliation.py` implements the remaining qualification scenarios.

### Q4a/Q4b — restart with broker state

Prepare a resting broker order and persist only qualification identity state:

```bash
python -m qualification.live.q4_reconciliation \
  --execute \
  --account '<sandbox-account-id>' \
  --prepare-restart
```

Then terminate that process and run a fresh process:

```bash
python -m qualification.live.q4_reconciliation \
  --resume-restart
```

The fresh process must reconstruct order/account state solely from:

- `GetSandboxOrders`;
- `GetSandboxPortfolio`;
- `GetSandboxPositions`;
- `GetSandboxOrderState`.

It must not resubmit the order.

Optional cleanup after successful reconciliation:

```bash
python -m qualification.live.q4_reconciliation \
  --execute \
  --resume-restart \
  --cleanup
```

### Q4c — UNKNOWN mutation simulation

The harness performs a real SandboxService dispatch, deliberately hides the returned response from the adapter layer, records the local outcome as `UNKNOWN`, and then reconciles only through broker-authoritative state:

```bash
python -m qualification.live.q4_reconciliation \
  --execute \
  --account '<sandbox-account-id>' \
  --unknown-after-dispatch \
  --cleanup
```

The request must not be retried blindly. `GetSandboxOrders` is searched by the unique `orderRequestId`, then `GetSandboxOrderState` resolves the authoritative status.

## PASS criteria

Q3/Q4 PASS requires all of the following:

- submit/fill/cancel/replace semantics map without identity loss;
- broker state is sufficient to reconstruct orders and positions after restart;
- ambiguous post-dispatch outcomes remain UNKNOWN until reconciliation;
- duplicate application of fills/positions is prevented;
- no requirement to change T-Invest semantics to satisfy Nautilus abstractions.

## FAIL criteria

Reject the runtime/adaptation approach if any of these are unavoidable:

- timeout must be represented as rejected/failed before broker evidence exists;
- broker request/order identity cannot be mapped losslessly;
- reconciliation cannot reconstruct broker-authoritative state after restart;
- replace/cancel races require blind mutation retries;
- futures/share quantity or price semantics must be approximated.
