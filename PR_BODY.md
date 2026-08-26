# feat(frontend): add Vox Trader design system foundation

> Status: **draft** until Head-of-Development review items are signed off. This revision
> implements all six blocking items from the PR #20 review.

Refs design system foundation

Adds the Vox Trader design system to the repository as reviewable, versioned source —
documentation plus a renderable reference implementation. Presentation only.

## What was exported from Claude Design

The Vox Trader design system reference sheet (dark terminal, `ru-RU`, 1440px, Compact
density) that until now existed only inside Claude Design. It is preserved in two forms:

- **Canonical visual reference** — `frontend/design-system/reference/vox-trader-design-system.reference.html`,
  a single self-contained file (styles, markup and inline SVG glyphs bundled). Opens
  from `file://` with no network, no Claude account and no runtime.
- **Provenance** — `frontend/design-system/reference/source/vox-trader-design-system.dc.html`
  (+ its `support.js` runtime), kept so the origin of the canonical export is traceable.
  Not a build input.

Everything reusable was then extracted out of that monolith into hand-maintainable
layers, so the project no longer depends on one generated HTML file:

- design tokens → `tokens/tokens.css` + machine-readable `tokens/tokens.json`
- primitives, controls/data components and trading patterns → three CSS layers
- rules and per-component anatomy/variants/states → two Markdown specs
- a hand-maintained layered reference sheet → `reference/index.html`

## Repository structure

```text
docs/
  design/
    VOX_TRADER_DESIGN_SYSTEM.md      normative rules: language, tokens, density,
                                     layout, trading semantics, a11y, prohibitions
    COMPONENT_SPEC.md                anatomy, variants and states per component
frontend/
  design-system/
    README.md                        local viewing, layer map, canonical file, how to extend
    reference/
      index.html                     layered reference sheet (hand-maintained)
      vox-trader-design-system.reference.html   canonical visual reference (generated)
      assets/                        vox-mark.svg + asset policy README
      source/                        Claude Design source kept for provenance
    tokens/       tokens.css, tokens.json
    primitives/   primitives.css
    components/   components.css
    patterns/     patterns.css
PR_BODY.md
```

## How to render/view it locally

```bash
open frontend/design-system/reference/index.html            # macOS (xdg-open / start elsewhere)
open frontend/design-system/reference/vox-trader-design-system.reference.html

# or serve statically
python3 -m http.server 8080
# → http://localhost:8080/frontend/design-system/reference/index.html
```

No build step, no dependencies, no CDN. Relative paths only; verified rendering from
`file://` and over a static server. Fonts degrade to system stacks (`system-ui`,
`ui-monospace`) — no webfont download.

## What is canonical

1. `docs/design/*.md` — normative rules. Documentation wins over any rendering.
2. `tokens/`, `primitives/`, `components/`, `patterns/` — source of truth for
   implementation. `tokens.css` is the only place a raw HEX may appear.
3. `reference/vox-trader-design-system.reference.html` — **canonical visual reference**:
   the intended visual result, frozen. Generated output; read it, don't hand-edit it.
4. `reference/index.html` — the sheet to extend when adding components.

Conflict order: docs > CSS layers > `index.html` > canonical export.

## Documentation vs generated/reference output

| Hand-maintained (edit these) | Generated / frozen (don't hand-edit) |
| --- | --- |
| `docs/design/VOX_TRADER_DESIGN_SYSTEM.md` | `reference/vox-trader-design-system.reference.html` |
| `docs/design/COMPONENT_SPEC.md` | `reference/source/vox-trader-design-system.dc.html` |
| `frontend/design-system/README.md` | `reference/source/support.js` |
| `tokens/`, `primitives/`, `components/`, `patterns/` | |
| `reference/index.html`, `reference/assets/` | |

## Design decisions carried over unchanged

- Dense professional trading terminal; **Compact is the production default**
  (control 28 / row 26 / table header 28 / widget header 32). Standard and Comfortable
  are accessibility preferences only, expressed purely as token overrides.
- **Russian primary UI language.** Latin survives only for tickers (`SBER`), technical
  states (`LIVE`, `READY`, `HALTED`), reason codes (`RISK_DAY_LOSS`) and marker letters.
  Numbers use space thousands separators and comma decimals.
- **Permanent dual order actions** `Купить` / `Продать`, one shared ticket body, each
  action showing its own executable price. **No buy/sell mode toggle exists anywhere.**
  A forbidden side stays in place, recessed, with its reason and reason code.
- **Event markers** are exactly `B / S / F / SL / TP / D / E`, identical in chart, tape,
  orders table and journal. Pictograms may not replace the letters.
- **Explicit states.** Environment `LIVE / SANDBOX / PAPER / BACKTEST` always visible as
  a labelled badge (never a bare dot); in `LIVE` irreversible controls take an inset red
  hairline while the UI as a whole stays neutral. Runtime `READY / RECONCILING /
  DEGRADED / HALTED` as a clickable diagnostics chip. Risk `SAFE / WARNING / BLOCKED /
  UNKNOWN / RESIZE` always icon + sentence + reason code. **`UNKNOWN` has its own violet
  semantic and is never rendered as failure.** Stale data declares its age.
- **Widget model**: 32px header is the only drag handle, resize snaps to the 8px
  workspace grid on a 12-column layout, layout persisted per workspace, and each widget
  shows a linked or pinned instrument-context chip.
- **No oversized generic SaaS cards** — portfolio numbers live in a dense bordered
  metric grid; widgets have no shadow and radius never exceeds 8px.
- **No raw broker identifiers in normal UI** — human account and instrument names only;
  ids stay in diagnostics and explicit copy actions.

## Implementation notes

- Layer order is one-directional (`tokens → primitives → components → patterns`); a
  lower layer never references a higher one.
- Every market number goes through `.vox-num` (tabular figures, right-aligned, no wrap)
  so streaming updates never reflow a row; price change is a 480 ms background flash.
- Shadows exist only on overlay layers (menu, tooltip, popover, modal).
- `prefers-reduced-motion` neutralises all animation; focus rings are token-driven.
- `tokens.json` mirrors `tokens.css` for future codegen (TS types, Figma sync).

## Pre-commit checks

- Reference renders locally from `file://` and from a static server; no console errors.
- No temp/generated junk beyond the two declared frozen files; no build artefacts,
  no `node_modules`, no editor files.
- No secrets, no credentials, no API endpoints, no account or broker identifiers.
- All paths relative; no external requests.
- README documents local viewing and names the canonical visual reference.

## Out of scope (intentionally)

- No React/Vue/Svelte component library, no framework bindings, no state management.
- No product feature work, no new screens beyond the reference sheet.
- No chart engine — the chart region is an explicit placeholder.
- No webfont bundling, no icon package vendoring, no light-theme rollout (light tokens
  are declared for parity only).
- No CI, lint or visual-regression wiring.

## Backend / runtime confirmation

No trading, runtime, risk-engine, broker-integration, API or configuration code was
touched. This branch adds only files under `docs/design/` and
`frontend/design-system/` (plus `PR_BODY.md`). Nothing is imported by the application
at runtime.

Do not merge automatically — review requested.
