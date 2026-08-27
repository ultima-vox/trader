# Vox Trader — Design System (frontend)

Reference implementation of the Vox Trader terminal UI. Plain CSS + static HTML, no
build step, no package manager, no network access required.

Normative documentation lives in [`../../docs/design/`](../../docs/design/):
`VOX_TRADER_DESIGN_SYSTEM.md` (rules) and `COMPONENT_SPEC.md` (anatomy, variants,
states).

## View locally

```bash
# from the repository root
open frontend/design-system/reference/index.html                          # macOS
xdg-open frontend/design-system/reference/index.html                      # Linux
start frontend\design-system\reference\index.html                        # Windows

# or serve the tree (any static server works)
python3 -m http.server 8080
# → http://localhost:8080/frontend/design-system/reference/index.html
```

Published: <https://ultima-vox.github.io/trader/>

Both files open directly from `file://`. All paths are relative; nothing is fetched
from a CDN, and no Claude Design runtime or account is needed.

## What is canonical

| File | Role |
| --- | --- |
| `reference/index.html` | **The one rendered reference.** Hand-maintained, built on the CSS layers below, and extended whenever a component or a state is added. Published by GitHub Pages. |
| `../../docs/design/source/Vox-Trader-Design-System-canonical.html` | **Visual and component authority.** Restored from the committed source with `restore-canonical-design.sh`; immutable provenance, never a runtime dependency and never imported. |
| `tokens/`, `primitives/`, `components/`, `patterns/` | **Source of truth for implementation.** Hand-maintained CSS layers. |
| `../../docs/design/*.md` | **Source of truth for rules.** Documentation beats any rendering. |

Conflict resolution for **semantics**: normative docs > CSS layers > `reference/index.html`.
For **visual language and geometry** the canonical source in `docs/design/source/` wins, per
`docs/design/CANONICAL_DESIGN.md`. There is exactly one executable design system and one
rendered reference; no second entry point exists.

## Layers

```
tokens/       tokens.css, tokens.json   colour, type, space, radius, motion, density
primitives/   primitives.css            Text, Number, Icon, Divider, Surface, Stack, Scroll, Tooltip, Popover
components/   components.css            Button, Input, NumericInput, Select, Tabs, Segmented, Checkbox,
                                        Switch, Slider, Menu, Badge, Env, Runtime, Risk, Marker, Table, Widget
patterns/     patterns.css              AppShell, Workspace, InstrumentHeader, OrderBook, TradeTape,
                                        OrderTicket (+ execution target, frozen target), Protection,
                                        TrailingReadback, Precedence, ProtectionPolicy, Reconciliation,
                                        BulkProtectionMigration, ExecutionAuthorization, BrokersSettings,
                                        PortfolioSummary
```

Import order is fixed and one-directional — a lower layer never depends on a higher one:

```html
<link rel="stylesheet" href="tokens/tokens.css">
<link rel="stylesheet" href="primitives/primitives.css">
<link rel="stylesheet" href="components/components.css">
<link rel="stylesheet" href="patterns/patterns.css">
<body class="vox-root" data-density="compact" data-theme="dark">
```

`tokens.json` is the machine-readable mirror for future codegen (TS types, Figma sync).
Change it in the same commit as `tokens.css`.

## Non-negotiables (short form)

- Dense professional terminal. **Compact is the production default density.**
- **Russian primary UI language**; Latin only for tickers, technical states
  (`LIVE`, `READY`, `HALTED`), reason codes and marker letters.
- Order ticket has **permanent dual actions** `Купить` / `Продать`. **No buy/sell mode
  toggle exists.** A forbidden side stays visible and states its reason.
- Chart/tape/journal markers are exactly **B / S / F / SL / TP / D / E**.
- Environment is `SANDBOX` or `PRODUCTION` — the only two values `RuntimeEnvironment`
  has. Runtime is one of the eight `RuntimeState` values, and new exposure is allowed in
  `READY` alone. Risk keeps the `SAFE / WARNING / BLOCKED / UNKNOWN / RESIZE` vocabulary
  required by #18, but no risk contract exists yet, so the verdict renders deferred
  (`BD-2`). `UNKNOWN` is never rendered as an error.
