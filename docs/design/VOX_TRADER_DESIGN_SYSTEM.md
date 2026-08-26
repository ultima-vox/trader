# Vox Trader — Design System

Version 1.0 · status: foundation · owner: frontend
Scope: the Vox Trader trading terminal UI. This document is **normative**. Where an
implementation disagrees with it, the implementation is wrong.

Related files

- `COMPONENT_SPEC.md` — anatomy, variants and states per component.
- `../../frontend/design-system/reference/index.html` — **the maintainable rendered reference**, built on the CSS layers and extended whenever a component or state is added.
- `../../frontend/design-system/reference/vox-trader-design-system.reference.html` — a stable local viewing entry point for that reference. It is not a separate design authority.
- `../../frontend/design-system/README.md` — how to view locally, layer map.

---

## 1. Product premise

Vox Trader is a **dense professional trading terminal**, not a consumer fintech app
and not a generic SaaS dashboard. The operator watches many numbers at once, acts in
seconds, and is accountable for money. Every design decision follows from that.

Consequences, applied everywhere:

1. **Information density is a feature.** Screen area spent on decoration is area
   taken from data. Whitespace is not a value in itself.
2. **Compact is the production default density.** Standard and Comfortable exist as
   an accessibility preference, never as the shipped default.
3. **Russian is the primary UI language** (`ru-RU`). Latin script survives only for
   tickers (`SBER`), technical states (`LIVE`, `READY`, `HALTED`), reason codes
   (`RISK_DAY_LOSS`) and event marker letters.
4. **Nothing money-related is implicit.** Environment, runtime health, risk verdict
   and staleness are always on screen, in words, not inferred from a colour.
5. **Uncertainty is a first-class state.** `UNKNOWN` is not `FAILED`. It has its own
   token, its own copy, and never renders red.

## 2. Language and copy rules

| Rule | Do | Don't |
| --- | --- | --- |
| UI language | `Купить`, `Продать`, `Снять заявку`, `Объём` | `Buy`, `Sell`, `Cancel` |
| Numbers | space thousands separator, comma decimal: `1 284 730,45 ₽` | `1,284,730.45` |
| Technical states | `LIVE`, `SANDBOX`, `READY`, `RECONCILING`, `DEGRADED`, `HALTED` (Latin, uppercase, never translated) | `Готово`, `Живой` |
| Reason codes | shown next to human text: `Превышен дневной лимит убытка · RISK_DAY_LOSS` | code alone |
| Identifiers | account and instrument by human name: `Счёт «Основной»`, `SBER · Сбербанк` | raw broker/account/order ids in normal UI |
| Errors | say what happened, what it blocks, what to do next | `Ошибка 500` |

Raw broker identifiers, order ids and account numbers appear **only** in the
diagnostics panel and in explicit "copy technical details" actions — never in tables,
tickets, headers or toasts.

## 3. Tokens

Single source of truth: `frontend/design-system/tokens/tokens.css` (+ machine-readable
`tokens.json`). Components consume tokens only; a raw HEX in a component file is a
review blocker.

### 3.1 Colour semantics

| Meaning | Token | Note |
| --- | --- | --- |
| Growth / long / `Купить` | `--vox-positive-*` | green |
| Decline / short / `Продать` | `--vox-negative-*` | red |
| Warning, near-limit, stale | `--vox-warning-*` | amber |
| Informational, own orders | `--vox-info-*` | blue |
| **UNKNOWN / unreconciled** | `--vox-unknown-*` | violet — never red |
| Accent, selection, focus | `--vox-accent-primary` | blue. **Accent never means BUY.** |

Surfaces: `bg-canvas` → `bg-workspace` → `bg-surface` → `bg-elevated`. Dark Terminal
is the product theme; light tokens exist for parity only. Adjacent regions are
separated by a 1px border first, not by a large gap.

### 3.2 Type

`--vox-font-ui` (Inter → system) for UI, `--vox-font-mono` (JetBrains Mono → system)
for codes, timestamps and shortcuts. Scale: 11 / 12 / **13 = base** / 14 / 16 / 18 /
22 / 28. Every market number goes through `.vox-num`: `font-variant-numeric:
tabular-nums`, right-aligned, `white-space: nowrap` — so streaming values never
reflow the row.

