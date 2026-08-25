# Vox Trader — Component Specification

Companion to `VOX_TRADER_DESIGN_SYSTEM.md`. Anatomy, variants and states for every
component in `frontend/design-system/`. All sizes are **Compact (production default)**;
Standard/Comfortable only change the token values listed in the density table.

Layer map: `tokens/` → `primitives/primitives.css` → `components/components.css` →
`patterns/patterns.css`. Class names are the contract.

---

## Primitives

### Text / Number — `.vox-text*`, `.vox-num`, `.vox-unit`
Anatomy: single text node. Variants: `--secondary`, `--tertiary`, `--disabled`,
`--caption` (11), `--dense` (12), `--title` (16), `--headline` (18), `--metric` (22),
`--metric-xl` (28), `--mono`, `--label` (10px mono, tracked, uppercase).
Every market number uses `.vox-num` (+ `--positive` / `--negative` / `--neutral` /
`--unknown`): tabular figures, right-aligned, no wrap. Units are a separate
`.vox-unit` span so the figure keeps alignment.

### Icon — `.vox-icon`
One family (Lucide) behind the class. Sizes 12/14/16/18/20; stroke 1.75 up to 16px,
1.5 at 18–20px. Never mix stroke widths in one row. `currentColor` only.

### Surface / Divider / Stack / Grid — `.vox-surface`, `.vox-divider`, `.vox-stack`, `.vox-row`, `.vox-grid-12`
Surface variants: default, `--workspace`, `--elevated`, `--active`, `--flat`
(square corners for edge-to-edge regions). Separation by border, not by gap.

### ScrollArea — `.vox-scroll`
Thin custom scrollbar, 10px, track = canvas, thumb = `--vox-border-default`. Scroll
position is preserved across data updates.

### Tooltip / Popover — `.vox-tooltip`, `.vox-popover`
The only shadow-bearing layers. Tooltip: 11px, max 320px, explains *why* (source, age,
limit), never repeats the label. Popover hosts menus and diagnostics.

---

## Controls

### Button — `.vox-btn`
Anatomy: `[icon?] label [count?]`. Height `--vox-control-height` (28), padding 8–12px,
radius 4, 12px medium.
Variants: `--primary`, `--secondary`, `--ghost`, `--danger`, `--buy`, `--sell`.
States: default, `:hover`, `:active` (pressed), `.is-selected`, `.is-focused` /
`:focus-visible`, `:disabled` / `[aria-disabled]`, loading (`.vox-spinner` replaces the
icon, label stays, width does not jump).
Rules: `--buy` / `--sell` are financial actions and always appear as a pair; `--danger`
is outline-only (destructive actions must not be the most attractive target); a
destructive action in `LIVE` also gets `.vox-live-action`.

### IconButton — `.vox-icon-btn`
28 / 24 (`--sm`) / 22 (`--xs`). Transparent until hover. Requires `aria-label`; tooltip
mandatory in widget headers.

### Input — `.vox-input`
Anatomy: `[prefix?] field [unit?] [affordance?]`. Height 28, radius 4, 13px.
States: default, hover, `:focus-within`, `.is-invalid` (red border + tinted bg +
`.vox-field-error` sentence below), `.is-validating`, disabled, read-only.
Width follows the data; never full-width without a reason.

### NumericInput — `.vox-numeric`
Anatomy: `[−] value [+]`, 22px steppers, value right-aligned tabular, min-width 56px.
Keyboard: ↑/↓ step, `Shift`+↑/↓ ×10, `Enter` commits, `Esc` reverts.
**Mouse wheel must not change a trading-critical value.** Invalid: `.is-invalid` plus a
concrete fix ("кратно лоту 10 — ближайшее 10").

### Select — `.vox-select`
28px trigger + `.vox-popover .vox-menu` list. Selected item marked with a check, not
only by highlight. No native `<select>` styling.

### Tabs — `.vox-tabs` / `.vox-tab`
28px, 2px bottom accent when `.is-selected`, optional `.vox-tab__count` (mono 10px).
Counts are live. `.is-disabled` explains itself via tooltip.

### SegmentedControl — `.vox-segmented`
2–4 short mutually exclusive options, 28px. Used for density, chart interval, table
scope. Never for buy/sell.

### Checkbox / Switch — `.vox-check`, `.vox-switch`
Checkbox 14px, three states (off / on / indeterminate) — for filters and lists.
Switch 32×18 — only for a live behavioural setting that applies immediately
(instrument linking, sound alerts). Never for submitting a form.

### Slider — `.vox-slider`
4px track, 12px knob. Only for non-critical continuous values (depth, opacity, zoom).
Never for quantity or price.

### Menu — `.vox-menu`
Row height = `--vox-row-height`, 12px label, mono 10px shortcut, `--danger` items at
the bottom after a separator. Order: contextual → navigational → destructive.

---

## Status components

### Badge — `.vox-badge`
20px, radius 2. Variants: neutral, `--positive`, `--negative`, `--warning`, `--info`,
`--unknown`. Always dot + text; a bare dot is never a status.

### EnvironmentBadge — `.vox-env`
22px, semibold, tracked. `--live` (red-outlined), `--sandbox` (blue), `--paper`
(violet), `--backtest` (neutral). Permanently visible in the top bar; also inlined in
the order ticket header. `.vox-live-action` adds the inset red hairline to irreversible
controls in `LIVE`.

### RuntimeStatus — `.vox-runtime`
Clickable chip, opens diagnostics. `--ready` (green dot), `--reconciling` (blue pulsing
dot), `--degraded` (amber surface), `--halted` (red surface, semibold). Label is the
Latin state word; the human explanation lives in the diagnostics popover.

