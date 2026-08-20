# Trader 2.0 — Runtime Architecture Qualification Plan

## Goal

Decide whether NautilusTrader can serve as the production trading runtime for Trader 2.0 while preserving T-Invest/MOEX semantics exactly enough for safe broker-connected operation.

This phase is a qualification exercise, not product development.

## Mandatory principles

- NautilusTrader is evaluated as an upstream dependency, not a fork.
- Broker SDK/protobuf/native types remain inside the T-Invest adapter boundary.
- Financial values must not be approximated at execution/risk boundaries.
- T-Invest futures quoted-point semantics must be mapped explicitly to money exposure and margin.
- Request sent != accepted.
- Connected != fresh.
- Unclassified broker mutation outcome = UNKNOWN, not rejected.
- Broker-authoritative reconciliation is required before READY.
- No LLM/AI component may directly approve risk or issue broker mutations.
- No UI, ML platform, agent catalogue, scanner, or product feature work is in scope until runtime qualification passes.

## Q1 — Instruments

Verify at minimum one MOEX share and one MOEX futures contract.

Preserve:

- application identity
- T-Invest UID / FIGI / ticker where applicable
- venue / exchange
- asset class
- currency
- lot size
- minimum price increment
- price and quantity precision
- futures expiration
- multiplier / point-value semantics
- authoritative margin data when available
- trading/API availability

### Futures invariant

Never assume:

`notional = quoted_price * quantity`

Document and test the exact relationship between quoted points, minimum price increment, minimum price increment amount / point value, contract/lot multiplier, money exposure, and margin.

### PASS

The selected instruments can be represented without loss of economically material semantics and without hidden approximations.

### FAIL

Reject NautilusTrader if correct MOEX/T-Invest futures economics requires changing fundamental runtime domain semantics or guessing broker data.

## Q2 — Market data

Verify:

`T-Invest stream -> adapter -> Nautilus normalized data -> DataEngine -> test consumer`

Minimum surfaces:

- quote / last price where semantically available
- trades
- order book
- candles/bars
- trading status

Preserve event and receive timestamps separately where available, price units, quantity/lot semantics, and depth semantics.

Reconnect must restore desired subscriptions automatically and must not fabricate empty depth or silently duplicate state.

## Q3 — Execution

After Q1/Q2 are accepted, qualify:

- account bootstrap
- submit
- partial fill
- cancel
- replace
- idempotency/client-order identifiers
- order/fill/position mapping

## Q4 — Failure and recovery

Mandatory adversarial scenarios:

- transport timeout before dispatch
- timeout after possible dispatch
- stream disconnect
- duplicate/out-of-order events
- partial-fill + cancel race
- replace race
- restart with open order
- restart with open position
- broker state != local cache
- external broker order not created by Trader

Mutation outcome taxonomy must remain:

- NOT_DISPATCHED
- REJECTED
- UNKNOWN

UNKNOWN must resolve through broker-authoritative reconciliation.

## Q5 — Futures economic parity

For selected real T-Invest futures metadata compare runtime calculations against broker-authoritative values for:

- contract value
- exposure
- PnL
- initial/maintenance margin where exposed
- lot/quantity semantics

Only explicitly documented rounding differences are acceptable.

## Q6 — Performance

Use a realistic load, not a synthetic headline benchmark. Initial target profile:

- ~500 catalogue instruments
- ~100 actively subscribed instruments
- quotes/trades/bars/order books
- multiple consumers

Measure CPU, RAM, event latency, queue/backpressure behaviour, and recovery under reconnect.

## Acceptance decision

NautilusTrader is accepted only if Q1-Q6 establish that it can preserve broker economics, failure semantics, recovery correctness, and acceptable resource behaviour without invasive upstream modification.

Final result must be one of:

- PASS
- PASS WITH CONSTRAINTS
- FAIL

and must be recorded in an ADR before Trader 2.0 product implementation starts.
