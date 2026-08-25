# Vox Trader — Design System

Version 1.0 · status: foundation · owner: frontend
Scope: the Vox Trader trading terminal UI. This document is **normative**. Where an
implementation disagrees with it, the implementation is wrong.

Related files

- `COMPONENT_SPEC.md` — anatomy, variants and states per component.
- `../../frontend/design-system/reference/vox-trader-design-system.reference.html` — **canonical visual reference** (self-contained export from Claude Design).
- `../../frontend/design-system/reference/index.html` — layered reference sheet built on the extracted CSS layers.
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

## 8. Governance

- Adding a token: change `tokens.css` + `tokens.json` + this document in one PR.
- Adding a component: spec entry in `COMPONENT_SPEC.md`, CSS in the correct layer,
  demo block in `reference/index.html`.
- Layer order is one-directional: `tokens → primitives → components → patterns`.
  A lower layer never imports an upper one.
