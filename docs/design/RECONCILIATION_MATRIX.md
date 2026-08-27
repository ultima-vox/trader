# Vox Trader — design system reconciliation matrix

Issue #42, Phase A. Compares the **canonical visual source**
(`docs/design/source/Vox-Trader-Design-System-canonical.html`, SHA-256
`5da71028760066f8781af367dc42daa1c65a586e544315947fedf71d8a473196`, 366 334 bytes,
32 sections) against the **repository-native design system** merged from #20, and fixes the
unified result for each row before any code changes.

## Method

The canonical source carries **no CSS custom properties**: it is a rendered design document
whose values are inline, using 39 distinct hex colours. Both systems were therefore compared
by value, not by variable name:

- every hex, spacing, radius, motion and density number extracted from both;
- 58 canonical vocabulary items probed against the repository layers, reference and
  component spec.

Result: **46 of 58 present, 12 missing.** The two systems share one lineage — the palette,
type scale, spacing, radii, motion and Compact geometry are already identical, value for
value. This is a delta reconciliation, not a merge of two visual directions.

## Conflict law applied

Semantics follow the backend contracts; visual language and geometry follow the canonical
source; safety beats convenience; an unsupported capability is deferred, never simulated.
Where a row resolves to *canonical*, the repository changes. Where it resolves to
*repository*, the canonical example is a stale semantic that the executable reference must
correct without erasing the feature.

---

## 1. Tokens

| Area | Canonical source | Repository | Unified result | Reason |
| --- | --- | --- | --- | --- |
| Neutral ramp 0–1000 | `FFFFFF · F7F8FA · EEF0F3 · D9DDE3 · B8BEC8 · 929AA7 · 707986 · 515966 · 383F49 · 252A32 · 1D2229 · 161A20 · 12161B · 0D1014 · 080A0D` | identical | keep | already canonical |
| Accent 300–700 | `78B7FF · 4C9FFF · 2788F5 · 176FD1 · 1158AA` | identical | keep | already canonical |
| Accent ≠ BUY | accent is selection/focus/active only | same rule in tokens and docs | keep | already canonical |
| Financial semantics | positive `27B07D`/`159466`, negative `E45858`/`CA3F3F`, warning `D9A441`/`BD8621`, info `5794E6`, unknown `A77BE8` | identical | keep | already canonical |
| Surfaces / borders / text | canvas `0D1014` → workspace `12161B` → surface `161A20` → hover `1D2229` → elevated `252A32`; borders `252A32/303741/454D59/2788F5`; text `EDF0F4/B8BEC8/858E9B/59616C` | identical | keep | already canonical |
| BUY/SELL pressed tints | `0F7D55`, `A93333` | **absent** — `:active` reuses `--vox-*-strong` | **canonical**: add `--vox-positive-pressed`, `--vox-negative-pressed` | the only two canonical hexes with no repository token |
| Type scale | 11/12/13/14/16/18/22/28, base 13 | identical | keep | already canonical |
| Weights | 400 / 500 / 600 / **700 (rare)** | 400/500/600 | **canonical**: add `--vox-weight-bold: 700` | canonical declares four weights |
| Fonts | Inter + JetBrains Mono | identical | keep | already canonical |
| Spacing | 4/8/12/16/20/24/32/40 | identical | keep | already canonical |
| Radius | 2/4/6/8, control 4, widget 6 | identical | keep | already canonical |
| Elevation | surface and elevated carry no shadow; overlays only | identical | keep | already canonical |
| Motion | 80/120/180/480 ms | identical | keep | already canonical |
| Density | Compact 28/26/28/32 · Standard 32/30/32/36 · Comfortable 36/36/36/36 | identical | keep | already canonical |
| Breakpoints | 1600 / 1440 / 1280 / 1024 / separate below | present in `tokens.json`, absent from the reference sheet | **canonical**: surface them in the reference | the canon states them as a visible rule |

