from __future__ import annotations

import asyncio
from typing import Any

from nautilus_trader.model.instruments import Equity, FuturesContract

from trader2.brokers.tinvest.instruments import TInvestInstrumentProvider


class FakeTransport:
    async def post(self, path: str, payload: dict[str, Any]) -> dict[str, Any]:
        if path.endswith("/Shares"):
            return {
                "instruments": [
                    {
                        "ticker": "SBER",
                        "classCode": "TQBR",
                        "uid": "share-uid",
                        "figi": "share-figi",
                        "currency": "rub",
                        "lot": 10,
                        "minPriceIncrement": {"units": "0", "nano": 10000000},
                        "apiTradeAvailableFlag": True,
                    }
                ]
            }
        if path.endswith("/Futures"):
            return {
                "instruments": [
                    {
                        "ticker": "TESTF",
                        "classCode": "SPBFUT",
                        "uid": "future-uid",
                        "figi": "future-figi",
                        "currency": "rub",
                        "lot": 1,
                        "minPriceIncrement": {"units": "0", "nano": 1000000},
                        "apiTradeAvailableFlag": True,
                        "assetType": "TYPE_COMMODITY",
                        "basicAsset": "TEST",
                        "firstTradeDate": "2026-01-01T00:00:00Z",
                        "expirationDate": "2026-12-31T00:00:00Z",
                    }
                ]
            }
        if path.endswith("/GetFuturesMargin"):
            assert payload["instrumentId"] == "future-uid"
            return {
                "minPriceIncrement": {"units": "0", "nano": 1000000},
                "minPriceIncrementAmount": {"units": "0", "nano": 833550000},
            }
        raise AssertionError(path)


def test_provider_maps_share_and_future_without_qualification_imports() -> None:
    provider = TInvestInstrumentProvider(FakeTransport())  # type: ignore[arg-type]
    asyncio.run(provider.load())

    instruments = provider.all()
    assert len(instruments) == 2

    share = provider.by_uid("share-uid")
    future = provider.by_uid("future-uid")

    assert isinstance(share, Equity)
    assert str(share.id) == "SBER.TINVEST"
    assert str(share.lot_size) == "10"

    assert isinstance(future, FuturesContract)
    assert str(future.id) == "TESTF.TINVEST"
    assert str(future.multiplier) == "833.55"
    assert future.info["tinvest"]["money_per_point"] == "833.55"
