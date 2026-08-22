# ADR-0003 — Keep AI/ML outside the execution safety path

Status: **ACCEPTED**

Date: 2026-08-22

## Decision

AI agents and ML models may produce typed analysis evidence, signals and trade candidates. They may not directly call broker APIs, mutate orders, bypass risk or alter reconciliation state.

The required path is:

```text
AI/ML evidence -> Decision -> TradeIntent -> Advanced Risk -> ExecutionPlan -> Nautilus -> Broker
```

Autonomous progression additionally requires an explicit `AutomationGrant`.

## Rationale

AI outputs are probabilistic and may be stale, malformed or unavailable. Execution safety must remain deterministic, auditable and independently testable.

## Consequences

- AI can evolve rapidly without destabilizing execution;
- every AI-originated trade remains explainable through evidence lineage;
- model outages degrade advisory capability rather than broker safety.