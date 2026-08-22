from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime
from decimal import Decimal
from typing import Any

from nautilus_trader.model.currencies import Currency
from nautilus_trader.model.enums import AssetClass
from nautilus_trader.model.identifiers import InstrumentId, Symbol
from nautilus_trader.model.instruments import Equity, FuturesContract
from nautilus_trader.model.objects import Price, Quantity

from trader2.brokers.tinvest.transport import TInvestHttpTransport

SHARES_PATH = "/tinkoff.public.invest.api.contract.v1.InstrumentsService/Shares"
FUTURES_PATH = "/tinkoff.public.invest.api.contract.v1.InstrumentsService/Futures"
FUTURES_MARGIN_PATH = "/tinkoff.public.invest.api.contract.v1.InstrumentsService/GetFuturesMargin"


class InstrumentMappingError(ValueError):
    pass


@dataclass(frozen=True, slots=True)
class TInvestInstrumentIdentity:
    ticker: str
    class_code: str
    uid: str
    figi: str | None


def _quotation(value: Any, *, field: str) -> Decimal:
    if not isinstance(value, dict):
        raise InstrumentMappingError(f"{field} is missing")
    try:
        units = Decimal(str(value.get("units", "0")))
        nano = Decimal(str(value.get("nano", 0))) / Decimal("1000000000")
    except Exception as exc:
        raise InstrumentMappingError(f"{field} is invalid") from exc
    return units + nano


def _positive(name: str, value: Decimal | int) -> None:
    if value <= 0:
        raise InstrumentMappingError(f"{name} must be positive, got {value!r}")


def _precision(value: Decimal) -> int:
    normalized = value.normalize()
    return max(0, -normalized.as_tuple().exponent)


def _iso_ns(value: str | None, *, field: str) -> int:
    if not value:
        raise InstrumentMappingError(f"{field} is missing")
    normalized = value.replace("Z", "+00:00")
    try:
        return int(datetime.fromisoformat(normalized).timestamp() * 1_000_000_000)
    except ValueError as exc:
        raise InstrumentMappingError(f"{field} is invalid: {value!r}") from exc


def _asset_class(value: str | None) -> AssetClass:
    normalized = (value or "").strip().upper()
    if normalized.startswith("TYPE_"):
        normalized = normalized[5:]
    mapping = {
        "SECURITY": AssetClass.EQUITY,
        "INDEX": AssetClass.INDEX,
        "CURRENCY": AssetClass.FX,
        "COMMODITY": AssetClass.COMMODITY,
    }
    try:
        return mapping[normalized]
    except KeyError as exc:
        raise InstrumentMappingError(f"unsupported T-Invest future asset_type={value!r}") from exc


