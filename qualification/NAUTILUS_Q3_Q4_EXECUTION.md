# Q3/Q4 — T-Invest sandbox execution, UNKNOWN outcome, and reconciliation

Status: **IMPLEMENTATION / LIVE EVIDENCE PENDING**

## Objective

Qualify the execution boundary before accepting NautilusTrader as the Trader 2.0 runtime.

The test must establish that T-Invest order semantics can be represented without unsafe inference and that broker-authoritative state can reconcile local state after ambiguous or interrupted mutations.

## Safety boundary

All mutation tests run only against T-Invest Sandbox. The live runner requires an explicit `--execute` flag and never sends a production order.

The token must be supplied only through `TINVEST_TOKEN`.

## Q3 — execution lifecycle

Required live scenarios:

1. create or select an isolated sandbox account;
2. fund the account in RUB;
3. submit a 1-lot SBER market order with a UUID idempotency key;
4. verify the returned request/order identifiers and authoritative `GetSandboxOrderState`;
5. verify `GetSandboxPortfolio` / `GetSandboxPositions` after execution;
6. submit a non-marketable limit order;
7. replace it using a new idempotency key;
8. cancel it;
9. verify the final broker state from `GetSandboxOrders` / `GetSandboxOrderState`;
10. flatten any test position before cleanup unless the Q4 restart scenario explicitly requires it to remain open.

## Q3 invariants

- client request IDs are UUIDs and are never reused for distinct mutations;
- broker order ID and client request ID are stored as distinct identities;
- a transport timeout after dispatch is never translated into `REJECTED`;
- no blind retry of an ambiguous mutation is allowed;
- cancel/replace use authoritative broker state to resolve races;
- quantity is expressed in broker lots at the T-Invest boundary and converted only at the domain adapter boundary.

## Q4 — reconciliation / recovery

Required scenarios:

### Q4a — authoritative snapshot

Given an account with broker-side orders/positions, collect:

- `GetSandboxOrders`;
- `GetSandboxPortfolio`;
- `GetSandboxPositions`;
- `GetSandboxOrderState` for known order IDs.

The resulting state is treated as broker-authoritative evidence.

### Q4b — restart with state

1. leave one known sandbox state item (open position and/or resting order);
2. terminate the local qualification process;
3. restart with only the persisted account ID and known client/order IDs;
4. reconstruct order/position state solely from broker reports;
5. prove local state converges to broker state without resubmitting the mutation.

### Q4c — UNKNOWN mutation simulation

The qualification harness must simulate a response loss **after dispatch**. It must record the local outcome as `UNKNOWN`, then query by request/order identity and reconcile to one authoritative result:

- accepted/open;
- partially filled;
- filled;
- canceled/replaced;
- rejected/not found, only when broker evidence proves non-acceptance.

The harness must not classify a socket timeout itself as a broker rejection.

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

## Current implementation

`qualification/live/q3_sandbox.py` provides the first sandbox execution qualification harness. It is intentionally adapter-oriented rather than product code.
