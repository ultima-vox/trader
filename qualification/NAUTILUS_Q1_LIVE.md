# Nautilus Q1 live T-Invest qualification

Status: **PENDING LIVE EVIDENCE**

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

A successful run is evidence for Q1, but the output must still be recorded and reviewed before the Q1 verdict changes from pending.

Do not commit broker tokens or raw secret-bearing HTTP traces.
