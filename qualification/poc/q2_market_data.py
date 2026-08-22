from __future__ import annotations

from dataclasses import dataclass
from decimal import Decimal
from hashlib import blake2b
from typing import Any

from nautilus_trader.model.data import Bar, BarType, TradeTick
from nautilus_trader.model.enums import AggressorSide
from nautilus_trader.model.identifiers import InstrumentId, TradeId
from nautilus_trader.model.objects import Price, Quantity


class MarketDataMappingError(ValueError):
    """Raised when broker market data cannot be mapped without guessing."""


def _require_positive(name: str, value: Decimal | int) -> None:
    if value <= 0:
        raise MarketDataMappingError(f"{name} must be positive, got {value!r}")


def lots_to_units(lots: int, lot_size: int) -> int:
    """Convert T-Invest market-data lots to Nautilus instrument units."""
    _require_positive("lots", lots)
    _require_positive("lot_size", lot_size)
    return lots * lot_size


def _trade_aggressor(direction: str) -> AggressorSide:
    normalized = direction.strip().upper()
    mapping = {
        "TRADE_DIRECTION_BUY": AggressorSide.BUYER,
        "TRADE_DIRECTION_SELL": AggressorSide.SELLER,
    }
    try:
        return mapping[normalized]
    except KeyError as exc:
        raise MarketDataMappingError(f"unsupported trade direction={direction!r}") from exc


def derived_tinvest_trade_id(
    *,
    instrument_uid: str,
    ts_event: int,
    price: Decimal,
    quantity_lots: int,
    direction: str,
    trade_source: str,
) -> TradeId:
    """Derive a deterministic ID because T-Invest public Trade has no trade ID.

    This is an adapter-level compatibility key, not a claim that T-Invest supplied
    a venue trade identifier. The provider cannot distinguish two economically
    identical public trades carrying exactly the same exposed fields.
    """
    payload = "|".join(
        [
            instrument_uid,
            str(ts_event),
            format(price, "f"),
            str(quantity_lots),
            direction,
            trade_source,
        ]
    ).encode("utf-8")
    return TradeId(blake2b(payload, digest_size=16).hexdigest())


def to_nautilus_trade_tick(
    *,
    instrument_id: InstrumentId,
    instrument_uid: str,
    price: Decimal,
    quantity_lots: int,
    lot_size: int,
    direction: str,
    trade_source: str,
    ts_event: int,
    ts_init: int,
) -> TradeTick:
    units = lots_to_units(quantity_lots, lot_size)
    trade_id = derived_tinvest_trade_id(
        instrument_uid=instrument_uid,
        ts_event=ts_event,
        price=price,
        quantity_lots=quantity_lots,
        direction=direction,
        trade_source=trade_source,
    )
    return TradeTick(
        instrument_id=instrument_id,
        price=Price.from_str(str(price)),
        size=Quantity.from_int(units),
        aggressor_side=_trade_aggressor(direction),
        trade_id=trade_id,
        ts_event=ts_event,
        ts_init=ts_init,
    )


def to_nautilus_bar(
    *,
    bar_type: BarType,
    open_price: Decimal,
    high_price: Decimal,
    low_price: Decimal,
    close_price: Decimal,
    volume_lots: int,
    lot_size: int,
    ts_event: int,
    ts_init: int,
) -> Bar:
    units = lots_to_units(volume_lots, lot_size)
    return Bar(
        bar_type=bar_type,
        open=Price.from_str(str(open_price)),
        high=Price.from_str(str(high_price)),
        low=Price.from_str(str(low_price)),
        close=Price.from_str(str(close_price)),
        volume=Quantity.from_int(units),
        ts_event=ts_event,
        ts_init=ts_init,
    )


@dataclass(frozen=True)
class BookLevel:
    price: Decimal
    quantity_units: int


@dataclass(frozen=True)
class NormalizedBookSnapshot:
    bids: tuple[BookLevel, ...]
    asks: tuple[BookLevel, ...]
    depth: int
    is_consistent: bool | None
    ts_event: int


def normalize_book_snapshot(
    *,
    bids: list[dict[str, Any]],
    asks: list[dict[str, Any]],
    lot_size: int,
    depth: int,
    ts_event: int,
    quotation_parser,
    is_consistent: bool | None = None,
) -> NormalizedBookSnapshot:
    _require_positive("lot_size", lot_size)
    _require_positive("depth", depth)

    def convert(level: dict[str, Any]) -> BookLevel:
        quantity_lots = int(level.get("quantity", 0))
        return BookLevel(
            price=quotation_parser(level.get("price"), field="order_book.price"),
            quantity_units=lots_to_units(quantity_lots, lot_size),
        )

    return NormalizedBookSnapshot(
        bids=tuple(convert(level) for level in bids),
        asks=tuple(convert(level) for level in asks),
        depth=depth,
        is_consistent=is_consistent,
        ts_event=ts_event,
    )
