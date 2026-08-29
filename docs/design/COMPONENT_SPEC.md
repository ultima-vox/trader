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

### Timestamp — `.vox-timestamp`
Mono, tabular, millisecond precision (`11:42:06.312`) with `__ms` in the disabled tone.
Two prints inside one second are ordinary in a tape, so the canon works to the
millisecond and every column that holds a timestamp is sized for it.

### Sparkline — `.vox-sparkline`
60×20 inline SVG, `--positive` / `--negative`, 1.5px stroke in `currentColor`. It carries a
direction only: it never replaces a number, never carries a precise value and is never the
only carrier of state.

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
Pressed BUY and SELL use their own tints (`--vox-positive-pressed`, `--vox-negative-pressed`),
which are the only two canonical colours that are not part of the semantic ramps.
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
controls in `PRODUCTION`.

### RuntimeStatus — `.vox-runtime`
Clickable chip, opens diagnostics. Renders any of the eight `RuntimeState` values;
`--ready` (green dot), `--reconciling` (blue pulsing dot), `--degraded` (amber surface),
`--halted` (red surface, semibold), and the neutral base for `STARTING`, `CONNECTING`,
`STOPPING`, `STOPPED`. Label is the Latin state word; the human explanation comes from
`RuntimeHealth.reason_code` and lives in the diagnostics popover. New exposure is allowed
in `READY` only.

### RiskIndicator — `.vox-risk`
Anatomy: dot/icon + verdict sentence + `.vox-risk__reason` + `.vox-reason-code`.
Variants `--safe`, `--warning`, `--blocked`, `--unknown`, `--resize`.
`--blocked` disables submit and names the limit. `--resize` shows `было → станет`.
`--unknown` states the decision is deferred (not refused) and offers retry.

### Marker — `.vox-marker`
14px badge, 9px semibold letter, radius 2. `--b` filled green, `--s` filled red,
`--f` filled blue, `--sl` outlined amber, `--tp` outlined green, `--d` outlined violet,
`--e` outlined info. Identical in chart, tape, orders and journal.
Group: several events on one candle collapse into one badge with a count (`B×3`).
Legend: the marker row is rendered next to the chart, never as free-floating shapes.
Tooltip (`.vox-tooltip`): type · timestamp · price · quantity · value · source
(`ручная заявка` / `стратегия` / `брокер`); raw provider ids stay in diagnostics.
Persistent levels — average price, working order, `SL`, `TP`, last — are **price
lines**, a different primitive rendered in its own legend row. A line is never drawn
as an event marker and a marker is never used for a standing level.

### Skeleton / StateNote / StaleBar — `.vox-skeleton`, `.vox-state-note`, `.vox-stale-bar`
Skeletons mirror final row geometry (no layout jump). StateNote covers empty, error and
permission-denied: title + one explaining sentence + up to two actions. StaleBar names
the data age and says last known values are shown.

---

## Data components

### Table — `.vox-table`
A table wider than its widget scrolls horizontally inside it (`overflow-x: auto`,
children at `min-width: max-content`) — columns are never silently clipped by the widget
edge, and header and body stay aligned while scrolling.
Anatomy: sticky `__header` (28px) → `__row` (26px) → optional `__footer` (24px totals).
12px body, 11px header. Numeric columns right-aligned via `.vox-num`; text left.
Row states: hover, `.is-selected` (accent left border 2px + tinted bg), `.is-unknown`
(violet left border, violet-tinted bg), flash on value change (`.vox-flash-up` /
`.vox-flash-down`, 480 ms, background only).
Row anatomy from the canon: optional `.vox-table__select` checkbox cell, the data cells,
and a trailing `.vox-table__menu` (⋮) that holds the row's actions instead of scattering
buttons across the grid. A `.vox-table__caption` under the table states the component's
capabilities in one line. Units inside a numeric cell use `.vox-unit` so the figure keeps
its alignment.
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
Top bar 44px, groups separated by 1px borders: brand · broker + AccountSelector
(`.vox-account`, human names, always visible — the execution target of the workspace)
· environment + runtime · portfolio P&L · MSK clock (mono, tabular). Nav rail 118px,
26px items, 2px active left border, secondary items pinned to the bottom.