## 2. Primitives and data

| Area | Canonical source | Repository | Unified result | Reason |
| --- | --- | --- | --- | --- |
| Icons | one family (Lucide) behind an abstraction, stroke 1.5–1.75, sizes 12–20, semantic status icons | identical | keep | already canonical |
| Number rendering | tabular, right aligned, sign unambiguous, units always visible, explicit anti-patterns (`12450.24738183`, `272.5`, `10 (?)`) | `.vox-num` + `.vox-unit` exist; the formatting rules and anti-patterns are not shown | **canonical**: add the formatting block, including anti-patterns | the rule is part of the canon and prevents float leakage in UI |
| Money / Price / Quantity / PnL | named data primitives | expressed through `.vox-num` variants | keep repository classes, **document the canonical names** | one primitive family; no competing classes |
| Timestamp | mono, millisecond precision `11:42:06.312` | second precision, no primitive | **canonical**: millisecond timestamps in the tape and a documented primitive | canon fixes tape precision |
| Metric 22px | `.vox-text--metric` equivalent | present | keep | already canonical |
| **Sparkline 60×20** | positive and negative variants | **absent** | **canonical**: add `.vox-sparkline` | missing primitive |
| Order status vocabulary | Открыта · Частично 6/10 · Исполнена · Снята · Отклонена · Истекла · UNKNOWN | badges exist, vocabulary not stated | **canonical vocabulary + repository semantics**: render the words, mark that `OrderFact` today exposes only `active`/`terminal` (BD-1/BD-4) | canon owns the words, contract owns what is currently observable |
| Protection status vocabulary | Защищена · Защита в процессе · Без защиты | partially present in the portfolio table | **canonical**: state the three words, map to `ProtectionEstablishmentState` | canon owns the words, contract owns the states |
| Mode vocabulary | Advisory · Нужно подтверждение · Autonomous | present on the strategy screen | keep, and hoist into the shared vocabulary | one vocabulary, one place |

## 3. Controls

| Area | Canonical source | Repository | Unified result | Reason |
| --- | --- | --- | --- | --- |
| Buttons | 6 variants × 7 states, height 28/32, padding-x 8–12, radius 4; BUY/SELL are financial actions, no giant CTA | same variants and geometry; states demonstrated | keep, add the pressed tints from row 1 | already canonical |
| NumericInput | live stepper, field sized to data, wheel never changes a trading value, invalid shown before submit, `↑↓` step, `Shift+↑` ×10, Enter commits, Escape reverts | identical | keep | already canonical |
| Tabs / Segmented / Checkbox / Switch / Slider / Menu | one interaction model | all present, slider included | keep | already canonical |
| Instrument Picker | one picker for the whole application, feature-local picker forbidden, UID/FIGI only in diagnostics | identical | keep | already canonical |

## 4. Trading components

| Area | Canonical source | Repository | Unified result | Reason |
| --- | --- | --- | --- | --- |
| Table | 12–13px, row 26, header 28, numbers right, rows never jump while streaming | identical, plus a proven column gutter and a 2px alignment fix | **repository geometry corrections on canonical form** | documented layout defects justify local correction |
| Streaming flash | 480 ms, background and figure colour only, geometry never changes | identical | keep | already canonical |
| Instrument header | symbol · name · venue · type, last, change, **bid / ask / spread / day range / volume / lot size / session status**, position, average, buy/sell | symbol, last, change, bid, ask, spread, volume, position | **canonical**: add day range, lot size and session status | canon fixes what the header must answer |
| ChartContainer | toolbar `1m 5m 15m 1h 1D`, indicators, drawing, settings; **OHLCV readout**; marker legend; price lines | placeholder with marker legend and price-line legend, timeframe segmented control | **canonical**: add the OHLCV readout and the toolbar actions | canon fixes the chart shell; the engine stays out of scope |
| Order book | depth behind the number, spread strip, cumulative column, level count selector | identical | keep | already canonical |
| Trade tape | ms timestamps, own trade marked, large trades weighted | present without ms | **canonical**: ms timestamps | canon fixes precision |
| Order ticket | one body, two final actions, no side toggle, value/exposure/margin/risk beside the buttons | identical, plus execution target row, frozen target and protection source | **canonical form + repository safety** | safety additions do not change the visual language |
| Orders table incl. UNKNOWN | transport loss after dispatch is not a broker refusal; risk reservation held until reconciliation | identical, expressed through `UNKNOWN_AFTER_DISPATCH` | keep | already canonical, and contract-exact |
| Account summary | dense metric grid: NAV, day PnL, unrealised, free, gross and net exposure | dense metric grid, values deferred to BD-4 | **canonical form, repository deferral** | no valuation contract exists (BD-4) |

