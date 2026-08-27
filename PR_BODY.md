# feat(frontend): Vox Trader design system — canonical terminal, protection and execution safety

Presentation/design-system only. No broker adapter, execution, risk runtime, persistence
or strategy code is touched, and nothing in this branch is imported by the application at
runtime.

This revision closes the Head-of-Development directive on this PR and the design half of
issue #18.

## Live preview

- Reference: <https://ultima-vox.github.io/trader/reference/index.html> (published by the Design Preview workflow)
- Compliance audit: [`docs/design/COMPLIANCE_AUDIT.md`](docs/design/COMPLIANCE_AUDIT.md)

The reference renders identically from `file://` — no build step, no CDN, no account.

## What is in the branch

```text
docs/design/
  VOX_TRADER_DESIGN_SYSTEM.md   normative rules
  COMPONENT_SPEC.md             anatomy, variants, states per component
  COMPLIANCE_AUDIT.md           directive-by-directive audit against #18 and this review
frontend/design-system/
  tokens/       tokens.css + tokens.json
  primitives/   primitives.css
  components/   components.css
  patterns/     patterns.css
  reference/    index.html — the rendered reference (23 sections)
```

Conflict order: normative docs > CSS layers > `reference/index.html` > viewing wrapper.
`vox-trader-design-system.reference.html` is only a stable entry point for viewing the
reference; it is not a separate authority, and the Claude Design document under
`reference/source/` is provenance, not a build input.

## Canonical production screens

The reference is no longer a set of isolated component demos. §9 is the canonical
**Trading workspace** — shell with environment, AccountSelector, runtime state, Vox
execution state, P&L and clock; nav rail; a 12-column workspace holding quote, chart,
order ticket, order book, positions, tape and portfolio metrics — and every remaining
workspace is drawn on the same shell and grid:

| § | Screen | § | Screen |
| --- | --- | --- | --- |
| 9 | Trading workspace | 17 | Portfolio, operations, event journal |
| 10 | Order ticket · execution target | 18 | Strategy |
| 11 | Position protection | 19 | Decision Center |
| 12 | Reconciliation and `UNKNOWN` | 20 | Research |
| 13 | Bulk protection migration | 21 | ML / Models |
| 14 | Execution authorization | 22 | System |
| 15 | Brokers and accounts | 16 | Markets |

A widget catalogue in §9 declares minimum and preferred sizes and states what each widget
drops when squeezed: instrument, account, environment, P&L and protection never drop.

## Directive items closed in this revision

- **Order Ticket execution target** as the first row, never collapsed, never inferred from
  the last used account, with an explicit mismatch state (`EXEC_TARGET_MISMATCH`).
- **Frozen submitted-command target** — a dispatched command keeps its broker, account and
  environment. Switching the active account afterwards updates account-scoped views and
  can never retarget that command; cancel and re-send are two separate operator actions.
- **Broker-authoritative protection runtime states** — exactly `ACTIVE`, `STALE`,
  `RECONCILING`, `TRIGGERED`, `CANCELLED`, alongside current level, reference level
  (high-water for long, low-water for short), activation condition and the broker's answer
  time. Unreported fields stay `UNKNOWN`. The terminal never recomputes or smooths a level,
  and `CANCELLED` always names the reason and states that the position is now unprotected.
- **`UNKNOWN_AFTER_DISPATCH` and reconciliation UX** — silence age, the facts Vox knows for
  certain, re-dispatch blocked until `RECON_CONFIRMED` / `RECON_NOT_FOUND` /
  `RECON_PENDING` resolves it. Never rendered as a failure.
- **Bulk protection migration** as a separate capital-affecting flow: preview, affected
  count with a breakdown, per-position `было → станет` in broker order terms, consequences
  including the unprotected window, typed confirmation, and per-position results including
  `ОТКЛОНЕНО` and reconciliation. Manually overridden positions are never touched.
- **Execution authorization** — broker token capability and Vox execution shown as separate
  facts, `PRODUCTION` off by default, scoped to account and environment, typed confirmation
  to enable, one-press halt, backend audit metadata. Strategy, ML and Decision Center link
  to it and can never grant it.
- **Default vs risk guardrail** — separated visually, with backend `BLOCKED` and `RESIZE`
  verdicts rendered rather than silently clamped in the browser.
