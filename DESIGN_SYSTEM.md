# Trader 2.0 — Design system baseline

## Objective

The interface is an operator trading terminal, not a generic admin dashboard. The design system must reduce decision latency, preserve information density and make state/risk visually unambiguous.

The design system is defined before feature screens so new modules reuse stable patterns instead of inventing new layouts.

## UX principles

1. **Information first.** Trading-critical data takes precedence over decoration.
2. **State is explicit.** Connected/disconnected, live/sandbox, reconciled/degraded, order state and risk state are always visible.
3. **Danger requires friction.** Destructive/live-trading actions are visually distinct and may require confirmation according to policy.
4. **Dense but readable.** Desktop terminal optimizes for many concurrent data regions without oversized form controls or excessive whitespace.
5. **Consistent instrument selection.** Ticker/name dropdown patterns are reused everywhere; raw broker IDs are never primary UI labels.
6. **One interaction model.** Quantity, price, stop, account and strategy controls behave identically across manual order, position management and strategy screens.
7. **Progressive disclosure.** Basic actions remain compact; advanced settings expand on demand.
8. **No hidden financial semantics.** Lots vs units, points vs currency and margin/exposure are labelled explicitly.

## Application shell

Desktop-first shell:

```text
+------------------------------------------------------------------+
| Top bar: environment | broker | account | health | PnL | user    |
+----------+-------------------------------------------------------+
| Nav      | Workspace                                             |
|          |                                                       |
| Markets  | draggable/resizable widget grid                       |
| Trade    |                                                       |
| Portfolio|                                                       |
| Strategy |                                                       |
| Research |                                                       |
| ML       |                                                       |
| System   |                                                       |
+----------+-------------------------------------------------------+
```

The workspace is widget-based. Widgets can be moved/resized and layouts can be saved per user/workspace.

## Primary workspaces

### Trading

Core widgets:

- instrument header/quote;
- chart;
- order book;
- trades/tape;
- compact order ticket;
- active orders;
- positions;
- portfolio/risk summary;
- strategy/AI evidence panel.

### Portfolio

Positions, exposure, realized/unrealized PnL, margin, risk budgets, concentration and protection state.

### Decision Center

Trade candidates and intents with evidence, confidence/source, risk outcome and approval state. Empty states must explain why no candidates exist; never present a dead blank panel.

### Research

Historical explorer, backtests, experiments, comparisons and performance attribution.

### ML / Models

Separate top-level workspace, not buried inside Settings. Dataset creation, training, model registry, validation and promotion are operational workflows.

### Settings

Sections:

- Broker & accounts;
- Trading / risk;
- System / updates;
- Users / access;
- Appearance / workspace preferences.

## Order ticket

Order ticket should use compact brokerage-terminal ergonomics:

```text
[ BUY ] [ SELL ]
Instrument: [ SBER v ]
Order:      [ Market v ]
Price:      [ -  272.50  + ]
Quantity:   [ -    10    + ] lots
Value:        2,725 RUB

Protection [collapsed]
Risk preview: exposure / margin / limit

[ Buy 10 lots ]
```

Rules:

- fields are sized to content, not full-width by default;
- +/- steppers and keyboard arrows supported;
- mouse wheel must not accidentally alter critical numeric input;
- quantity labels state `lots`, `contracts` or `units` explicitly;
- price labels state currency or points where relevant;
- order button includes the actual action and size;
- risk denial/resizing is shown inline with reason codes translated to human text.

## Instrument picker

A shared component used everywhere:

- search by ticker or name;
- keyboard navigation;
- recent/favorites;
- shows ticker, short name, venue/type;
- avoids UID/FIGI unless advanced diagnostics are expanded;
- canonical instrument selection returns a stable instrument ID internally.

## Chart system

Chart requirements:

- candlesticks;
- selectable timeframe/date range;
- OHLC/volume on hover/crosshair;
- indicator overlays/panes;
- buy/sell/fill markers;
- dividends/corporate-event markers where available;
- current position/average price;
- stop/take-profit/working-order lines;
- synchronized instrument selection with other widgets.

Do not build a charting engine from scratch unless licensing/technical constraints force it. Use a mature chart library behind an abstraction.

## Widget model

Every widget defines:

- minimum/ideal size;
- data dependencies;
- loading state;
- stale/degraded state;
- empty state;
- error state;
- permissions;
- instrument/account context behavior.

Widgets must not individually open broker sessions. They subscribe to application read models/WebSocket topics.

## Visual tokens

Use semantic tokens rather than hard-coded component colors.

### Color roles

```text
bg.canvas
bg.surface
bg.elevated
border.default
text.primary
text.secondary
text.muted
accent.primary
state.positive
state.negative
state.warning
state.info
state.disabled
risk.safe
risk.warning
risk.blocked
```

Buy/sell colors must remain distinguishable in dark/light themes and not be the sole indicator of state; use icon/text as well.

### Typography

- UI sans-serif for controls/navigation;
- tabular numerals for prices, quantities, PnL and timestamps;
- compact line heights in tables/order book;
- hierarchy by size/weight, not excessive cards.

### Spacing

Base spacing unit: 4 px.

Recommended scale:

```text
4 / 8 / 12 / 16 / 24 / 32
```

Trading widgets prefer 8–12 px internal density; large 24–32 px spacing is reserved for page-level structure.

### Radius/elevation

Subtle. Avoid consumer-fintech oversized rounded cards. Dense terminal panels should read as one coherent workspace.

## Tables

Tables are first-class components, not generic data grids with arbitrary behavior.

Required capabilities:

- column resize/reorder;
- sticky header;
- compact/comfortable density;
- numeric alignment right;
- sorting/filtering;
- keyboard navigation where useful;
- streaming row updates without layout jumping;
- row-level state/action menu.

## Status language

Use stable status vocabulary across UI and API:

```text
Connected / Reconnecting / Disconnected
Ready / Reconciling / Degraded / Halted
Open / Partially filled / Filled / Canceled / Rejected / Unknown
Advisory / Approval required / Autonomous
Protected / Protection pending / Unprotected
```

`Unknown` is visually high-attention but must not be presented as failure/rejection.

## Responsive strategy

The primary product is desktop web. Responsive behavior prioritizes:

1. desktop 1440+;
2. laptop 1280;
3. tablet supervisory views;
4. future mobile client uses purpose-built workflows rather than shrinking the full terminal.

Do not force the complete desktop trading grid onto a phone viewport.

## Accessibility

- keyboard-operable core trading flow;
- visible focus states;
- adequate contrast;
- color-independent state indicators;
- screen-reader labels for controls;
- confirmation dialogs focus the destructive action correctly.

## Frontend architecture implications

The design system should be implemented as reusable tokens + components before feature pages:

```text
frontend/
  design-system/
    tokens/
    primitives/
    components/
    patterns/
  app/
    shell/
    workspaces/
    widgets/
```

Feature code may compose design-system components but must not introduce one-off control patterns without design-system review.

## Design acceptance

Before implementation of a major workspace, define and review:

- information architecture;
- primary user journeys;
- loading/empty/error/degraded states;
- keyboard behavior;
- widget dimensions;
- live/sandbox safety treatment.

This baseline prevents repeated full frontend redesigns while still allowing visual refinement without changing interaction contracts.