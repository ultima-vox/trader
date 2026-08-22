# Trader 2.0 — Advanced risk model

## Scope

Trader 2.0 advanced risk is an application-domain layer above Nautilus runtime safeguards. It evaluates economic exposure and automation permissions before an execution plan reaches the runtime.

The risk engine is deterministic, explainable and fail-closed.

## Risk hierarchy

Risk limits may exist at multiple scopes:

```text
System
  -> Broker
    -> Account
      -> Strategy
        -> Instrument / asset class
```

The most restrictive applicable rule wins unless a policy explicitly defines another aggregation rule.

## Required controls

### Exposure

- max gross exposure;
- max net exposure;
- max account exposure;
- max instrument exposure;
- max asset-class exposure;
- max strategy exposure;
- max single-order notional;
- max position quantity/contracts.

### Concentration

- max percentage of NAV in one instrument;
- sector/group concentration when classification exists;
- correlated exposure groups;
- futures underlying concentration.

### Loss and drawdown

- daily realized loss;
- daily total PnL floor;
- rolling loss limits;
- account drawdown;
- strategy drawdown;
- consecutive-loss/circuit-breaker policy where configured.

### Liquidity / execution

- maximum order size relative to observed liquidity;
- maximum spread/slippage tolerance;
- stale quote/order-book guard;
- market-order restrictions;
- allowed trading sessions/statuses.

### Leverage / derivatives

Derivatives risk must use contract economics and broker/account margin information, not naive `price × quantity` assumptions.

For futures retain distinct concepts:

```text
quoted price
contract monetary value
initial/maintenance margin
position exposure
```

### Protection

Protection requirements apply to **new or increased exposure**, not blindly to any sell transaction.

A reduction/close of an existing long position must not be rejected merely because it lacks a new protective stop.

Policy may require:

- stop-loss before/after opening;
- max time allowed unprotected;
- allowed protection distance;
- minimum stop validity;
- fail-safe flatten if required protection cannot be established.

### Automation

Autonomous execution requires both risk approval and a valid `AutomationGrant`.

Automation limits include:

- max order size;
- max daily turnover;
- max simultaneous positions;
- instrument allowlist/universe;
- order type allowlist;
- maximum strategy risk budget;
- allowed session/time windows;
- grant expiry.

## Risk evaluation input

Risk receives an immutable projected state containing at least:

- account snapshot/NAV;
- existing positions;
- active orders and reserved exposure;
- proposed intent/plan;
- instrument economics;
- current market-data freshness/liquidity metrics;
- strategy and automation identity;
- realized/unrealized PnL and risk counters.

## Reservation model

Risk must account for in-flight orders so concurrent requests cannot independently pass against the same capacity.

A reservation is created before dispatch and released/adjusted only by authoritative lifecycle evidence.

If a mutation becomes `UNKNOWN`, the reservation remains until reconciliation resolves exposure.

## Risk decision

Every evaluation returns a typed result:

```text
ALLOW
RESIZE
DENY
REQUIRE_PROTECTION
REQUIRE_MANUAL_APPROVAL
```

Plus:

- reason codes;
- observed metrics;
- applicable limits;
- projected metrics;
- reservation ID where relevant.

Human-readable messages are generated from reason codes, not used as the canonical decision representation.

## Kill switches

### System kill switch

Blocks new exposure globally. Closing/reducing risk remains allowed unless the broker/runtime itself is unsafe.

### Account kill switch

Same semantics for one account.

### Strategy kill switch

Stops new strategy-driven exposure but permits controlled exits.

Kill switches must distinguish:

- `NO_NEW_EXPOSURE`;
- `CANCEL_OPENING_ORDERS`;
- `FLATTEN`.

A single ambiguous boolean is insufficient.

## Degraded modes

Risk can move execution into degraded mode when:

- market data is stale;
- reconciliation is not healthy;
- broker state is incomplete;
- reference economics are stale/missing;
- loss counters cannot be trusted.

Default degraded behavior: no new exposure; risk-reducing actions remain available if they can be evaluated safely.

## Auditability

Every risk decision is persisted with:

- policy/config version;
- input snapshot identity;
- calculation timestamp;
- reason codes;
- result;
- correlation ID.

This enables exact reconstruction of why an order was accepted, resized or denied.