### 3.3 Space, radius, elevation, motion

- 4px base unit; 6px workspace gutter; 8px minimum widget resize step.
- Radius: 2 / 4 (control) / 6 (widget) / 8 (max). Nothing rounder.
- Shadows on overlay layers only (menu, tooltip, popover, modal). **Widgets have no shadow.**
- Motion: 80 / 120 / 180 ms; price flash 480 ms; `prefers-reduced-motion` respected.
  No slide-ins, no parallax, no decorative transitions.

### 3.4 Density

| Density | Control | Row | Table header | Widget header | Use |
| --- | --- | --- | --- | --- | --- |
| **Compact (default)** | 28px | 26px | 28px | 32px | production |
| Standard | 32px | 30px | 32px | 36px | user preference |
| Comfortable | 36px | 36px | 36px | 36px | accessibility |

Set via `data-density` on the app root. Density changes token values only — never
layout structure, column count or which information is shown.

## 4. Layout model

- **Shell**: 44px top bar → (118px nav rail | workspace).
- The top bar answers five questions without navigation: where am I · which broker ·
  which account · can I trade right now · what is the portfolio doing.
- **Workspace**: 12-column grid, `grid-auto-rows: minmax(48px, auto)`, 6px gap.
- **Widgets are draggable and resizable**: drag by the header (`cursor: grab`), resize
  from the bottom-right handle in 8px steps, drop target shown as a dashed accent
  outline. Layout is persisted per workspace and per user. A pinned widget is not
  draggable (`is-pinned`).
- **Instrument context** is per widget and either *linked* (follows the workspace
  selection, `.is-linked`) or *pinned* (fixed to one instrument, `.is-pinned`). The
  current context is always visible as a chip in the widget header — a widget must
  never show data whose instrument the user cannot name from the header.
- **Every widget declares a minimum and a preferred size** (width in grid columns,
  height in 8px steps) and what it drops first when squeezed. Nothing that carries
  money may be dropped: instrument, account, environment, P&L and protection survive
  every resize; legends, secondary metrics and row counts go first. The Order Ticket
  does not shrink below its minimum at all — execution target, protection and both
  actions are mandatory. The catalogue lives in `reference/index.html` §9.

### 4.1 Workspace inventory

One widget system, several saved layouts. The product has these workspaces, all built
from the same shell and the same 12-column grid:

| Workspace | Purpose | Capital-affecting |
| --- | --- | --- |
| Рынки | watchlist, instrument search, session state | no |
| Торговля | quote, chart, ticket, book, tape, positions, portfolio | **yes** |
| Портфель | positions, exposure, limits, operations, event journal | no |
| Стратегии | strategy list, account binding, protection policy, mode | **yes** (via authorization) |
| Решения | candidates/intents, evidence, risk verdict, approval | **yes** |
| Исследования | backtests, parameters, comparison — `BACKTEST` only | no |
| ML / Модели | datasets, training, registry, validation, promotion | no |
| Система | runtime diagnostics, emergency halt, audit, permissions | **yes** (halt) |
| Настройки → Брокеры и счета | connections, secrets, discovery, defaults, authorization | **yes** |

A capital-affecting workspace always states broker, account and environment on the
screen itself; a non-capital-affecting one still names the account whose data it shows.
The event journal reuses the marker letters `B/S/F/SL/TP/D/E` unchanged — it is the same
vocabulary as the chart, the tape and the orders table, never a second one.

## 5. Trading semantics

### 5.1 Dual order actions

The order ticket has **one shared body and two permanent actions**: `Купить` and
`Продать`, side by side, each showing its own executable price.

- There is **no buy/sell mode toggle** anywhere in the product. The side *is* the
  final action; a mode toggle adds a hidden state between intent and execution.
- Keyboard maps straight to actions: `B` = Купить, `S` = Продать, `Esc` = reset.
- If a side is not permitted, it **stays in place**, visually recessed
  (`.is-blocked`), with the reason next to it. It is never hidden or removed —
  the restriction concerns execution, not the concept of the widget.

