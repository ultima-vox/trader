from __future__ import annotations

import asyncio
from typing import Any

from nautilus_trader.model.instruments import Equity, FuturesContract

from trader2.brokers.tinvest.instruments import InstrumentFamily, TInvestInstrumentProvider


class FakeTransport:
    async def post(self, path: str, payload: dict[str, Any]) -> dict[str, Any]:
        if path.endswith("/Shares"):
            return {"instruments": [self._instrument("SBER", "share-uid", "TQBR", "rub", 10)]}
        if path.endswith("/Bonds"):
            return {"instruments": [self._instrument("BOND1", "bond-uid", "TQOB", "rub", 1)]}
        if path.endswith("/Etfs"):
            return {"instruments": [self._instrument("ETF1", "etf-uid", "TQTF", "rub", 1)]}
        if path.endswith("/Currencies"):
            return {"instruments": [self._instrument("USDRUB", "currency-uid", "CETS", "rub", 1)]}
        if path.endswith("/Futures"):
            return {
                "instruments": [
                    {
                        **self._instrument("TESTF", "future-uid", "SPBFUT", "rub", 1),
                        "assetType": "TYPE_COMMODITY",
                        "basicAsset": "TEST",
                        "firstTradeDate": "2026-01-01T00:00:00Z",
                        "expirationDate": "2026-12-31T00:00:00Z",
                    }
                ]
            }
        if path.endswith("/StructuredNotes"):
            return {"instruments": [self._instrument("NOTE1", "note-uid", "SP", "rub", 1)]}
        if path.endswith("/Dfas"):
            return {"instruments": [self._instrument("DFA1", "dfa-uid", "DFA", "rub", 1)]}
        if path.endswith("/Indicatives"):
            return {
                "instruments": [
                    {
                        **self._instrument("IMOEX", "index-uid", "INDEX", "rub", 1),
                        "apiTradeAvailableFlag": False,
                    }
                ]
            }
        if path.endswith("/GetFuturesMargin"):
            assert payload["instrumentId"] == "future-uid"
            return {
                "minPriceIncrement": {"units": "0", "nano": 1000000},
                "minPriceIncrementAmount": {"units": "0", "nano": 833550000},
            }
        if path.endswith("/OptionsBy"):
            assert payload["basicAssetUid"] == "share-uid"
            return {
                "instruments": [
                    {
                        **self._instrument("SBEROPT", "option-uid", "SPBOPT", "rub", 1),
                        "expirationDate": "2026-12-31T00:00:00Z",
                        "basicAssetUid": "share-uid",
                    }
                ]
            }
        if path.endswith("/FindInstrument"):
            assert payload["query"] == "SBER"
            return {
                "instruments": [
                    {
                        **self._instrument("SBER", "share-uid", "TQBR", "rub", 10),
                        "instrumentType": "INSTRUMENT_TYPE_SHARE",
                    }
                ]
            }
        raise AssertionError(path)

    @staticmethod
    def _instrument(ticker: str, uid: str, class_code: str, currency: str, lot: int) -> dict[str, Any]:
        return {
            "ticker": ticker,
            "classCode": class_code,
            "uid": uid,
            "figi": f"{uid}-figi",
            "name": ticker,
            "currency": currency,
            "lot": lot,
            "minPriceIncrement": {"units": "0", "nano": 1000000},
            "apiTradeAvailableFlag": True,
        }


def test_provider_loads_all_top_level_instrument_families() -> None:
    provider = TInvestInstrumentProvider(FakeTransport())  # type: ignore[arg-type]
    asyncio.run(provider.load())

    records = provider.all_records()
    assert {record.family for record in records} == {
        InstrumentFamily.SHARE,
        InstrumentFamily.BOND,
        InstrumentFamily.ETF,
        InstrumentFamily.CURRENCY,
        InstrumentFamily.FUTURE,
        InstrumentFamily.STRUCTURED_NOTE,
        InstrumentFamily.DFA,
        InstrumentFamily.INDICATIVE,
    }
    assert provider.record_by_uid("bond-uid").family is InstrumentFamily.BOND  # type: ignore[union-attr]
    assert provider.record_by_uid("index-uid").api_trade_available is False  # type: ignore[union-attr]


def test_provider_keeps_exact_nautilus_mappings_separate_from_full_catalogue() -> None:
    provider = TInvestInstrumentProvider(FakeTransport())  # type: ignore[arg-type]
    asyncio.run(provider.load())

    runtime_instruments = provider.all()
    assert len(runtime_instruments) == 2

    share = provider.by_uid("share-uid")
    future = provider.by_uid("future-uid")
    assert isinstance(share, Equity)
    assert str(share.id) == "SBER.TINVEST"
    assert str(share.lot_size) == "10"
    assert isinstance(future, FuturesContract)
    assert str(future.id) == "TESTF.TINVEST"
    assert str(future.multiplier) == "833.55"
    assert future.info["tinvest"]["money_per_point"] == "833.55"

    assert provider.by_uid("bond-uid") is None
    assert provider.record_by_uid("bond-uid") is not None


def test_options_by_uses_non_deprecated_endpoint_and_returns_typed_records() -> None:
    provider = TInvestInstrumentProvider(FakeTransport())  # type: ignore[arg-type]
    options = asyncio.run(provider.options_by(basic_asset_uid="share-uid"))

    assert len(options) == 1
    assert options[0].family is InstrumentFamily.OPTION
    assert options[0].identity.uid == "option-uid"
    assert options[0].basic_asset_uid == "share-uid"


def test_find_instrument_returns_typed_records() -> None:
    provider = TInvestInstrumentProvider(FakeTransport())  # type: ignore[arg-type]
    records = asyncio.run(provider.find("SBER"))

    assert len(records) == 1
    assert records[0].family is InstrumentFamily.SHARE
    assert records[0].identity.ticker == "SBER"
