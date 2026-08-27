# Vox Trader — backend dependency specification for the frontend

What the backend must expose before frontend application code can be written. Companion
to `BACKEND_CONTRACTS.md`, which records what exists today; this document specifies what
is missing, in the shape the frontend needs it.

Ground rules taken from issue #18 and applied to every item below:

- The frontend consumes typed Vox read models and commands. It never calls a provider.
- No provider wire/protobuf type may appear in a Vox response consumed by the UI.
- Money crosses the boundary as **exact** values — either `{ units: i64, nano: i32 }` or a
  decimal string — never a JSON number. The existing `FixedPoint`/`UnitsNano` types are the
  reference (`NANO_SCALE = 1e9`).
- Secrets never cross: connections and credentials are addressed by `OpaqueRef`, which
  already rejects secret-like material.
- Every refusal carries a machine value from a canonical enum plus a human sentence. New
  enums extend the existing ones (`ReasonCode`, `SafetyCondition`, `BrokerResultClass`)
  rather than inventing a parallel vocabulary.
- Every read model that can be stale carries `observed_at_unix_ms` and the
  `runtime_epoch` it belongs to, so the client can suppress a response that belongs to a
  previous account context or epoch.

Priority column: **P0** blocks all frontend application code; **P1** blocks a named
workspace; **P2** blocks a capability inside a workspace.

---

## BD-1 · Vox frontend transport — **P0**

Nothing else can be consumed without this. `vox-core` is currently a configuration binary;
`reqwest`/`tonic` in the workspace are provider-facing clients, not a Vox server.

Required:

1. A process that serves the existing `vox-runtime` read models over a local transport
   (HTTP/JSON is sufficient; gRPC-web is acceptable). It must be the only thing the UI
   talks to.
2. Request scoping: every read and every command carries the `RuntimeScope` triple
   (`provider`, `environment`, `broker_account_id`) plus `connection_ref`; the server
   rejects a request whose scope does not match an owned runtime with
   `STALE_EPOCH`/`OWNERSHIP_FAILURE`.
3. Responses carry `runtime_epoch` and `observed_at_unix_ms` at the envelope level.
4. A change stream (SSE or WebSocket) delivering `BrokerEvent`, `StreamHealth` and
   `RuntimeHealth` transitions, so the shell does not poll.
5. Endpoints, one per existing model, all account-scoped:
   `GET /runtime/health` → `RuntimeHealth`;
   `GET /runtime/scopes` → available `RuntimeScope` + `BrokerAccount`;
   `GET /accounts/{id}/snapshot` → `BrokerSnapshot`;
   `GET /accounts/{id}/orders`, `/stops`, `/positions`, `/portfolio`;
   `GET /accounts/{id}/operations?cursor=` → `OperationsPage`;
   `GET /accounts/{id}/mutations` → `MutationRecord[]`;
   `GET /accounts/{id}/reconciliation` → `ReconciliationCheckpoint`;
   `POST /accounts/{id}/commands/{order|cancel|replace|protection}` → returns
   `logical_request_id` and the resulting `JournalState`.
6. Serialization contract: the existing serde representation
   (`SCREAMING_SNAKE_CASE` enums, `OpaqueRef` transparent, `FixedPoint` as total nanos)
   is the wire format, and a schema artefact (OpenAPI or JSON Schema) is generated from
   the Rust types so the TypeScript client is generated, not hand-written.

Acceptance: a client can list scopes, read one account fully, submit a command, observe
`DISPATCHING → ACKNOWLEDGED|REJECTED|UNKNOWN_AFTER_DISPATCH`, and be told to stop when the
epoch changes.

Owner: **#11**.

## BD-2 · Risk read model — **P1** (Trade, Portfolio, Decision)

Nothing in the repository returns a risk verdict. The design carries the vocabulary
(`SAFE / WARNING / BLOCKED / UNKNOWN / RESIZE`) that #18 mandates, and the reference now
renders it as deferred.

Required:

1. `RiskVerdict { state, reason_code, reason, adjusted_quantity_lots: Option<i64>, evaluated_at_unix_ms }`
   where `state` is the five-value enum and `reason_code` extends `ReasonCode`.
2. Pre-trade evaluation for a constructed order: `POST /accounts/{id}/risk/preview` taking
   the same payload as the order command and returning `RiskVerdict` without dispatching.
3. Account risk state for the portfolio screen: exposure per instrument, concentration,
   day loss against its limit, margin usage — each as an exact value plus its limit, so the
   UI never computes a percentage from two unrelated numbers.
4. Guardrails as their own objects (`RiskGuardrail { scope, kind, limit, enforced }`),
   explicitly separate from account default protection (BD-9): a default is not a limit.
5. `RESIZE` must arrive as the backend's adjusted value; the browser never clamps.

Acceptance: submitting an order that violates a limit returns `BLOCKED` with the limit
named; one that can be reduced returns `RESIZE` with `adjusted_quantity_lots`.

