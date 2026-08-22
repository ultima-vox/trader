from decimal import Decimal

import pytest
from nautilus_trader.model.data import BarType
from nautilus_trader.model.identifiers import InstrumentId

from qualification.poc.q2_market_data import MarketDataMappingError
from qualification.poc.q2_market_data import derived_tinvest_trade_id
from qualification.poc.q2_market_data import lots_to_units
from qualification.poc.q2_market_data import to_nautilus_bar
from qualification.poc.q2_market_data import to_nautilus_trade_tick


def test_lots_to_units_uses_authoritative_lot_size() -> None:
    assert lots_to_units(3, 10) == 30


def test_lots_to_units_fails_closed() -> None:
    with pytest.raises(MarketDataMappingError):
        lots_to_units(0, 10)
    with pytest.raises(MarketDataMappingError):
        lots_to_units(1, 0)


def test_trade_id_derivation_is_deterministic() -> None:
    kwargs = dict(
        instrument_uid="uid-1",
        ts_event=123,
        price=Decimal("100.25"),
        quantity_lots=2,
        direction="TRADE_DIRECTION_BUY",
        trade_source="TRADE_SOURCE_EXCHANGE",
    )
    assert derived_tinvest_trade_id(**kwargs) == derived_tinvest_trade_id(**kwargs)


def test_trade_tick_maps_lots_to_units() -> None:
    tick = to_nautilus_trade_tick(
        instrument_id=InstrumentId.from_str("TEST.TINVEST"),
        instrument_uid="uid-1",
        price=Decimal("100.25"),
        quantity_lots=2,
        lot_size=10,
        direction="TRADE_DIRECTION_BUY",
        trade_source="TRADE_SOURCE_EXCHANGE",
        ts_event=123,
        ts_init=124,
    )
    assert str(tick.size) == "20"
    assert str(tick.price) == "100.25"


def test_bar_maps_broker_volume_lots_to_units() -> None:
    bar = to_nautilus_bar(
        bar_type=BarType.from_str("TEST.TINVEST-1-MINUTE-LAST-EXTERNAL"),
        open_price=Decimal("100"),
        high_price=Decimal("102"),
        low_price=Decimal("99"),
        close_price=Decimal("101"),
        volume_lots=7,
        lot_size=10,
        ts_event=123,
        ts_init=124,
    )
    assert str(bar.volume) == "70"
    assert str(bar.close) == "101"