class TInvestInstrumentProvider:
    """Loads authoritative T-Invest instrument metadata and maps it into Nautilus types."""

    def __init__(self, transport: TInvestHttpTransport) -> None:
        self._transport = transport
        self._by_uid: dict[str, Equity | FuturesContract] = {}
        self._by_id: dict[InstrumentId, Equity | FuturesContract] = {}

    async def load(self) -> None:
        common = {
            "instrumentStatus": "INSTRUMENT_STATUS_BASE",
            "instrumentExchange": "INSTRUMENT_EXCHANGE_UNSPECIFIED",
        }
        shares_response = await self._transport.post(SHARES_PATH, common)
        futures_response = await self._transport.post(FUTURES_PATH, common)

        mapped: list[Equity | FuturesContract] = []
        for raw in shares_response.get("instruments") or []:
            if raw.get("apiTradeAvailableFlag") is not True:
                continue
            mapped.append(self._map_share(raw))

        for raw in futures_response.get("instruments") or []:
            if raw.get("apiTradeAvailableFlag") is not True:
                continue
            margin = await self._transport.post(
                FUTURES_MARGIN_PATH,
                {"instrumentId": self._require_text(raw, "uid")},
            )
            mapped.append(self._map_future(raw, margin))

        self._by_uid = {
            str(item.info["tinvest"]["instrument_uid"]): item
            for item in mapped
        }
        self._by_id = {item.id: item for item in mapped}

    def all(self) -> tuple[Equity | FuturesContract, ...]:
        return tuple(self._by_id.values())

    def by_uid(self, uid: str) -> Equity | FuturesContract | None:
        return self._by_uid.get(uid)

    def by_id(self, instrument_id: InstrumentId) -> Equity | FuturesContract | None:
        return self._by_id.get(instrument_id)

    @staticmethod
    def _require_text(raw: dict[str, Any], field: str) -> str:
        value = raw.get(field)
        if not isinstance(value, str) or not value.strip():
            raise InstrumentMappingError(f"{field} is missing")
        return value.strip()

    def _map_share(self, raw: dict[str, Any]) -> Equity:
        ticker = self._require_text(raw, "ticker")
        currency = self._require_text(raw, "currency")
        uid = self._require_text(raw, "uid")
        class_code = self._require_text(raw, "classCode")
        lot = int(raw.get("lot", 0))
        tick = _quotation(raw.get("minPriceIncrement"), field="share.minPriceIncrement")
        _positive("share.lot", lot)
        _positive("share.minPriceIncrement", tick)

        return Equity(
            instrument_id=InstrumentId.from_str(f"{ticker}.TINVEST"),
            raw_symbol=Symbol(ticker),
            currency=Currency.from_str(currency.upper()),
            price_precision=_precision(tick),
            price_increment=Price.from_str(str(tick)),
            lot_size=Quantity.from_int(lot),
            ts_event=0,
            ts_init=0,
            info={
                "tinvest": {
                    "instrument_uid": uid,
                    "figi": raw.get("figi") or None,
                    "class_code": class_code,
                    "api_trade_available": True,
                }
            },
        )

    def _map_future(self, raw: dict[str, Any], margin: dict[str, Any]) -> FuturesContract:
        ticker = self._require_text(raw, "ticker")
        currency = self._require_text(raw, "currency")
        uid = self._require_text(raw, "uid")
        class_code = self._require_text(raw, "classCode")
        lot = int(raw.get("lot", 0))
        tick = _quotation(raw.get("minPriceIncrement"), field="future.minPriceIncrement")
        margin_tick = _quotation(margin.get("minPriceIncrement"), field="margin.minPriceIncrement")
        tick_amount = _quotation(
            margin.get("minPriceIncrementAmount"),
            field="margin.minPriceIncrementAmount",
        )
        _positive("future.lot", lot)
        _positive("future.minPriceIncrement", tick)
        _positive("margin.minPriceIncrement", margin_tick)
        _positive("margin.minPriceIncrementAmount", tick_amount)
        if tick != margin_tick:
            raise InstrumentMappingError(
                f"future tick mismatch for {ticker}: catalogue={tick} margin={margin_tick}"
            )
        multiplier = tick_amount / tick
        underlying = (
            raw.get("basicAsset")
            or raw.get("basicAssetPositionUid")
            or raw.get("assetUid")
            or ticker
        )

        return FuturesContract(
            instrument_id=InstrumentId.from_str(f"{ticker}.TINVEST"),
            raw_symbol=Symbol(ticker),
            asset_class=_asset_class(raw.get("assetType")),
            currency=Currency.from_str(currency.upper()),
            price_precision=_precision(tick),
            price_increment=Price.from_str(str(tick)),
            multiplier=Quantity.from_str(str(multiplier)),
            lot_size=Quantity.from_int(lot),
            underlying=str(underlying),
            activation_ns=_iso_ns(raw.get("firstTradeDate"), field="future.firstTradeDate"),
            expiration_ns=_iso_ns(raw.get("expirationDate"), field="future.expirationDate"),
            ts_event=0,
            ts_init=0,
            info={
                "tinvest": {
                    "instrument_uid": uid,
                    "figi": raw.get("figi") or None,
                    "class_code": class_code,
                    "api_trade_available": True,
                    "min_price_increment_amount": str(tick_amount),
                    "money_per_point": str(multiplier),
                }
            },
        )