### 5.2 Event markers

One letter language, reused identically in the chart, the tape, the orders table and
the event journal: **B** buy · **S** sell · **F** fill · **SL** stop-loss ·
**TP** take-profit · **D** dividend · **E** event. 14px badge, 9px semibold letter.
Pictogram icons may not replace the letters.

### 5.3 Environment

`LIVE` / `SANDBOX` / `PAPER` / `BACKTEST` is always visible in the top bar as a
labelled badge — never a bare coloured dot. In `LIVE`, irreversible actions carry an
inset red hairline (`.vox-live-action`); the interface as a whole does not turn red.
Environment can only be switched deliberately, and never while an order form is dirty.

### 5.4 Runtime states

| State | Meaning | Trading |
| --- | --- | --- |
| `READY` | streams and broker session healthy | allowed |
| `RECONCILING` | positions/orders being reconciled | allowed, values may be `UNKNOWN` |
| `DEGRADED` | partial data or slow broker | allowed with an explicit caveat |
| `HALTED` | trading stopped by runtime or risk | new orders blocked; cancel stays available |

The runtime chip is clickable and opens diagnostics (last heartbeat, stream lag,
broker session, reconcile queue). Widget-level health is separate: `loading`, `live`,
`stale`, `reconnecting`, `degraded`, `empty`, `error`, `permission-denied`.

### 5.5 Risk states

`SAFE` / `WARNING` / `BLOCKED` / `UNKNOWN` / `RESIZE`. Always icon + human sentence +
reason code. `BLOCKED` disables submission and says which limit; `RESIZE` states the
old and new quantity (`120 → 80`); `UNKNOWN` says the decision is deferred, not
refused, and offers a retry. Colour is never the only carrier of meaning.

### 5.6 Data freshness

Streaming values flash for 480 ms on change (background only, never layout). A widget
older than its freshness budget switches to `is-stale`: amber border plus a stale bar
naming the age and stating that last known values are shown. Stale data is never
silently displayed as current.

### 5.7 Position protection — Stop Loss / Take Profit

Protection is part of the canonical Order Ticket, not a separate advanced screen.

- **Stop Loss and Take Profit are independent and optional.** Enabling one never
  requires the other. Either may be off; that is a normal ticket.
- **Stop Loss has two modes: `Фиксированный` and `Трейлинг`.** Trailing is
  **broker-native**: it maps to the provider's own trailing-stop order.
- Trailing offset supports **relative %** and, where the provider supports it,
  **absolute** currency distance. If the provider does not support a mode, the UI
  **states that** with a reason code (`BRK_TRAIL_ABS_UNSUPPORTED`) and refuses the
  mode. Client-side emulation of broker protection is prohibited: protection must
  survive a closed terminal.
- Where the broker reports it, the ticket and the position show the trailing
  **current level** and **reference level** (the extreme the trail is measured from).
  A level the broker does not report is `UNKNOWN` — never an error, never 0.