### Workspace — `.vox-workspace`, `.vox-workspace__col-*`, `.vox-drop-target`
12-column grid, 6px gap, `minmax(48px, auto)` rows. Placement is
`--vox-grid-col-start` / `--vox-grid-row-start` / `--vox-grid-col-span` /
`--vox-grid-row-span` (CSS Grid is 1-indexed; stored `col`/`row` are 0-based).
Span classes `.vox-workspace__col-{2,3,4,5,6,7,8,12}` set the span token — never an
inline `grid-column`. A persisted layout may set the same custom properties on the
widget so `col,row,colSpan,rowSpan` restore after reload. Below 1366px the class
fallback widens ticket (3 → 4) and yields chart (7 → 6); element-level persisted
span wins over that fallback. The ticket carries `min-width: 300px`, so no layout
can shrink it below its declared minimum. Drag by widget header; drop target
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
Width 300. Anatomy: `__target` execution-target row (broker · account · environment,
first row, never collapsed, `.is-live` inset marker, `.is-mismatch` when it differs
from the workspace selection) → instrument strip → type Select → quantity
NumericInput (+ lot hint) → price Input → `__preview` (сумма / комиссия / маржа
после) → `.vox-protect` (independent Stop Loss / Take Profit) → RiskIndicator →
`__actions` → mono shortcut hint.
`__target` states: default · `.is-live` (inset PRODUCTION hairline; the class name is a
token identifier, not an environment value) · `.is-mismatch` (amber,
names the workspace account it disagrees with, offers to move the ticket or keep it) ·
`.is-frozen` (elevated surface, non-interactive, `__target-lock` = `ЗАФИКСИРОВАНО`). A
frozen target belongs to a constructed or submitted command: switching the active
account in the shell never retargets it, and the row states that in words.
When width allows, each action carries quantity and value (`Купить 10 лотов · 31 840 ₽`).
The ticket also renders the effective protection and its source
(`Плавающий стоп 1,00 % · Источник: заявка`) via `.vox-inherited`, never the plan alone.
`__actions`: two 34px buttons, `--buy` = `Купить`, `--sell` = `Продать`, each with its
own executable price. Both always present; **no mode toggle exists**. Blocked side:
`.is-blocked` + `__action-note` + reason. In `PRODUCTION` both actions carry
`.vox-live-action`. Submission requires a non-blocked risk verdict; `RECONCILING` and
`DEGRADED` allow submission with the caveat displayed; `HALTED` blocks new orders and
leaves cancel available.

### Protection — `.vox-protect`, `.vox-trailing`
Anatomy per block: `__head` (mark `SL`/`TP` + label + inherited chip + Switch) →
`__body` (mode SegmentedControl → value row → `__result` naming the broker order).
Blocks are **independent**: `.is-off` collapses the body and keeps the head readable.
Stop Loss modes: `Фиксированный`, `Трейлинг` (broker-native). Trailing offset:
`%` or provider-supported absolute; `.vox-trailing__state` shows current and
reference level, `.is-unknown` for a level the broker does not report.
`.vox-trailing__unsupported` states an unsupported provider mode with a reason code —
never a silent client-side fallback. Every block names the resulting broker order
(`STOP_LOSS`, `TAKE_PROFIT`, `TRAILING_STOP`) and the level as absolute price + distance.
States: off, on, inherited, overridden, validating, rejected, `UNKNOWN`, unsupported.

### ProtectionReadback — `.vox-trailing__state`
Broker-authoritative runtime state of a live protection order, rendered as label/value
pairs: state badge, current level, reference level (high-water for long, low-water for
short), activation condition, and the time the broker last answered.
State badges: `ACTIVE` (`--positive`) · `STALE` (`--warning`, plus `.vox-stale-bar`
naming the age and `BRK_PROTECT_STALE`) · `RECONCILING` (`--unknown`, plus
`.vox-recon` with `UNKNOWN_AFTER_DISPATCH`, re-dispatch disabled, position counted as
unprotected) · `TRIGGERED` (`--info`, trigger price / fill price / slippage as separate
values and `SL`+`F` journal events) · `CANCELLED` (neutral, `.is-off` head, reason and
actor named, followed by a `--warning` verdict that the position is now unprotected).
Any field the provider does not report renders `.is-unknown`, never `0` and never an
error. The terminal displays this state; it never recomputes or smooths it.

### RiskGuardrail — `.vox-migrate__policy` (default vs limit)
Two bordered cells side by side: `ЗНАЧЕНИЕ ПО УМОЛЧАНИЮ` (account setting, applies to
new orders) and `ОГРАНИЧЕНИЕ РИСКА` (risk policy, own screen, own reason codes). A
request beyond the limit is rendered as the backend returned it — `.vox-risk--blocked`
(`RISK_TRAIL_MAX`) or `.vox-risk--resize` showing `12,0 % → 8,0 %` (`RISK_TRAIL_RESIZE`).
Silent clamping in the browser is prohibited.

