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
# → http://localhost:8080/frontend/design-system/reference/vox-trader-design-system.reference.html
```

Both files open directly from `file://`. All paths are relative; nothing is fetched
from a CDN, and no Claude Design runtime or account is needed.

## What is canonical

| File | Role |
| --- | --- |
| `reference/vox-trader-design-system.reference.html` | Stable canonical viewing entry point. It loads the layered reference locally with no external runtime and should remain a stable path for tooling/bookmarks. |
| `reference/index.html` | Layered visual reference, hand-maintained, built on the CSS layers below. Extend this when adding components. |
| `tokens/`, `primitives/`, `components/`, `patterns/` | **Source of truth for implementation.** Hand-maintained CSS layers. |
| `../../docs/design/*.md` | **Source of truth for rules.** Documentation beats any rendering. |

Conflict resolution: docs > CSS layers > `index.html`. The stable reference entry point
is only a launcher for the layered reference; it is not an independent design authority.

## Layers

```
tokens/       tokens.css, tokens.json   colour, type, space, radius, motion, density
primitives/   primitives.css            Text, Number, Icon, Divider, Surface, Stack, Scroll, Tooltip, Popover
components/   components.css            Button, Input, NumericInput, Select, Tabs, Segmented, Checkbox,
                                        Switch, Slider, Menu, Badge, Env, Runtime, Risk, Marker, Table, Widget
patterns/     patterns.css              AppShell, Workspace, InstrumentHeader, OrderBook, TradeTape,
                                        OrderTicket, PortfolioSummary
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
- Environment (`LIVE` / `SANDBOX` / `PAPER` / `BACKTEST`), runtime
  (`READY` / `RECONCILING` / `DEGRADED` / `HALTED`) and risk
  (`SAFE` / `WARNING` / `BLOCKED` / `UNKNOWN` / `RESIZE`) are always explicit.
  `UNKNOWN` is never rendered as an error.
- Widgets are draggable by the header, resizable in 8px steps, and carry a linked or
  pinned instrument context chip.
- No oversized generic SaaS cards. No raw broker identifiers in normal UI.
- Tokens only: a raw HEX outside `tokens/tokens.css` is a review blocker.

## Adding a component

1. Write the spec entry in `../../docs/design/COMPONENT_SPEC.md` (anatomy, variants, states).
2. Add CSS to the correct layer, tokens only.
3. Add a demo block to `reference/index.html` covering every state.
4. Open `reference/index.html` at Compact, Standard and Comfortable before review.

## Out of scope here

No JS framework bindings, no React/Vue component library, no runtime or trading logic.
This directory is presentation only.