## 5. State languages

| Area | Canonical source | Repository | Unified result | Reason |
| --- | --- | --- | --- | --- |
| RiskIndicator | Разрешено · Заблокировано · Неизвестно · Уменьшено (RESIZE) with a human reason | same vocabulary, verdict deferred to BD-2 | **canonical vocabulary, repository deferral** | no risk contract exists |
| RuntimeStatus | READY · RECONCILING · DEGRADED · HALTED, clickable, opens diagnostics | eight `RuntimeState` values | **repository**: the contract has eight states | semantics follow the backend |
| EnvironmentBadge | LIVE · SANDBOX · PAPER · BACKTEST | `SANDBOX`/`PRODUCTION`; PAPER/BACKTEST disabled under BD-13 | **repository semantics, canonical form**: broker environment is `SANDBOX`/`PRODUCTION`; PAPER and BACKTEST are **trading modes**, a separate axis, deferred | the canonical text itself describes PAPER as local execution and BACKTEST as a historical run — modes, not broker environments |
| Connection health | Соединён · Переподключение · Отключён | contract values only | **repository** | connection states must come from `ReasonCode`/`StreamState` |
| Widget states | eight mandatory: loading, live, stale, reconnecting, degraded, empty, error, permission | all eight present | keep | already canonical |
| UNKNOWN | own semantic, never rendered as failure | identical | keep | already canonical |

## 6. Shell, workspace and governance

| Area | Canonical source | Repository | Unified result | Reason |
| --- | --- | --- | --- | --- |
| AppShell | top bar answers five questions; nav rail; settings is not a primary workspace; operator avatar | identical | keep | already canonical |
| Multi-broker (canonical §28–§32) | token ≠ portfolio; connection owns environment and credentials; accounts discovered per connection; execution permission is a property of the account | identical, contract-mapped | keep | already canonical and contract-exact |
| Connections screen | add, validate, discover, bind, rotate, disable | identical, with credential lifecycle deferred (BD-5) | keep | rotation lifecycle has no contract |
| Protection | executable, not decoration | identical, mapped to `ProtectionPlan`/`ProtectionCapability`/`ProtectionEstablishmentState` | keep | already canonical and contract-exact |
| Execution authorization | broker permission ≠ Vox permission | identical | keep | already canonical |
| Hard rules / DoD | no feature-local button, no raw HEX outside tokens, no arbitrary radius, no broker UID as label, no full-width input without reason, no big card for one number, no hidden units, no hidden environment, no colour-only state, no widget without empty/error/stale, no feature-specific picker, no custom chart engine | identical, plus: no simulated capability without a contract | **union** | the repository rule is an addition, not a conflict |
| Verification | not part of the canonical document | strict nine-rule verifier, 1280/1440/1920 × three densities, negative controls | **repository** | proven defect detection is kept |

---

## Decisions summary

**Repository changes to match the canon (12):** pressed tints for BUY/SELL · weight 700 ·
breakpoints shown in the reference · number formatting rules with anti-patterns · millisecond
timestamps · sparkline primitive · order status vocabulary · protection status vocabulary ·
mode vocabulary hoisted · instrument header day range, lot size and session status · chart
OHLCV readout and toolbar actions.