- Every protection control names the **resulting broker order** (`STOP_LOSS`,
  `TAKE_PROFIT`, `TRAILING_STOP`) and the level in both absolute price and distance.
  Protection UI that does not map to an execution object owned by
  **Broker Foundation 05 — orders, stop orders, sandbox parity and execution streams
  (issue #10)** is decoration and is not allowed to ship.
- Protection state follows the same runtime language as any order: `READY`,
  `RECONCILING`, `DEGRADED`, `HALTED`, plus `UNKNOWN` when the broker has not
  answered yet.
- **The broker is the authority on a live trailing stop.** The readback shown in the
  ticket and on the position is whatever the provider last confirmed: order state
  (`ACTIVE`, `PENDING`, `REPLACED`), current level, reference level, activation
  condition and the time of the broker's answer. The terminal never recomputes a
  level, never smooths a discrepancy and never fills a gap with a plausible number.
- Direction semantics are displayed, not enforced, by the UI: for a long position the
  level follows the favourable high-water mark and never widens downward when price
  falls; for a short it mirrors against the favourable low-water mark. A field the
  broker does not report stays `UNKNOWN`.

### 5.8 Protection defaults and precedence

A default protection policy exists at **portfolio/account scope**, including a global
trailing default.

Precedence is fixed, explicit and **visible in the UI**:

```
order / position override   >   strategy policy   >   portfolio / account default
```

- The effective value is marked as effective; the values it overrides stay readable
  (struck through), never hidden. A value that is not the object's own is labelled as
  inherited (`.vox-inherited`); editing it locally creates an explicit override.
- **A default is not a hard risk limit.** Guardrails (hard limits that can refuse an
  order) are a separate policy with their own screen, their own reason codes and their
  own `BLOCKED` semantics. Never present a default as a limit or a limit as a default.
- **Changing a default never silently rewrites existing broker stop orders.** The
  change applies to new orders; affected existing orders are migrated only through an
  explicit action that lists them. Silent server-side rewriting of live protection is
  prohibited.
- Where the backend exposes bulk re-application, it is a **separate capital-affecting
  action** with a fixed anatomy: preview before anything is sent · count of affected
  positions with a breakdown · per-position `было → станет` stated in broker orders ·
  consequences in words, including the window in which a position carries no
  protection · typed confirmation that keyboard shortcuts cannot bypass · and a
  per-position result, including `ОТКЛОНЕНО` and reconciliation. Positions with a manual
  override are never touched by a bulk action, and a single aggregate "done" is not an
  acceptable result screen.

### 5.9 Execution target — broker, account, environment

Capital-affecting commands always state where they go.

- An **AccountSelector** is permanently visible in the shell: broker connection ·
  account (human label) · environment · connection health.
- The **Order Ticket shows its execution target as its first row**, above the
  instrument. It is never collapsed, never inferred from "the last used account".
  A target that differs from the workspace selection is shown as a mismatch, not
  silently accepted.
- The visual model must make it **impossible to confuse which account receives a
  capital-affecting command**: LIVE targets carry the inset LIVE marker on the target
  row, and the account label is a human name — never a raw identifier.
- **A token is not a portfolio and not an account.** One connection may discover
  several accounts; execution permission is a property of the *account*, not of the
  connection.
- Stored secrets are **write-only in the UI**: after saving, only a fingerprint,
  expiry and a "replace token" action are shown. Normal UI never reveals a stored
  token, and a token is never a URL parameter, log line or copyable field.
- **Once a command is constructed and submitted, its target is frozen.** The broker
  connection, account and environment travel with the command until it reaches a final
  state. Switching the active account in the shell afterwards updates account-scoped
  views but can never retarget, redirect or re-environment that command; cancelling and
  re-submitting are two separate operator actions. The frozen row says so in words.
- Changing the active account updates every account-scoped view atomically — portfolio,
  positions, orders, operations, risk, ticket, protection state and strategy binding.
  A late response belonging to the previous account never overwrites the current one,
  and widgets pinned to an instrument keep their instrument: account context and
  instrument context are independent.
- Connection health is its own vocabulary: `CONNECTED`, `VALIDATING`, `RECONNECTING`,
  `DEGRADED`, `INVALID_CREDENTIAL`, `REVOKED`, `PERMISSION_LIMITED`, `ROTATE`
  (expiring), `PROVIDER_UNAVAILABLE`, `DISABLED`, `UNKNOWN`. These are never collapsed
  into one generic red `Ошибка`: a revoked token, a scope-limited token and an
  unreachable provider demand different operator actions. Ownership of these states is
  **Platform Foundation — broker connections, secret storage, account discovery and
  execution authorization (issue #17)**.

### 5.10 Live execution authorization

A broker credential that permits trading does **not** enable Vox execution. The two
facts are separate objects on screen and separate policy states.

- In `PRODUCTION` Vox execution is **off by default**. The shell shows the
  authorization state next to the environment and the runtime chip.
- Enabling is deliberately high-friction and scoped to one account and one
  environment: consequences stated in words, typed confirmation, audit metadata
  (actor, time, scope) rendered from the backend.
- **Halting is always cheaper than enabling**: one control, no typed word, no dialog.
  Halting stops new dispatches only; orders and stop orders already accepted by the
  broker keep living at the broker, and the UI says that.
- Strategy, ML and Decision Center screens may link to the authorization screen but can
  never grant it, imply it or bypass its confirmation.

### 5.11 Dispatch without an answer — reconciliation

A command that left Vox without a broker answer is an **unfinished answer, not a
refusal**.

- The state is `UNKNOWN` with reason code `UNKNOWN_AFTER_DISPATCH`, the violet unknown
  semantic, and the age of the silence stated explicitly.
- The frozen execution target stays visible, together with the facts Vox knows for
  certain: what was sent, when, at what price, to which account.
- **Re-submission is blocked until reconciliation answers.** Requesting state from the
  broker and opening diagnostics stay available; cancel and re-send remain two separate
  actions.
- Reconciliation outcomes are rendered distinctly: `RECON_CONFIRMED`,
  `RECON_NOT_FOUND` (re-submission now permitted), `RECON_PENDING` (account marked
  unreconciled). None of them is styled as a failure. Ownership of the resolution is
  **Runtime Foundation — persistence, reconciliation and readiness (issue #11)**.

### 5.12 Permissions in the interface

The frontend renders the backend permission model and never treats a hidden control as
enforcement. A denied capability is shown as a disabled control with the reason and a
way forward; an authoritative `403` arriving against stale UI permission state must
leave the screen coherent rather than half-submitted.

## 6. Accessibility

- Contrast ≥ 4.5:1 for text, ≥ 3:1 for meaningful borders on dark surfaces.
- Colour + text + shape for every state; red/green never alone (position side always
  carries a sign or a letter marker).
- Visible focus ring on every interactive element (`--vox-focus-ring`), full keyboard
  path through ticket, table and menus, `Esc` always retreats.
- Mouse wheel never changes a trading-critical value.
- Minimum 14px hit target for icon buttons at Compact; 24px for anything destructive.

## 7. Prohibitions

1. No oversized generic SaaS cards — no big-padding card holding one number. Metrics
   live in a dense bordered grid.
2. No buy/sell mode toggle.
3. No raw broker/account/order identifiers in normal UI.
4. No `UNKNOWN` rendered as an error.
5. No English in primary UI copy.
6. No Comfortable-by-default, no widget shadows, no radius > 8px, no gradients as
   surface treatment, no decorative illustration, no emoji.
7. No screenshot used as specification — the reference implementation is the artefact.
8. No client-side emulation of broker-native protection, and no protection UI that
   does not map to a broker execution object.
9. No silent rewrite of existing broker stop orders when a default changes, and no
   default presented as a hard risk limit.
10. No revealed token in normal UI, no token treated as an account or portfolio, and
    no capital-affecting command without an explicit, visible execution target.
11. No retargeting of a submitted command by changing the active account, and no
    re-submission while the outcome is `UNKNOWN_AFTER_DISPATCH`.
12. No bulk protection change without preview, affected list, consequences, typed
    confirmation and a per-position result.
13. No automated execution implied by a trading-capable broker token, and no strategy,
    model or Decision Center screen that appears to grant execution authorization.
14. No connection failure collapsed into a generic red `Ошибка`, and no client-side
    recomputation of a broker-reported trailing level.

## 8. Governance

- Adding a token: change `tokens.css` + `tokens.json` + this document in one PR.
- Adding a component: spec entry in `COMPONENT_SPEC.md`, CSS in the correct layer,
  demo block in `reference/index.html`.
- Layer order is one-directional: `tokens → primitives → components → patterns`.
  A lower layer never imports an upper one.
- Canonical hierarchy: this document and `COMPONENT_SPEC.md` are normative; the CSS
  layers are the implementation source of truth;
  `frontend/design-system/reference/index.html` is the maintainable rendered
  reference built on those layers; `vox-trader-design-system.reference.html` is only
  a stable viewing entry point for that reference and is not a design authority.
- Before review, validate at 1280 / 1440 / 1920 px, in Compact / Standard /
  Comfortable, and in every state: happy, loading, empty, stale, reconnecting,
  degraded, error, permission-denied, `UNKNOWN`, `BLOCKED`.
