# Q1 — T-Invest instrument semantics on NautilusTrader

Status: **IN PROGRESS**

## Current finding

The NautilusTrader domain model has the required high-level instrument types for the first qualification slice:

- `Equity` for MOEX shares;
- `FuturesContract` for dated MOEX futures.

The current Python API exposes the material fields required by this PoC: instrument identity, raw symbol, currency, price precision/increment, lot size, futures multiplier, underlying, activation/expiration timestamps, optional exchange and adapter metadata.

## Futures economics invariant

T-Invest documents futures monetary value as:

```text
money_value = quoted_price / min_price_increment * min_price_increment_amount
```

This is algebraically equivalent to:

```text
money_value = quoted_price * money_per_point
money_per_point = min_price_increment_amount / min_price_increment
```

The PoC maps `money_per_point` to the Nautilus futures contract `multiplier` and preserves the original T-Invest values in adapter metadata.

This mapping is **not accepted yet**. It must be verified against live authoritative `GetFuturesMargin` values and the broker-reported portfolio/PnL for at least one actual MOEX futures contract.

## Fail-closed rules implemented

The PoC rejects:

- zero/negative `lot`;
- zero/negative `min_price_increment`;
- zero/negative `min_price_increment_amount` for futures.

It does not infer a futures asset class. The caller must provide an explicit Nautilus `AssetClass` derived from authoritative instrument/reference-data mapping.

## Quantity semantics requiring live verification

T-Invest market-data order-book/trade quantities are expressed in lots, while quoted prices are per instrument. Nautilus quantities are instrument units and expose `lot_size` separately.

The proposed adapter rule is:

```text
broker quantity in lots * broker lot size -> Nautilus quantity in instrument units
```

This must be verified with a real share and a real future before Q1 is accepted.

## Evidence currently available

Implemented on branch `codex/nautilus-q1-q2-poc`:

- exact Decimal point-to-money formula;
- synthetic futures test vectors;
- fail-closed invalid metadata test;
- `Equity` constructor mapping;
- `FuturesContract` constructor mapping;
- preservation of T-Invest UID/FIGI/class-code metadata.

Synthetic vectors are deliberately not presented as real broker values.

## External facts used

- T-Invest: futures prices are quoted in points; monetary value uses `price / min_price_increment * min_price_increment_amount`.
- T-Invest: `min_price_increment_amount` can change and should be obtained from `GetFuturesMargin`.
- T-Invest: order-book and trade quantities are expressed in lots.
- NautilusTrader: `FuturesContract` supports `price_increment`, `multiplier`, `lot_size`, underlying and expiry.

## Remaining acceptance work

1. Install and execute against pinned NautilusTrader `1.231.0`.
2. Pull one real MOEX share from T-Invest and capture authoritative metadata.
3. Pull one real MOEX future plus `GetFuturesMargin`.
4. Instantiate both Nautilus instruments and round-trip material fields.
5. Compare futures monetary value/notional/PnL semantics with T-Invest.
6. Verify lot-to-unit quantity conversion using actual market-data payloads.

## Current verdict

**NOT YET QUALIFIED.** No incompatibility has been found in Q1 so far, but live broker evidence is still required.
