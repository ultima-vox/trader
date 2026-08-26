# Vox Trader — design compliance audit

Scope: the repository-native design system (`docs/design/*.md`,
`frontend/design-system/**`) audited against **issue #18** (Frontend Foundation) and the
**PR #20 Head-of-Development directive**.

`§n` refers to a numbered section of `frontend/design-system/reference/index.html`.
Status values:

- **Represented** — visible in the rendered reference and stated in the normative docs.
- **Deferred** — the issue itself marks it optional or a non-goal; deliberately absent.
- **Outside artefact** — belongs to application code or verification, not to a design
  reference.

Sections of the rendered reference:

| § | Screen / topic | § | Screen / topic |
| --- | --- | --- | --- |
| 1 | Tokens | 13 | Bulk protection migration |
| 2 | Density | 14 | Execution authorization |
| 3 | Primitives | 15 | Brokers & accounts |
| 4 | Controls | 16 | Markets |
| 5 | Environment / runtime / risk | 17 | Portfolio (+ operations, event journal) |
| 6 | Event markers | 18 | Strategy |
| 7 | Table | 19 | Decision Center |
| 8 | Widget | 20 | Research |
| 9 | Trading workspace (canonical screen) | 21 | ML / Models |
| 10 | Order ticket · execution target | 22 | System |
| 11 | Position protection | 23 | Prohibitions |
| 12 | Reconciliation & UNKNOWN | | |

---

## 1. Issue #18 — directive by directive

