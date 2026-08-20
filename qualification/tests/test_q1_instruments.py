from decimal import Decimal

import pytest

from qualification.poc.q1_instruments import MappingError
from qualification.poc.q1_instruments import TInvestFutureSpec
from qualification.poc.q1_instruments import TInvestShareSpec
from qualification.poc.q1_instruments import future_money_per_point
from qualification.poc.q1_instruments import future_value_money
from qualification.poc.q1_instruments import to_nautilus_equity
from qualification.poc.q1_instruments import to_nautilus_future


# These are synthetic qualification vectors. They intentionally do not claim to
# be current metadata for any real T-Invest instrument. Live broker vectors are
# required before Q1 can be marked PASS.


def test_future_point_to_money_formula_matches_tinvest_documented_formula() -> None:
    spec = TInvestFutureSpec(
        ticker="TESTF",
        class_code="SPBFUT",
        instrument_uid="synthetic-future",
        figi=None,
        currency="RUB",
        lot=1,
        min_price_increment=Decimal("10"),
        min_price_increment_amount=Decimal("12.50"),
        underlying="TEST",
        activation_ns=1,
        expiration_ns=2,
        api_trade_available=True,
        exchange_mic="MISX",
    )

    assert future_money_per_point(spec) == Decimal("1.25")
    assert future_value_money(spec, Decimal("100000")) == Decimal("125000.00")


def test_future_mapping_fails_closed_without_valid_tick_money_metadata() -> None:
    spec = TInvestFutureSpec(
        ticker="TESTF",
        class_code="SPBFUT",
        instrument_uid="synthetic-future",
        figi=None,
        currency="RUB",
        lot=1,
        min_price_increment=Decimal("10"),
        min_price_increment_amount=Decimal("0"),
        underlying="TEST",
        activation_ns=1,
        expiration_ns=2,
        api_trade_available=True,
    )

    with pytest.raises(MappingError):
        future_money_per_point(spec)


def test_equity_mapping_preserves_board_lot_and_broker_identity() -> None:
    spec = TInvestShareSpec(
        ticker="TEST",
        class_code="TQBR",
        instrument_uid="synthetic-share",
        figi="SYNTHETIC",
        currency="RUB",
        lot=10,
        min_price_increment=Decimal("0.01"),
        api_trade_available=True,
    )

    instrument = to_nautilus_equity(spec)

    assert str(instrument.id) == "TEST.TINVEST"
    assert str(instrument.lot_size) == "10"
    assert instrument.info["tinvest"]["instrument_uid"] == "synthetic-share"
    assert instrument.info["tinvest"]["class_code"] == "TQBR"


def test_future_mapping_uses_broker_tick_value_as_contract_multiplier() -> None:
    spec = TInvestFutureSpec(
        ticker="TESTF",
        class_code="SPBFUT",
        instrument_uid="synthetic-future",
        figi="SYNTHETIC-FUT",
        currency="RUB",
        lot=1,
        min_price_increment=Decimal("5"),
        min_price_increment_amount=Decimal("7.50"),
        underlying="TEST",
        activation_ns=1,
        expiration_ns=2,
        api_trade_available=True,
        exchange_mic="MISX",
    )

    instrument = to_nautilus_future(spec)

    assert str(instrument.id) == "TESTF.TINVEST"
    assert str(instrument.multiplier) == "1.5"
    assert instrument.info["tinvest"]["min_price_increment_amount"] == "7.50"
