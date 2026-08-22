# ADR-0002 — Start as a modular monolith with isolated runtimes

Status: **ACCEPTED**

Date: 2026-08-22

## Context

Trader 2.0 needs strong module boundaries, but premature microservices would add deployment, network, observability and distributed-consistency complexity before there is measured scaling pressure. The previous application also demonstrated that placing live trading and heavy analytical work in one runtime causes CPU/memory contention.

## Decision

Use a modular-monolith application architecture with explicit bounded modules, while isolating the safety-critical trading runtime and CPU/GPU-heavy research/ML workers into separate processes from the start.

Logical boundaries are defined independently from process boundaries. A module can later become a service without changing domain contracts.

## Consequences

- application logic remains easy to develop/test transactionally;
- live trading cannot be starved by training/backtest workloads;
- fewer infrastructure components are required initially;
- extraction to services is evidence-driven rather than speculative.

## Extraction criteria

A module becomes a service only for independent scaling, fault isolation, deployment cadence, runtime/language requirements or security/regulatory isolation.