| # | Directive | Where | Status |
| --- | --- | --- | --- |
| Mandatory architecture 1–3 | backend authoritative, no wire DTO leakage, no duplicated execution logic | Design system §5 governance; every protection/risk block names the owning issue (#10/#11/#17) | Represented |
| Mandatory architecture 4 | no secret persistence client-side | §15 SecretInput, write-only, fingerprint only | Represented |
| Mandatory architecture 5 | explicit financial semantics | §9 shell, §10 target row, §11 source badge, §5 vocabularies | Represented |
| Mandatory architecture 6 | uncertainty first-class | §12, violet `UNKNOWN` semantic, `.is-unknown` rows | Represented |
| Mandatory architecture 7 | capability-first UI, no simulation | §11 unsupported mode, §15 capability badges | Represented |
| Canonical terminal character | density, language, tabular numerals, one icon family | §1, §2, §3 | Represented |
| Shell and global context | broker · account · environment · runtime · P&L · clock | §9 top bar | Represented |
| Navigation | Markets · Trade · Portfolio · Strategy · Decision · Research · ML · System | §9 nav rail; screens §16–§22 | Represented |
| Multi-broker / multi-account settings | connections, environments, discovery, binding, rotation, lifecycle | §15 | Represented |
| Credential UX | entry-only visibility, no readback, fingerprint, no secret echo | §15 rotation, three steps | Represented |
| AccountSelector & context propagation | always visible, atomic account-scoped update | §9, §15; atomicity rule in design system §5.9 | Represented |
| Submitted command never retargeted | frozen target | §10 `.is-frozen`, design system §5.9 | Represented |
| `Все счета` aggregate | read-only aggregate view | — | Deferred (issue marks it optional/future) |
| Widget context model | linked vs pinned, context named in header | §8, §9 (chart linked, tape pinned) | Represented |
| Workspace model | 12 columns, drag by header, resize step, presets, persistence | §9 (+ widget size catalogue) | Represented |
| Responsive validation 1280/1440/1920 | | Verified by rendering — see section 5 | Represented |
| Canonical Order Ticket | instrument, target, type, quantity, price, estimates, protection, risk, dual actions | §9, §10 | Represented |
| Dual actions with size/value; blocked side visible | | §9, §10 | Represented |
| Keyboard-first, no bypass of confirmations | | §9 hint, §13/§14 typed confirmations | Represented |
| Protection combinations | none / fixed / trailing / TP / fixed+TP / trailing+TP / stop-limit | §11 combinations matrix | Represented |
| Stop Loss modes, trailing native only | | §11 | Represented |
| Trailing runtime state | broker-reported state and levels | §11 `ACTIVE` / `STALE` / `RECONCILING` / `TRIGGERED` / `CANCELLED` | Represented |
| High/low-water semantics, never widens | | §11 long and short blocks | Represented |
| Take Profit independent | | §11 | Represented |
| Account/portfolio protection defaults | mode, distance, TP, applicability | §11 policy block | Represented |
| Protection precedence + source | order > strategy > account default, source shown | §11 precedence, §9/§10 source badge, §18 strategy view | Represented |
| Default ≠ risk guardrail | | §11 guardrail block, `BLOCKED` and `RESIZE` | Represented |
| Existing positions & bulk updates | preview, count, before/after, consequences, confirmation, result | §13 | Represented |
| Risk / runtime / connection vocabularies | | §5, §11, §12, §15 | Represented |
| Live execution controls | credential ≠ authorization, off by default, high-friction on, fast halt, audit | §14, §22 | Represented |
| RBAC-aware frontend | disabled-and-explained, 403 leaves screen coherent | §14, §22 | Represented |
| Chart and event markers | vocabulary, tooltips, grouping, price lines | §6, §9 | Represented |
| Marker geometry on the candle | below/above/adjacent placement rules | — | Deferred (no chart engine — explicit #18 non-goal) |
| Tables and streaming data | compact rows, tabular numerics, stable widths, all states | §7, §9, §16–§22 | Represented |
| Numeric input safety | metadata-driven, wheel never changes value | §4, §10 | Represented |
| Instrument picker | one shared picker, no raw ids | §10, §16 | Represented |
| Component inventory | 25 named components/patterns | `COMPONENT_SPEC.md`, all demonstrated in the reference | Represented |
| Design-system governance | hierarchy, hard rules | Design system §8, §23 prohibitions | Represented |
| Component Design DoD | anatomy, states, keyboard, a11y, anti-patterns | `COMPONENT_SPEC.md` | Represented |
| Screen Design DoD | components only, states, semantics, explicit target | §9, §16–§22; audit table in section 2 below | Represented |
| Data/state architecture | typed clients, atomic switch, stale-response guards, frozen target | Frozen target and atomicity stated in design system §5.9; the client itself is code | Outside artefact (implementation) |
| Testing matrix | | — | Outside artefact (implementation) |

## 2. Screen-by-screen Design DoD

Every production screen must name the account whose data it shows, show the
environment, cover at least one non-happy state, carry a reason code where something is
refused or unknown, and use tabular numerals for money.

| Screen | § | Account | Environment | Non-happy states shown | Reason codes | Capital-affecting target explicit |
| --- | --- | --- | --- | --- | --- | --- |
| Trading workspace | 9 | yes | `PRODUCTION` | `UNKNOWN` position row, near-limit risk | `RISK_OK`, `RISK_CONC_NEAR` | yes — ticket target row |
| Order ticket / target | 10 | yes | yes | mismatch, frozen, blocked side, invalid lot | `EXEC_TARGET_MISMATCH`, `RISK_NO_SHORT`, `ORD_LOT_STEP` | yes |
| Protection | 11 | via ticket | yes | `STALE`, `RECONCILING`, `CANCELLED`, unsupported mode, guardrail refusal | `BRK_PROTECT_STALE`, `UNKNOWN_AFTER_DISPATCH`, `BRK_TRAIL_ABS_UNSUPPORTED`, `RISK_TRAIL_MAX` | yes |
| Reconciliation | 12 | yes | via frozen target | `UNKNOWN`, discrepancy, pending | `UNKNOWN_AFTER_DISPATCH`, `RECON_*` | yes (frozen) |
| Bulk migration | 13 | yes | `PRODUCTION` | rejected, reconciling, untouched override | `BRK_TRAIL_SHORT_UNSUPPORTED`, `UNKNOWN_AFTER_DISPATCH` | yes |
| Execution authorization | 14 | yes | `PRODUCTION` | off, permission denied, `HALTED` | — (audit metadata instead) | yes |
| Brokers & accounts | 15 | yes | all four | 11 connection states, empty discovery, malformed token | `BRK_TOKEN_*`, `BRK_PROVIDER_UNAVAILABLE`, `BRK_HEALTH_UNKNOWN` | yes |
| Markets | 16 | yes | `PRODUCTION` | stale stream, empty watchlist, closed session | `MD_STREAM_SILENT` | n/a (no execution) |
| Portfolio | 17 | yes | `PRODUCTION` | unreconciled row, unprotected position, near day-limit | `RISK_DAY_LOSS_NEAR`, `UNKNOWN_AFTER_DISPATCH` | n/a (read-only) |
| Strategy | 18 | yes | `PRODUCTION` | advisory mode, stopped strategy, read-only account | `STR_ACCOUNT_READ_ONLY` | yes — via authorization |
| Decision Center | 19 | yes | `PRODUCTION` | risk rejection, empty candidate list | `RISK_CONC_NEAR` | yes — approval is an order |
| Research | 20 | n/a | `BACKTEST` | running, interrupted run | `RSH_HISTORY_GAP` | no — backtest only |
| ML / Models | 21 | yes (promotion scope) | `PRODUCTION` | drift, missing metrics, degraded training | `ML_METRICS_MISSING` | promotion does not grant execution |
| System | 22 | yes | `PRODUCTION` | `DEGRADED`, `RECONNECTING`, `HALTED`, limited permissions | `RUNTIME_HALTED` | yes — emergency halt |

## 3. PR #20 directive items

| Item | Subject | Status |
| --- | --- | --- |
| 1 | backend-driven, capability gated | Represented (§11, §15) |
| 2 | multi-broker/account shell, AccountSelector, independence of contexts | Represented (§9, §15) |
| 3 | `Настройки → Брокеры и счета` with the full lifecycle | Represented (§15) |
| 4 | ticket execution target and permanent dual actions | Represented (§9, §10) |
| 5 | protection as executable semantics, all shapes, trailing state | Represented (§11) |
| 6 | account defaults, precedence, source in the ticket | Represented (§11, §9) |
| 7 | default ≠ guardrail, `BLOCKED`/`RESIZE`, bulk re-application | Represented (§11, §13) |
| 8 | live execution authorization, halt, audit | Represented (§14, §22) |
| 9 | runtime / risk / connection vocabularies, no generic red error | Represented (§5, §15) |
| 10 | chart/event language, price lines as a separate primitive | Represented (§6, §9); on-candle geometry deferred with the chart engine |
| 11 | linked vs pinned widget context | Represented (§8, §9) |
| 12 | workspace model, presets, responsive targets | Represented (§9), verified by rendering |
| 13 | tables, numeric controls, streaming safety | Represented (§4, §7) |
| 14 | one instrument picker | Represented (§10, §16) |
| 15 | hierarchy and governance | Represented (design system §8, §23) |
| 16 | token-only cleanup | **Closed** — no raw HEX or `rgba()` outside `tokens.css`, including the viewing wrapper |
| 17 | canonical reference wording | **Closed** — `reference/index.html` is the rendered reference; the wrapper is a viewing entry point only |
| 18 | Component Design DoD | Represented (`COMPONENT_SPEC.md`) |
| 19 | Screen/Reference DoD before leaving draft | Represented; the two outstanding items are listed below |

## 4. Outstanding

1. **On-candle marker geometry** (`B`/`TP` below, `S`/`SL` above, `F` adjacent, `D`/`E`
   offset) cannot be demonstrated while the chart region is a placeholder. The letter
   vocabulary, grouping, tooltip contents and the marker-versus-price-line distinction
   are specified.
2. **`Все счета`** read-only aggregate — deferred by the issue until a backend aggregate
   execution contract exists.
3. **Implementation half of #18** — typed Vox API clients, atomic account-context
   switching, stale-response guards and the test matrix. The design contract for each is
   written; the code is not part of this artefact.

Items 1 and 2 are scoped out by issue #18 itself. Item 3 is application code.

## 5. Verification matrix

Rendered in Chromium at each viewport width with `data-density` switched on the app root,
measuring page overflow, elements past the viewport, clipped content, control geometry and
the ticket minimum.

| | 1280 | 1440 | 1920 |
| --- | --- | --- | --- |
| **Compact** — control 28 / row 26 / widget header 32 | pass | pass | pass |
| **Standard** — control 32 / row 30 / widget header 36 | pass | pass | pass |
| **Comfortable** — control 36 / row 36 / widget header 36 | pass | pass | pass |

Pass means: no horizontal page scroll, no element past the viewport, no clipped content
outside a declared scroll container, measured control/row/widget-header heights equal to
the density table, order ticket at or above its 300px minimum (360px at 1280 where the
ticket takes the wider column, 309px above the breakpoint), and all 23 sections present.

Defects the matrix found and that were fixed rather than waived:

- the account name in the Order Ticket execution target truncated to an ellipsis
  (133px of text in 51px) — it now wraps, because the account is never dropped;
- the ticket fell to 269px at 1280, under its own declared 300px minimum — the ticket
  column now widens below 1366px and the chart yields the column;
- the positions table was clipped by its widget edge — tables now scroll inside the widget;
- the account name column could collapse to zero width in the discovered-accounts list —
  it now has a 112px floor;
- policy rows, protection rows, reconciliation heads and reconciliation actions clipped
  their content in a narrow widget — they wrap;
- long form labels in Research overflowed the ticket-sized label column — that screen now
  uses the settings form row, which sizes its own label.

On the design side, no directive of issue #18 or of the PR #20 review is unrepresented.
