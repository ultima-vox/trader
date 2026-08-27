# Vox Trader canonical design authority

## Canonical visual source

Restored working file:

`docs/design/source/Vox-Trader-Design-System-canonical.html`

The exact uploaded source is preserved byte-for-byte in the repository as three XZ/base64 text parts and reconstructed with:

```bash
bash docs/design/source/restore-canonical-design.sh
```

The restore script verifies SHA-256 before and after decompression. See `docs/design/source/README.md` for integrity details.

This source is the **visual and component design authority** for Vox Trader.

It defines the canonical visual language, density, geometry, typography, component character, trading-terminal interaction model and reference composition. Repository-native design-system implementation must converge to this source rather than create a competing visual direction.

## Conflict resolution

When the canonical visual source and executable repository contracts differ, resolve them in this order:

1. **Backend/runtime truth wins for semantics.** Current accepted Rust/domain/runtime contracts define real states, enums, capabilities and financial behavior.
2. **Canonical visual source wins for visual language and component geometry.** Preserve its palette, typography, density, spacing, table character, controls, widgets and operator-terminal feel unless a proven accessibility or layout defect requires a local correction.
3. **Safety wins over convenience.** Capital-affecting target, environment, authorization, risk/protection/reconciliation and uncertainty must remain explicit.
4. **Unsupported capabilities are deferred, never simulated.** If no accepted backend contract exists, render a disabled/deferred state with the tracked dependency rather than fake operational data.
5. **Exact financial values remain exact.** Never introduce JSON/JavaScript floating-point arithmetic for capital-affecting values.
6. **Provider wire DTOs and secrets never enter normal UI contracts.**

## What must be preserved from the canonical visual source

- dark graphite professional terminal character;
- cold blue neutral accent; BUY/SELL remain financial positive/negative actions rather than primary accent semantics;
- Inter + JetBrains Mono;
- base UI size 13px;
- 4px spacing grid;
- Compact as desktop default;
- Compact geometry: control 28px, table row 26px, table header 28px, widget header 32px;
- restrained 2/4/6/8px radii, default control 4px and default widget 6px;
- borders and surface hierarchy before decorative spacing/shadows;
- shadows only for overlay layers;
- dense stable tables with tabular numbers;
- one shared Instrument Picker;
- compact dual-action Order Ticket;
- professional Order Book and Trade Tape;
- chart event vocabulary `B / S / F / SL / TP / D / E`;
- widget/context/workspace interaction philosophy;
- keyboard-first interaction and explicit financial units;
- UNKNOWN as a distinct semantic state, never ordinary failure.

## Production hardening imported from the repository

The unified design must retain accepted production corrections from the repository:

- current canonical backend enums and reason codes;
- `SANDBOX / PRODUCTION` broker environment semantics;
- separation of future PAPER/BACKTEST trading mode from broker environment;
- frozen execution target;
- `UNKNOWN_AFTER_DISPATCH` and broker-authoritative reconciliation;
- canonical protection lifecycle from backend contracts;
- execution authorization distinct from credential capability;
- multi-account/broker context safety;
- exact fixed-point handling;
- explicit deferred capabilities;
- strict per-widget collision verification;
- 1280 / 1440 / 1920 × Compact / Standard / Comfortable verification;
- no clipping, overlap, scrollbar collision or truncation of capital-affecting values;
- accessibility and reduced-motion constraints.

## Repository-native target

The canonical source is provenance/reference. It is **not** a runtime dependency.

The executable design system remains repository-native under:

```text
frontend/design-system/
  tokens/
  primitives/
  components/
  patterns/
  reference/
```

The reconciliation task must produce **one** authoritative repository-native Vox Trader Design System and **one** published Pages reference. Duplicate/competing component definitions and alternate visual systems must be removed or archived as provenance.

## Governance

After reconciliation:

- the uploaded canonical source remains unchanged as provenance;
- repository tokens/components/reference become the executable implementation of that canon;
- backend contract changes may update semantics, states and deferred capabilities without silently changing the visual language;
- visual redesign requires an explicit product/design decision rather than incidental feature work.