### RiskIndicator — `.vox-risk`
Anatomy: dot/icon + verdict sentence + `.vox-risk__reason` + `.vox-reason-code`.
Variants `--safe`, `--warning`, `--blocked`, `--unknown`, `--resize`.
`--blocked` disables submit and names the limit. `--resize` shows `было → станет`.
`--unknown` states the decision is deferred (not refused) and offers retry.

### Marker — `.vox-marker`
14px badge, 9px semibold letter, radius 2. `--b` filled green, `--s` filled red,
`--f` filled blue, `--sl` outlined amber, `--tp` outlined green, `--d` outlined violet,
`--e` outlined info. Identical in chart, tape, orders and journal.

### Skeleton / StateNote / StaleBar — `.vox-skeleton`, `.vox-state-note`, `.vox-stale-bar`
Skeletons mirror final row geometry (no layout jump). StateNote covers empty, error and
permission-denied: title + one explaining sentence + up to two actions. StaleBar names
the data age and says last known values are shown.

---

## Data components

### Table — `.vox-table`
Anatomy: sticky `__header` (28px) → `__row` (26px) → optional `__footer` (24px totals).
12px body, 11px header. Numeric columns right-aligned via `.vox-num`; text left.
Row states: hover, `.is-selected` (accent left border 2px + tinted bg), `.is-unknown`
(violet left border, violet-tinted bg), flash on value change (`.vox-flash-up` /
`.vox-flash-down`, 480 ms, background only).
Rules: column widths declared by the caller via `grid-template-columns` so header and
rows stay aligned; sorting on the header cell (`.vox-table__sort`); streaming updates
must not reflow; no zebra striping; no per-row action buttons — row context menu instead.

### Widget — `.vox-widget`
Anatomy: `__header` (32px, `cursor: grab`, title + instrument context chip +
`__tools`) → `__body` → optional `__footer` (24px) → `__resize` (12px handle).
States: `.is-active` (focused widget, brighter border), `.is-stale` / `.is-degraded`
(amber border + stale bar), `.is-error` (red border + StateNote), `.is-pinned` (header
not draggable), loading (skeleton body), empty and permission-denied (StateNote).
Context chip: `.vox-widget__context` (pinned/neutral) or
`--instrument` (linked to the workspace instrument).
Rules: no shadow, radius 6, header is the only drag handle, resize snaps to 8px, and
the widget never shows data whose instrument is not named in its header.

---

## Patterns

### AppShell — `.vox-shell`, `.vox-topbar`, `.vox-nav`
Top bar 44px, groups separated by 1px borders: brand · broker + account (human names)
· environment + runtime · portfolio P&L · MSK clock (mono, tabular). Nav rail 118px,
26px items, 2px active left border, secondary items pinned to the bottom.

### Workspace — `.vox-workspace`, `.vox-drop-target`
12-column grid, 6px gap, `minmax(48px, auto)` rows. Drag by widget header; drop target
= dashed accent outline; `.is-dragging` sets `cursor: grabbing`. 8px resize step.
Layout persisted per workspace; workspaces are switchable and duplicable.

### InstrumentHeader / Quote — `.vox-quote`, `.vox-context-link`
56px band: symbol + name (16px semibold) · last price (22px tabular, flashes) ·
change · bordered stat cells (bid/ask/spread/volume/high/low) · actions.
`.vox-context-link` states: default, `.is-linked` (blue, follows selection),
`.is-pinned` (amber, fixed instrument).

### OrderBook — `.vox-book__*`
22px header, 20px rows, three columns (price / size / cumulative). Depth is a bar
**behind** the number (`__depth`, ≤12% alpha) — never replacing it. Asks red, bids
green, 22px spread strip between them showing absolute and percentage spread. Own
orders: `.is-own` (accent tint + 2px accent left border). Click a price to prefill the
ticket — never to submit.

### TradeTape — `.vox-tape__*`
20px rows: mono time · price (bid/ask coloured) · size (`--large` emphasised) · marker
letter. Own trades highlighted. New rows append without scroll jump.

### OrderTicket — `.vox-ticket`
Width 300. Anatomy: instrument strip → type Select → quantity NumericInput (+ lot
hint) → price Input → `__preview` (сумма / комиссия / маржа после) → RiskIndicator →
`__actions` → mono shortcut hint.
`__actions`: two 34px buttons, `--buy` = `Купить`, `--sell` = `Продать`, each with its
own executable price. Both always present; **no mode toggle exists**. Blocked side:
`.is-blocked` + `__action-note` + reason. In `LIVE` both actions carry
`.vox-live-action`. Submission requires a non-blocked risk verdict; `RECONCILING` and
`DEGRADED` allow submission with the caveat displayed; `HALTED` blocks new orders and
leaves cancel available.

### PortfolioSummary — `.vox-metrics`, `.vox-limit`
4-column bordered metric grid (10px label + 16px tabular value) — the explicit
alternative to oversized SaaS cards. Limit usage: 4px track with
`--warning` / `--blocked` / `--neutral` fill plus a percentage caption.

---

## Review checklist

- [ ] Compact geometry (28/26/32) and no hardcoded pixel heights.
- [ ] Tokens only — no raw HEX outside `tokens.css`.
- [ ] Russian copy; Latin only for tickers, states, reason codes, marker letters.
- [ ] `Купить` and `Продать` both present; no mode toggle.
- [ ] Markers limited to B/S/F/SL/TP/D/E.
- [ ] Environment + runtime visible; `UNKNOWN` not styled as error.
- [ ] All numbers via `.vox-num`; streaming causes no reflow.
- [ ] No raw broker identifiers; no oversized single-number cards; no widget shadow.
- [ ] Focus ring present; keyboard path complete; wheel does not alter critical values.