### ReconciliationNotice — `.vox-recon`
Anatomy: `__head` (dot + title + reason code) → `__body` (one sentence saying the
command may have been accepted) → `__facts` (sent at / silence age / command /
price) → `__actions`. Used when a dispatch has no broker answer
(`UNKNOWN_AFTER_DISPATCH`). Violet unknown semantic only. Re-submission is disabled
until reconciliation answers; requesting state from the broker and diagnostics stay
enabled. Outcomes render as RiskIndicator variants: `RECON_CONFIRMED` (safe),
`RECON_NOT_FOUND` (warning, re-submission unlocked), `RECON_PENDING` (unknown).

### BulkProtectionMigration — `.vox-migrate`
A separate capital-affecting action, never a side effect of editing a default.
Anatomy: `__policy` (`было` → `станет`, the target cell accent-bordered) → `__count`
(affected positions plus a breakdown: replace / create / manually overridden and
untouched) → preview Table with per-position `было у брокера` / `станет` in broker
order terms → `.vox-exec-consequences` (including the unprotected window between
cancel and place) → `.vox-exec-confirm` (typed word) → result Table with per-position
`ПРИМЕНЕНО` / `ОТКЛОНЕНО` / `СВЕРКА`. No aggregate "done".

### ExecutionAuthorizationControl — `.vox-exec-auth`
Two-column pattern. Left: `.vox-exec-fact` cards stating separate facts — broker token
capability, Vox execution state (`--vox` card, `.is-off` / `.is-on`) and the scope
(account · environment). Right: consequences, typed confirmation naming the account,
`.vox-exec-halt` (full-width, one press, no typed word), and backend-supplied audit
metadata. `PRODUCTION` defaults to off. Strategy screens may link here and never grant
authorization themselves. Permission-denied renders as a StateNote with disabled
controls — a hidden button is not enforcement.

### PrecedenceList — `.vox-precedence`
Three fixed rows in order: order/position override → strategy policy →
portfolio/account default. `.is-effective` marks the winning value (accent left
border); `.is-overridden` strikes the losing value but keeps it readable. Never
hide a losing value, never reorder the list.

### ProtectionPolicy — `.vox-policy`
Portfolio/account default protection, including the global trailing default. Anatomy:
label · mode SegmentedControl · value Input per row, plus `__migration` — the amber
notice that a changed default applies to new orders only and that existing broker
stop orders are migrated only via an explicit listed action. A default is not a hard
risk limit; guardrails are a separate policy with their own screen.

### AccountSelector — `.vox-account`, `.vox-account-row`
The row grid is `minmax(112px, 1fr) auto auto auto`: status columns size to their
content and the row may scroll, but the human account name never collapses. In the
Order Ticket target row the account name wraps instead of truncating — ellipsis is
allowed on decoration, never on the execution target.
Always visible in the shell. Anatomy: broker · separator · human account label ·
environment badge / connection health · disclosure. Modifiers `.is-live` (inset PRODUCTION
marker), `.is-degraded`, `.is-unknown`. The popover lists `.vox-account-row`
(name + masked identifier meta · environment · health · value). A raw identifier
appears only as masked meta; full identifiers live in diagnostics.

### ConnectionHealth — `.vox-conn`
Mono chip covering the whole vocabulary: `--ok` (`CONNECTED`), `--validating`,
`--reconnecting`, `--degraded`, `--invalid` (`INVALID_CREDENTIAL`), `--revoked`,
`--scope` (`PERMISSION_LIMITED`), `--expiring` (`ROTATE`), `--provider`
(`PROVIDER_UNAVAILABLE`), `--disabled`, `--unknown`. Each state carries its own human
sentence, action and reason code; they are never collapsed into a generic red
`Ошибка`. `VALIDATING` and `RECONNECTING` are work in progress and never borrow the
negative semantic; `UNKNOWN` uses the violet unknown semantic.
Health describes a *connection*, never a portfolio.

### SecretInput — `.vox-secret`
Write-only token entry. Anatomy: masked `__input` + `__fingerprint` + `__note`.
After saving, the UI shows fingerprint, expiry and "replace token" only — there is no
reveal control in normal UI. States: empty, typing, `.is-validating` (spinner +
"checking connection and execution scope"), saved, `.is-invalid` (reason code,
e.g. `BRK_TOKEN_NO_TRADE_SCOPE`), expiring, rotated.

