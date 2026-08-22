# Trader 2.0 — Implementation plan

## Rule

Do not start by building every module. Build vertical slices that prove production behavior while preserving the architecture contracts.

## Phase 0 — Baseline and governance

Deliverables:

- accepted architecture/domain/design documents;
- ADRs;
- repository layout;
- coding/testing conventions;
- CI baseline;
- secrets policy;
- environment model (`research|sandbox|live`).

Exit: architecture PR merged and no unresolved foundational decision blocks code structure.

## Phase 1 — Runtime skeleton + T-Invest adapter

Build:

- application process skeleton;
- isolated trading-runtime process;
- instrument registry;
- T-Invest reference data;
- T-Invest market-data stream;
- execution adapter;
- reconciliation/readiness state machine;
- structured logging/metrics;
- sandbox account support.

Reuse qualification tests as adapter regression tests.

Exit: headless process can connect, reconcile, stream market data and execute/cancel/replace in Sandbox safely after restart.

## Phase 2 — Portfolio + advanced risk

Build:

- broker-authoritative account/position projections;
- valuation/PnL;
- risk reservation model;
- exposure/concentration/loss limits;
- kill-switch semantics;
- risk-reducing close path;
- protection policy;
- audit lineage.

Exit: all execution passes deterministic risk and no stale/UNKNOWN state can free exposure incorrectly.

## Phase 3 — Application API + operator shell

Build API contracts before full screens:

- auth/RBAC;
- accounts/broker health;
- instruments/search/favorites;
- quotes/market-data topics;
- portfolio/positions/orders;
- manual TradeIntent endpoint;
- risk preview;
- system readiness/audit.

Frontend:

- design tokens/components;
- application shell;
- workspace/widget grid;
- instrument picker;
- order ticket;
- positions/orders/portfolio widgets;
- chart integration.

Exit: operator can perform complete Sandbox trading through the final UI path without direct broker calls from frontend.

## Phase 4 — Strategy + backtest parity

Build:

- strategy registry/versioning;
- common strategy interface;
- backtest/replay runtime;
- sandbox/live strategy runner;
- performance metrics;
- strategy risk budgets;
- parity test suite.

Exit: one reference strategy runs unchanged in backtest and Sandbox/live adapter paths.

## Phase 5 — Research + ML

Build:

- historical data storage/query;
- dataset definitions;
- feature pipelines;
- isolated workers;
- training jobs;
- model registry;
- evaluation/promotion lifecycle;
- inference service/interface.

Exit: approved model prediction can become typed `AnalysisEvidence` and be reproduced from versioned inputs.

## Phase 6 — Decision Center + AI agents

Build:

- analysis agent interface;
- typed evidence store;
- candidate aggregation;
- Decision Engine;
- TradeIntent lineage;
- manual approval workflow;
- AutomationGrant policies.

Exit: AI/ML can recommend and, when explicitly permitted, progress through the same deterministic risk/execution path.

## Phase 7 — Position management and automation

Build:

- protection policies;
- exits/trailing/time stops;
- lifecycle automation;
- emergency flatten modes;
- automation supervision and revocation.

Exit: positions remain manageable/recoverable through reconnect/restart and automated actions never bypass risk.

## Phase 8 — Production hardening

Required before real capital:

- long-running soak tests;
- broker reconnect chaos tests;
- process crash/restart tests;
- DB backup/restore;
- secret rotation;
- resource/latency profiling;
- rate-limit stress;
- stale-data scenarios;
- reconciliation mismatch drills;
- security review;
- operator runbooks;
- alerting;
- explicit live-environment activation procedure.

## First implementation issue after architecture baseline

Create a single vertical-slice issue:

**Runtime Foundation 01 — T-Invest production adapter + reconciled trading runtime**

It should not include UI, ML or AI. Its acceptance test is a persistent headless Sandbox runtime that survives reconnect/restart and exposes stable application read/write ports for the next layer.