Owner: unassigned — needs a risk-engine issue.

## BD-3 · Market-data read model — **P1** (Markets, Trade)

`vox-tinvest` has market data and streams, but `vox-runtime` exposes no market read model,
so quote, order book, tape and chart have nothing to consume.

Required:

1. `QuoteFact { instrument_uid, last, bid, ask, change_abs, change_pct, volume, observed_at_unix_ms }`
   with exact values.
2. `DepthFact { instrument_uid, bids: [{price, size, cumulative}], asks: [...], observed_at_unix_ms }`.
3. `TradeTick { instrument_uid, price, size, side, ts_unix_ms, own: bool }` as a stream.
4. `Candle { open, high, low, close, volume, ts_unix_ms }` over a range and timeframe, for
   the chart abstraction.
5. Instrument catalogue for the shared picker: `InstrumentIdentity` (already exists) plus
   lot size, min price increment, trading status and session state — the ticket needs lot
   and step metadata to validate quantity without inventing rules.
6. Session/market status per venue.

Acceptance: the Markets watchlist and the Trade quote strip render from Vox alone, and the
ticket's lot/step validation is metadata-driven.

Owner: **#8** (market data) extended into a runtime read model.

## BD-4 · Portfolio valuation and P&L — **P1** (Portfolio, Trade)

`PortfolioFact` today is `currencies: Map<code, string>`. Position facts carry quantity
only. Every money figure on the portfolio screen is therefore unbacked.

Required:

1. `PositionValuation { instrument_uid, quantity_units, average_price, current_price, unrealized_pnl, currency, observed_at_unix_ms }`.
2. `AccountValuation { total_value, cash, realized_pnl_day, unrealized_pnl, margin_used, margin_available, currency }`.
3. Operation amounts: extend `OperationFact` with `kind` (buy/sell/fee/dividend/transfer),
   `amount`, `currency` — the identity-only shape cannot render an operations table.
4. Protection coverage per position: which of `PositionFact` is protected, by which
   `StopFact`, and the state of that protection, so the portfolio can show
   protected / partially protected / unprotected / unknown without deriving it in the UI.

Acceptance: the portfolio screen shows value, P&L and coverage without a single computed
guess, and every figure is exact.

Owner: unassigned — depends on BD-3 for pricing.

## BD-5 · Account, credential and RBAC API — **P1** (Settings → Brokers & Accounts, Users)

`CredentialResolution { execution_authorized }` and `OpaqueRef` exist. The lifecycle around
them does not.

Required:

1. Connections: `BrokerConnection { connection_ref, provider, environment, label, health, capabilities, created_at, last_checked_at }`
   with create / validate / rename / disable / re-enable / delete.
2. Credential lifecycle without exposing the secret: submit new secret, validate, rotate,
   and read back only `fingerprint`, `expires_at`, `scopes` and a lifecycle state that
   distinguishes **rejected**, **revoked by owner**, **scope-limited** and **expiring** —
   the three states the reference currently marks deferred.
3. Account discovery and selective binding: discovered `BrokerAccount` list plus a Vox-side
   binding record, with per-account `execution_authorized`.
4. Execution authorization as its own object: `ExecutionAuthorization { scope, enabled, actor, changed_at, audit_ref }`
   with enable/disable, so the UI shows a fact instead of inferring one.
5. RBAC read model: `Role`, `Permission`, current actor's effective permissions, and a
   denial shape that survives a stale UI (`403` carrying the permission name).

Acceptance: the settings screen can add a sandbox and a production connection to the same
provider, discover accounts, bind two of them, rotate a credential, and never receive the
stored secret back.

Owner: **#17**.

## BD-9 · Protection defaults — **P2** (Trade, Portfolio, Settings)

`ProtectionPlan` and `ProtectionCapability` exist per command. Account-scoped defaults and
the precedence resolution do not.

Required:

1. `ProtectionDefault { scope, stop_loss: Option<StopLossProtection>, take_profit: Option<TakeProfitProtection>, applies_to_manual: bool, applies_to_strategy: bool }`
   with read and update.
2. Backend-resolved effective protection for a given order or position:
   `EffectiveProtection { plan, source: ORDER | STRATEGY | ACCOUNT_DEFAULT | NONE }` —
   the UI renders the source badge and never computes precedence.
3. Broker-reported protection runtime already has `ProtectionEstablishmentState` and
   `ProtectionLifecycle`; expose them per position, plus the age of the last broker answer.

Acceptance: the ticket shows `Плавающий стоп 1,00 % · Источник: заявка` from a backend
field, and changing an account default does not alter any existing stop order.

Owner: **#10**.

## BD-10 · Bulk protection migration — **P2** (Portfolio, Settings)

Only single mutations exist (`MutationKind`, `MutationRecord`).

Required:

1. Preview: `POST /accounts/{id}/protection/migrate/preview` returning, per position, the
   current broker order, the proposed one, and a per-position disposition
   (`REPLACE | CREATE | SKIP_MANUAL_OVERRIDE | UNSUPPORTED`).