### InheritedValue — `.vox-inherited`
18px chip marking a value that belongs to a higher scope (dashed border, tertiary
text). `--override` (solid accent) marks a locally overridden value. Used by
protection blocks, precedence rows and policy screens.

### BrokersSettings — `.vox-brokers`
Настройки → Брокеры и счета. Two panes: connection list (label · environment ·
health · account count · "add connection") and detail: connection (broker, environment,
human label) → secret (SecretInput, validate, rotate, health) → discovered accounts
(read from the connection, never typed; execution permission per account). A connection
is not a portfolio: accounts are listed separately from the connection that found them.
States per connection: new, validating, ok, degraded, invalid token, expiring,
permission-denied, discovery empty, `UNKNOWN`.

### InstrumentPicker — `.vox-input` + `.vox-popover .vox-menu`
One picker for the whole product: search by ticker or name, rows showing ticker · name
· venue/type, keyboard navigation, recents and favourites. Normal UI never shows
UID/FIGI or other provider identifiers — they belong to diagnostics. Selection returns
a stable internal instrument id. Feature screens may not build their own picker.

### Deferred — `.vox-deferred`, `.vox-dep`
A region whose data or action has no backend contract yet. Anatomy: `__head` (title +
`.vox-dep` dependency id) → `__body` (what is missing, in words) → optional
`__actions` holding the real controls in a disabled state. Dashed border, violet unknown
tint, never the negative semantic — deferred is not an error. `.vox-dep` is a mono chip
carrying the tracked id (`BD-3`) from `docs/design/BACKEND_CONTRACTS.md`. Simulating the
missing data is prohibited; so is quietly removing the region, because the design decision
must stay reviewable.

### PortfolioSummary — `.vox-metrics`, `.vox-limit`
4-column bordered metric grid (10px label + 16px tabular value) — the explicit
alternative to oversized SaaS cards. Limit usage: 4px track with
`--warning` / `--blocked` / `--neutral` fill plus a percentage caption.

---

## Review checklist

- [ ] Compact geometry (28/26/32) and no hardcoded pixel heights.
- [ ] Tokens only — no raw HEX **and no raw `rgba()`** outside `tokens.css`.
- [ ] Russian copy; Latin only for tickers, states, reason codes, marker letters.
- [ ] `Купить` and `Продать` both present; no mode toggle.
- [ ] Markers limited to B/S/F/SL/TP/D/E.
- [ ] Environment + runtime visible; `UNKNOWN` not styled as error.
- [ ] All numbers via `.vox-num`; streaming causes no reflow.
- [ ] No raw broker identifiers; no oversized single-number cards; no widget shadow.
- [ ] Focus ring present; keyboard path complete; wheel does not alter critical values.
- [ ] Order Ticket states its execution target (broker · account · environment) as its
      first row; a mismatch with the workspace selection is shown, not swallowed.
- [ ] Stop Loss and Take Profit are independently switchable; trailing mode is named as
      broker-native and every level maps to a broker order (issue #10).
- [ ] Unsupported provider mode is stated with a reason code — no client-side emulation.
- [ ] Precedence visible: order/position override > strategy policy > account default;
      overridden values readable, not hidden.
- [ ] Changing a default does not rewrite existing broker stop orders; guardrails kept
      separate from defaults.
- [ ] AccountSelector visible in the shell; token never revealed, never equated with an
      account or portfolio (issue #17).
- [ ] A submitted command keeps its frozen target: switching the active account never
      retargets it, and the frozen row says so.
- [ ] `UNKNOWN_AFTER_DISPATCH` renders as an unfinished answer with the silence age,
      the known facts and re-submission disabled until reconciliation answers.
- [ ] Trailing readback is broker-reported: state badge, current and reference level,
      activation and answer time; unreported fields are `UNKNOWN`, not `0`.
- [ ] Bulk protection migration has preview, affected count, per-position
      `было → станет`, consequences, typed confirmation and per-position results.
- [ ] Broker token capability and Vox execution authorization are separate facts;
      `PRODUCTION` execution is off by default and halting is a single control.
- [ ] Connection states are rendered individually, never as one generic red error.
- [ ] Validated at 1280 / 1440 / 1920, in Compact / Standard / Comfortable, in happy,
      loading, empty, stale, reconnecting, degraded, error, permission-denied,
      `UNKNOWN` and `BLOCKED` states.
