# Nautilus Q1 live T-Invest qualification

Status: **LIVE EVIDENCE CAPTURED — REVIEW REQUIRED**

Run from the repository root on `codex/nautilus-q1-q2-poc`:

```bash
export TINVEST_TOKEN='...'
python -m qualification.live.q1_tinvest
```

The runner uses the current T-Invest REST API directly and does not persist the token.

It:

1. fetches the BASE share catalogue;
2. resolves exactly one API-tradeable `SBER/TQBR` share;
3. fetches the BASE futures catalogue;
4. chooses the nearest currently active API-tradeable `SPBFUT` contract;
5. calls `GetFuturesMargin` for that future;
6. requires the catalogue and margin tick sizes to agree;
7. derives money-per-point exactly as `min_price_increment_amount / min_price_increment`;
8. maps both instruments into Nautilus types;
9. fails closed for missing/unknown economic metadata.

## Captured live evidence

Observed successful run:

```text
Q1 LIVE QUALIFICATION
=====================
SBER:   SBER/TQBR uid=e6123145-9665-43e0-8413-cd61b8aa9b13
        lot=1 tick=0.01 nautilus_id=SBER.TINVEST
FUTURE: KCQ6/SPBFUT uid=2d9fb16b-71c1-4d0b-bebf-c73c12e2b802 asset_type=TYPE_COMMODITY
        tick=0.001 tick_amount=0.83355 money_per_point=833.55
        lot=1 multiplier=833.55 nautilus_id=KCQ6.TINVEST
        initial_margin_buy=345.770000000 initial_margin_sell=345.770000000
PASS: live T-Invest instrument metadata mapped into Nautilus without approximation
```

The `TYPE_COMMODITY` enum form observed from the live T-Invest API is normalized explicitly to Nautilus `AssetClass.COMMODITY`; unknown values remain fail-closed.

## Current interpretation

This is sufficient evidence that the current Q1 adapter mapping can represent:

- live T-Invest share identity, lot and tick semantics;
- live MOEX futures identity and expiry-side metadata used by the mapper;
- broker-authoritative futures `min_price_increment_amount`;
- exact point-to-money multiplier derivation without approximation;
- commodity futures asset-class mapping into Nautilus.

Q1 should not be considered fully accepted until the evidence is reviewed together with the synthetic regression tests and the remaining lot-to-unit/portfolio-PnL parity checks described in `NAUTILUS_Q1_INSTRUMENTS.md`.

Do not commit broker tokens or raw secret-bearing HTTP traces.