2. Apply: the same payload with a confirmation token, returning one `logical_request_id`
   per position so each row resolves through the normal journal states — including
   `UNKNOWN_AFTER_DISPATCH`.
3. An explicit statement in the contract that positions carrying a manual override are
   never touched.

Acceptance: preview changes nothing at the broker; apply produces per-position results, and
a failure of one position never silently succeeds as a whole.

Owner: **#10**.

## BD-6 · Strategy and decision contracts — **P1** (Strategy, Decision)

No types exist.

Required:

1. `Strategy { id, name, scope, instrument_universe, mode: ADVISORY | APPROVAL_REQUIRED | AUTONOMOUS | STOPPED, state, schedule, protection_policy_ref }`
   with start / pause / stop, and an explicit rule that a strategy cannot grant itself
   execution authorization (BD-5 owns that switch).
2. `StrategySignal { strategy_id, instrument_uid, ts, payload_summary }` as a stream.
3. `DecisionCandidate { id, scope, instrument_uid, side, quantity_lots, protection_plan, confidence, rationale, model_ref, expires_at, risk_verdict_ref }`
   with approve/reject, where approval creates an ordinary order command subject to BD-2
   and BD-5.
4. Audit records for both, reusing `RuntimeAuditRecord`.

Acceptance: the Decision queue renders candidates and approves one through the same command
path as a manual order.

Owner: unassigned.

## BD-7 · ML / model contracts — **P1** (ML / Models, Settings → ML)

No types exist.

Required:

1. `Model { name, version, state: CANDIDATE | VALIDATING | PRODUCTION | ARCHIVED | FAILED, dataset_ref, trained_at, metrics: Map<String, String>, drift }`.
2. `TrainingJob { id, model_ref, dataset_ref, instrument_universe, progress, state, logs_ref }`.
3. `Dataset { id, source, instrument_universe, period_start, period_end, rows }`.
4. Promotion and rollback as commands with an audit record, and a contract statement that
   promotion never implies execution authorization.
5. Instrument selection by `InstrumentIdentity`, never by raw internal id.

Acceptance: the registry lists models with real metrics, and promoting a candidate is an
audited command that leaves execution authorization untouched.

Owner: unassigned.

## BD-8 · Research / backtest contracts — **P1** (Research)

No types exist, and `RuntimeEnvironment` has no `BACKTEST` variant.

Required:

1. `BacktestRun { id, strategy_ref, instrument_universe, period, protection_plan, state, progress, failure_reason }`.
2. `BacktestResult { equity_curve, drawdown, pnl, win_rate, trade_count, trades }` with exact
   money values.
3. Dataset availability per instrument and period, so a run that cannot be executed is
   refused with a reason instead of producing a partial curve presented as complete.
4. A research environment marker distinct from `RuntimeEnvironment`, so a research result
   can never be mistaken for portfolio state.

Acceptance: a run either completes with a full result or fails with a named reason; no
partial curve is ever shown as final.

Owner: unassigned.

## BD-11 · Aggregate accounts read model — **P2** (`Все счета`)

Required: a read-only aggregate over bound accounts — positions by instrument across
accounts, aggregate valuation, and an explicit flag that the aggregate scope is
**not executable**. Until it exists, the UI keeps the mode disabled; there is no
aggregate execution contract and the frontend must not synthesise one.

Owner: unassigned.

## BD-12 · Application version, updates and background jobs — **P2** (System, Settings)

Required: current version, available version, update state, restart-required flag,
maintenance state, update history, and a list of background jobs with their state. No field
in normal UI may accept a Git URL or a shell command.

Owner: unassigned.

## BD-13 · Second provider and additional environments — **P2**

`Provider` has one variant and `RuntimeEnvironment` has two. Multi-provider and
`PAPER`/`BACKTEST` shapes are designed and currently rendered disabled. Adding a provider
means extending `Provider` and the capability metadata; adding an environment means
extending `RuntimeEnvironment` and every scope key derived from it.

Owner: unassigned.

---

## Order of work

```
BD-1  (transport)                    ← blocks everything
  ├─ BD-5  (accounts/credentials/RBAC)   → shell, settings, execution authorization
  ├─ BD-3  (market data)                 → Markets, Trade quote/book/tape/chart
  ├─ BD-4  (valuation/P&L)               → Portfolio, positions economics
  ├─ BD-2  (risk)                        → ticket verdict, guardrails, decision approval
  ├─ BD-9/BD-10 (protection defaults/bulk)
  └─ BD-6/BD-7/BD-8 (strategy, ML, research)
BD-11, BD-12, BD-13 — after the workspaces they belong to.
```

Frontend application code cannot begin before **BD-1**, because there is no transport to
generate a typed client from. Everything after BD-1 unblocks a specific workspace, and the
frontend can proceed workspace by workspace in the order above.