- Widgets are draggable by the header, resizable in 8px steps, and carry a linked or
  pinned instrument context chip.
- No oversized generic SaaS cards. No raw broker identifiers in normal UI.
- Tokens only: a raw HEX **or raw `rgba()`** outside `tokens/tokens.css` is a review
  blocker.
- Order ticket protection: **independent, optional Stop Loss and Take Profit**. Stop
  Loss supports `Фиксированный` and broker-native `Трейлинг` (relative % and
  provider-supported absolute), shows current/reference level where the broker reports
  it, and always names the resulting broker order. No client-side emulation of broker
  protection (execution semantics: issue #10).
- Default protection policy lives at portfolio/account scope (incl. a global trailing
  default). Precedence is visible and fixed: **order/position override > strategy
  policy > portfolio/account default**. A default is not a hard risk limit — guardrails
  are a separate policy — and changing a default never silently rewrites existing
  broker stop orders.
- **A broker token that permits trading does not enable Vox live execution.** PRODUCTION
  execution is off by default (`.vox-exec-auth`): enabling is a high-friction flow
  (account · broker · environment · scope · consequences · typed confirmation), halting is
  one step. Strategy, ML and Decision Center can never bypass this authorization.
- Connection state uses contract values only: `CONNECTED`, `CONNECTING`, `DEGRADED`,
  `CREDENTIAL_REJECTED`, `EXECUTION_UNAUTHORIZED`, `REQUIRED_READ_UNAVAILABLE`,
  `ACCOUNT_UNAVAILABLE`, `UNKNOWN`. Failures are never collapsed into one generic red
  state. Credential validation, revocation and rotation are a lifecycle the contract does
  not have yet and render deferred (`BD-5`).
- **A capability without a contract renders deferred, never simulated**: `.vox-deferred`
  names it, disables it and carries its `BD-*` id. See `docs/design/BACKEND_CONTRACTS.md`
  and `docs/design/BACKEND_DEPENDENCY_SPEC.md`.
- **AccountSelector is always visible** in the shell, and the order ticket states its
  execution target (broker · account · environment) as its first row. A token is not an
  account and not a portfolio; a stored token is never revealed in normal UI
  (connections, secret storage, discovery, authorization: issue #17).
- **A submitted command freezes its target.** Switching the active account afterwards
  updates account-scoped views but never retargets that command; cancel and re-send are
  two separate operator actions.
- A dispatch without a broker answer is `UNKNOWN_AFTER_DISPATCH`: violet unknown
  semantic, silence age stated, re-submission blocked until reconciliation answers
  (`RECON_CONFIRMED` / `RECON_NOT_FOUND` / `RECON_PENDING`, issue #11). Never red.
- Protection readback is broker-authoritative. Runtime states are exactly `ACTIVE` /
  `STALE` / `RECONCILING` / `TRIGGERED` / `CANCELLED`, alongside current and reference
  level, activation and the broker's answer time. Unreported fields stay `UNKNOWN`; the
  terminal never recomputes a level. `CANCELLED` always states the reason and that the
  position is now unprotected.
- Re-applying a default to existing positions is a separate capital-affecting flow:
  preview, affected count, per-position `было → станет`, consequences, typed
  confirmation, per-position result including `ОТКЛОНЕНО` and reconciliation.

## Adding a component

1. Write the spec entry in `../../docs/design/COMPONENT_SPEC.md` (anatomy, variants, states).
2. Add CSS to the correct layer, tokens only.
3. Add a demo block to `reference/index.html` covering every state.
4. Validate `reference/index.html` at 1280 / 1440 / 1920 px, at Compact, Standard and
   Comfortable, and in every state: happy, loading, empty, stale, reconnecting,
   degraded, error, permission-denied, `UNKNOWN`, `BLOCKED`.

## Out of scope here

No JS framework bindings, no React/Vue component library, no runtime or trading logic.
This directory is presentation only.
