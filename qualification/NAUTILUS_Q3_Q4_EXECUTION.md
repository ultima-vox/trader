# Q3/Q4 — T-Invest sandbox execution, UNKNOWN outcome, and reconciliation

Status: **Q3 PASS / Q4 PASS — LIVE EVIDENCE RECORDED**

## Objective

Qualify the execution boundary before accepting NautilusTrader as the Trader 2.0 runtime.

The tests establish that T-Invest order semantics can be represented without unsafe inference and that broker-authoritative state can reconcile local state after ambiguous or interrupted mutations.

## Safety boundary

All mutation tests ran only against T-Invest Sandbox. The live runners require an explicit `--execute` flag and never send a production order.

The token is supplied only through `TINVEST_TOKEN`.

## Q3 — execution lifecycle

### Live evidence

Q3 market execution passed:

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

## Q4 — reconciliation / recovery

### Q4a/Q4b — restart with broker state

A resting SBER order was created and minimal qualification identity state persisted locally.

A fresh process then reconstructed the broker state solely from authoritative SandboxService reads, without resubmitting the original mutation.

Observed live evidence:

- persisted client request ID remained available;
- persisted broker order ID remained available;
- authoritative order state after restart was `EXECUTION_REPORT_STATUS_NEW`;
- broker snapshot contained one active order;
- portfolio and positions were re-read from the broker;
- no order mutation was resent;
- cleanup subsequently changed the authoritative final state to `EXECUTION_REPORT_STATUS_CANCELLED`.

Result:

**PASS: fresh process reconstructed broker-authoritative order/account state without resubmitting the mutation.**

### Q4c — UNKNOWN mutation simulation

The harness dispatched a real SandboxService mutation and deliberately discarded the returned mutation response at the local adapter boundary.

Local outcome was explicitly represented as:

`UNKNOWN`

It then reconciled using the unique request identity and broker-authoritative state.

Observed live evidence:

- the dispatched mutation was found broker-side;
- it resolved to a broker order with `EXECUTION_REPORT_STATUS_NEW`;
- the ambiguous local result was never translated into `REJECTED`;
- no blind retry was performed;
- cleanup produced authoritative `EXECUTION_REPORT_STATUS_CANCELLED`.

Results:

**PASS: post-dispatch ambiguity remained UNKNOWN until broker evidence resolved it.**

**PASS: no blind retry was performed.**

## Q3/Q4 invariants — result

PASS:

- client request IDs are UUIDs and are never reused for distinct mutations;
- broker order ID and client request ID are distinct identities;
- a post-dispatch response loss is not translated into `REJECTED`;
- no blind retry of an ambiguous mutation is necessary;
- cancel/replace use authoritative broker state to resolve outcomes;
- quantity remains broker lots at the T-Invest boundary;
- broker state is sufficient to reconstruct the tested active order/account state after process restart;
- no T-Invest semantic approximation was required.

## Qualification verdict

**Q3: PASS**

**Q4: PASS**

Combined with the previously completed Q1 instrument/futures qualification and Q2 market-data/reconnect qualification, the architecture qualification provides sufficient evidence to accept NautilusTrader as the Trader 2.0 trading-runtime foundation.

The formal decision is recorded in `architecture/adr/ADR-0001-nautilus-runtime.md`.

Remaining work is implementation hardening, not foundation selection: production-grade T-Invest adapter plumbing, partial-fill/race coverage, persistent deduplication, rate limiting, observability, performance qualification, advanced risk integration, and controlled Nautilus upgrade compatibility tests.
