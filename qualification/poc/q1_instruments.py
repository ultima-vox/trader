from __future__ import annotations

from dataclasses import dataclass
from decimal import Decimal

from nautilus_trader.model.currencies import Currency
from nautilus_trader.model.enums import AssetClass
from nautilus_trader.model.identifiers import InstrumentId, Symbol
from nautilus_trader.model.instruments import Equity, FuturesContract
from nautilus_trader.model.objects import Price, Quantity


class MappingError(ValueError):
    """Raised when broker metadata is insufficient for an exact mapping."""


@dataclass(frozen=True)
class TInvestShareSpec:
    ticker: str
    class_code: str
    instrument_uid: str
    figi: str | None
    currency: str
    lot: int
    min_price_increment: Decimal
    api_trade_available: bool


@dataclass(frozen=True)
class TInvestFutureSpec:
    ticker: str
    class_code: str
    instrument_uid: str
    figi: str | None
    currency: str
    lot: int
    min_price_increment: Decimal
    min_price_increment_amount: Decimal
    underlying: str
    activation_ns: int
    expiration_ns: int
    api_trade_available: bool
    exchange_mic: str | None = None


def _precision(value: Decimal) -> int:
    normalized = value.normalize()
    return max(0, -normalized.as_tuple().exponent)


def _require_positive(name: str, value: Decimal | int) -> None:
    if value <= 0:
        raise MappingError(f"{name} must be positive, got {value!r}")


def future_money_per_point(spec: TInvestFutureSpec) -> Decimal:
    """Return settlement-currency value of one quoted point for one contract.

    T-Invest documents futures value as:

        price / min_price_increment * min_price_increment_amount

    Therefore the economically equivalent Nautilus contract multiplier is:

        min_price_increment_amount / min_price_increment

    This function deliberately fails closed when either broker value is absent,
    zero or negative. The adapter must refresh futures margin metadata instead of
    inventing a multiplier.
    """

    _require_positive("min_price_increment", spec.min_price_increment)
    _require_positive("min_price_increment_amount", spec.min_price_increment_amount)
    return spec.min_price_increment_amount / spec.min_price_increment


def future_value_money(spec: TInvestFutureSpec, quoted_price: Decimal, contracts: int = 1) -> Decimal:
    _require_positive("contracts", contracts)
    return quoted_price * future_money_per_point(spec) * Decimal(contracts)


def to_nautilus_equity(spec: TInvestShareSpec, *, ts_event: int = 0, ts_init: int = 0) -> Equity:
    _require_positive("lot", spec.lot)
    _require_positive("min_price_increment", spec.min_price_increment)

    return Equity(
        instrument_id=InstrumentId.from_str(f"{spec.ticker}.TINVEST"),
        raw_symbol=Symbol(spec.ticker),
        currency=Currency.from_str(spec.currency.upper()),
        price_precision=_precision(spec.min_price_increment),
        price_increment=Price.from_str(str(spec.min_price_increment)),
        lot_size=Quantity.from_int(spec.lot),
        ts_event=ts_event,
        ts_init=ts_init,
        info={
            "tinvest": {
                "instrument_uid": spec.instrument_uid,
                "figi": spec.figi,
                "class_code": spec.class_code,
                "api_trade_available": spec.api_trade_available,
            }
        },
    )


def to_nautilus_future(spec: TInvestFutureSpec, *, ts_event: int = 0, ts_init: int = 0) -> FuturesContract:
    _require_positive("lot", spec.lot)
    _require_positive("min_price_increment", spec.min_price_increment)

    multiplier = future_money_per_point(spec)

    return FuturesContract(
        instrument_id=InstrumentId.from_str(f"{spec.ticker}.TINVEST"),
        raw_symbol=Symbol(spec.ticker),
        asset_class=AssetClass.INDEX,
        currency=Currency.from_str(spec.currency.upper()),
        price_precision=_precision(spec.min_price_increment),
        price_increment=Price.from_str(str(spec.min_price_increment)),
        multiplier=Quantity.from_str(str(multiplier)),
        lot_size=Quantity.from_int(spec.lot),
        underlying=spec.underlying,
        activation_ns=spec.activation_ns,
        expiration_ns=spec.expiration_ns,
        ts_event=ts_event,
        ts_init=ts_init,
        exchange=spec.exchange_mic,
        info={
            "tinvest": {
                "instrument_uid": spec.instrument_uid,
                "figi": spec.figi,
                "class_code": spec.class_code,
                "api_trade_available": spec.api_trade_available,
                "min_price_increment_amount": str(spec.min_price_increment_amount),
                "money_per_point": str(multiplier),
            }
        },
    )