- **Connection vocabulary** — `CONNECTED`, `VALIDATING`, `RECONNECTING`, `DEGRADED`,
  `INVALID_CREDENTIAL`, `REVOKED`, `PERMISSION_LIMITED`, `ROTATE`, `PROVIDER_UNAVAILABLE`,
  `DISABLED`, `UNKNOWN`, each with its own action and reason code. Plus multiple
  connections per provider, connection lifecycle actions, three-step token rotation that
  never reads a stored secret back, and provider capability gating.

## Review blockers from the previous round

- **Item 16 — token-only cleanup: closed.** No raw HEX and no raw `rgba()` outside
  `tokens/tokens.css`, including the viewing wrapper.
- **Item 17 — canonical wording: closed.** Wording implying the wrapper is the original
  self-contained Claude export or an independent authority is removed from the docs, the
  README and the wrapper itself.

## Scope after the ownership decision

#18 — and therefore this PR — owns frontend **foundation and infrastructure**: the design
system, the shell, account/instrument context, typed state, the generated Vox client,
atomic context switching, stale-response suppression, frozen command targets and the shared
components. Production operator workspaces and screen composition belong to **#30**, which
builds on this infrastructure over stable backend contracts and is blocked on #21–#29.

The workspace designs in the reference stay here as the canonical visual specification for
those screens; implementing them is #30's work.

Frontend application code does not start before **#38 — Application API Foundation**
publishes the first generated Vox TypeScript client: the repository has canonical Rust
contracts but no Vox transport, so a typed client could only be invented. This PR therefore
delivers the design system, the contract conformance pass and the backend dependency
specification, and #18 stays open for the infrastructure work that follows #38.

## Contract conformance

Every state word, environment and reason code rendered on a working screen is now a value
from `vox-domain` or `vox-runtime`: `SANDBOX`/`PRODUCTION` environments, the single
`Provider` variant, all eight `RuntimeState` values, nine canonical reason codes, and
`ProtectionEstablishmentState` for protection runtime with the operator vocabulary mapped
onto it. Eleven invented codes were removed.

Fifteen regions whose capability has no contract render **deferred** — named, disabled and
tagged with a `BD-*` dependency — instead of simulating data: risk verdict and guardrails
(BD-2), quotes/book/tape/chart (BD-3), valuation and P&L (BD-4), credential lifecycle and
RBAC (BD-5), strategy and decision (BD-6), models (BD-7), research (BD-8), protection
defaults (BD-9), bulk migration (BD-10), aggregate accounts (BD-11), updates and jobs
(BD-12), second provider and PAPER/BACKTEST (BD-13).

`docs/design/BACKEND_CONTRACTS.md` records what the contracts expose today;
`docs/design/BACKEND_DEPENDENCY_SPEC.md` specifies what each backend owner must add, with
acceptance criteria, and carries the definition of done for #18.

## Visual verification

A strict per-widget verifier — painted geometry, nine rules, 1280/1440/1920 ×
Compact/Standard/Comfortable — reports **zero findings**, and four injected defects are
still detected, so the zero is meaningful. The first run of that verifier found 663
findings, all fixed in the layers.

## Compliance position

`docs/design/COMPLIANCE_AUDIT.md` maps every issue #18 directive and all nineteen review
items to a section of the rendered reference, and carries a screen-by-screen Design DoD
table for all fourteen screens: account named, environment shown, non-happy states
covered, reason codes present, and an explicit target for capital-affecting actions.

Outstanding, all stated in that document:

1. Responsive rendering check at 1280 / 1440 / 1920 in Compact / Standard / Comfortable —
   the reference declares and is built to the targets; a human still has to look.
2. On-candle marker geometry cannot be demonstrated while the chart region is a
   placeholder; a chart engine is an explicit non-goal of #18. The letter vocabulary,
   grouping, tooltip contents and the marker-versus-price-line distinction are specified.
3. `Все счета` read-only aggregate — deferred by #18 until a backend aggregate execution
   contract exists.
4. The implementation half of #18 — typed Vox API clients, atomic account-context
   switching, stale-response guards and the test matrix. The design contract for each is
   written; the code is not in this branch.

On the design side no directive of issue #18 or of this review is unrepresented.

Refs #18
Refs #10
Refs #11
Refs #17