**Canonical examples corrected by production truth (5):** `LIVE` → `PRODUCTION` for the
broker environment · PAPER/BACKTEST moved off the broker-environment axis and deferred ·
runtime states extended from four to eight · risk and valuation figures rendered deferred
instead of stated · connection health expressed with contract values.

**Nothing is averaged.** Every row resolves to one source, and the reason is recorded.

---

## Phase F — verification and outcome

### Automated

| Check | Result |
| --- | --- |
| Strict per-widget verifier, 1280/1440/1920 × Compact/Standard/Comfortable | **0 findings in 9 of 9** |
| Geometry matrix (control/row/widget-header heights, ticket minimum, page overflow) | **0 failing combinations of 9** |
| Negative controls (gutter removed, fixed-height target row, header border removed, narrow numeric) | all four still detected — the zero is not vacuous |
| Structure | tags balanced, no class used that the layers do not define, no raw HEX outside tokens |
| Canonical coverage probe | **58 of 58 canonical vocabulary items present** (46 before this work) |
| Canonical source integrity | SHA-256 `5da71028…3196` unchanged; the restored file is git-ignored provenance |

### Human visual review

The canonical source and the unified reference were rendered side by side at 1440 and read
screen by screen. What changed as a result, beyond the token/component deltas above:

- the reference adopted the canonical **document chrome** — a fixed left rail of numbered
  sections grouped Foundation / Controls / Data / Trading / Workspaces / System, a mono
  kicker over each section (`07 · ТАБЛИЦЫ`), title and subtitle — because the canon
  presents the system that way and a reader comparing the two should not have to translate;
- the section number is no longer printed twice: the kicker carries it;
- the table gained the canonical row anatomy: selection checkbox, row-level `⋮` menu, unit
  suffixes inside numeric cells, a capability caption and a `Выбрано 1 из 4` footer.

Defects the review and the verifier found in this pass, fixed rather than waived:

1. the millisecond timestamp overflowed the tape's 76px time column → column widened to 88px;
2. the left rail narrowed the content column enough to break the workspace demos at 1280 →
   the rail folds below 1440, the width the canonical sheet is drawn at;
3. the instrument header, widget header, widget title, top bar and settings form rows all
   held fixed geometry that clipped once the column narrowed → each grows or wraps instead;
4. a hard-coded `336×71px` stale bar in the widget demo overflowed its card → removed.

### Deferred backend-owned regions (unchanged by this reconciliation)

BD-2 risk verdict, guardrails, day loss, concentration · BD-3 quotes, book, tape and chart
data · BD-4 valuation, P&L, operation amounts · BD-5 credential lifecycle and RBAC ·
BD-6 strategy and decision · BD-7 models · BD-8 research runs · BD-9 account protection
defaults · BD-10 bulk migration · BD-11 aggregate accounts · BD-12 version and jobs ·
BD-13 second provider and PAPER/BACKTEST. Sixteen regions render `.vox-deferred` with their
dependency id; none simulates data.

### Statement on the canonical source

`docs/design/source/Vox-Trader-Design-System-canonical.html` was restored with the committed
script and **not modified**. Its SHA-256 still matches the value recorded in
`docs/design/source/README.md`. It is provenance, is git-ignored as a restored artefact, and
is not imported by anything.

### Recommendation

The unified system is ready to become the sole implementation baseline. There is one
executable design system under `frontend/design-system/`, one rendered reference published
by Pages, and no competing component family: the pre-implementation `DESIGN_SYSTEM.md`
baseline is now a pointer stub and the duplicate viewing wrapper is removed. Visual language
and geometry follow the canonical source; every state word, environment and reason code on a
working screen comes from `vox-domain` or `vox-runtime`; unsupported capabilities are visibly
deferred.
