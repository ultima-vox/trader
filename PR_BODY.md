# feat(frontend): add Vox Trader design system foundation

Adds the Vox Trader design system to the repository as reviewable, versioned source: normative documentation, design tokens, primitives, reusable components/patterns and an offline renderable reference sheet.

## Scope

This PR is presentation/design-system foundation only. It does not modify broker adapters, execution, risk runtime, persistence or strategy code.

## Repository structure

```text
docs/
  design/
    VOX_TRADER_DESIGN_SYSTEM.md
    COMPONENT_SPEC.md
frontend/
  design-system/
    README.md
    tokens/
      tokens.css
      tokens.json
    primitives/
      primitives.css
    components/
      components.css
    patterns/
      patterns.css
    reference/
      index.html
      vox-trader-design-system.reference.html
      assets/
        README.md
        vox-mark.svg
PR_BODY.md
```

## Reference implementation

`frontend/design-system/reference/index.html` is the maintainable layered visual reference and renders locally with relative repository assets only.

`frontend/design-system/reference/vox-trader-design-system.reference.html` is the stable canonical entry point and loads the layered reference locally. No Claude account, CDN, package manager or build step is required.

The large Claude-generated monolithic `.dc.html`/runtime provenance files from the supplied export were intentionally not made application source-of-truth; the extracted repository-native layers are the maintainable artifact.

## Design rules included

- dense professional trading terminal;
- Compact production density;
- Russian primary UI language;
- permanent dual `Купить` / `Продать` actions with no buy/sell mode toggle;
- canonical chart/event markers `B / S / F / SL / TP / D / E`;
- explicit `LIVE / SANDBOX / PAPER / BACKTEST` environment state;
- explicit `READY / RECONCILING / DEGRADED / HALTED` runtime state;
- explicit `SAFE / WARNING / BLOCKED / UNKNOWN / RESIZE` risk semantics;
- `UNKNOWN` is distinct from failure;
- draggable/resizable widgets and linked/pinned instrument contexts;
- tabular numeric treatment for streaming values;
- no oversized generic SaaS cards;
- no raw broker identifiers in normal UI.

## Local viewing

```bash
# repository root
xdg-open frontend/design-system/reference/index.html
# or on Windows
start frontend\design-system\reference\index.html

# optional static server
python3 -m http.server 8080
```

## Canonical hierarchy

1. `docs/design/*.md` — normative interaction/design rules.
2. `frontend/design-system/tokens|primitives|components|patterns` — implementation source of truth.
3. `frontend/design-system/reference/index.html` — maintainable rendered reference.
4. `frontend/design-system/reference/vox-trader-design-system.reference.html` — stable canonical viewing entry point.

## Out of scope

- no React/Vue/Svelte bindings yet;
- no runtime state management;
- no chart engine implementation;
- no broker/execution logic;
- no CI visual-regression wiring;
- no automatic merge.

Review requested before merge.
