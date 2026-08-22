from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime
from decimal import Decimal
from enum import StrEnum
from typing import Any, Iterable

from nautilus_trader.model.currencies import Currency
from nautilus_trader.model.enums import AssetClass
from nautilus_trader.model.identifiers import InstrumentId, Symbol
from nautilus_trader.model.instruments import Equity, FuturesContract
from nautilus_trader.model.objects import Price, Quantity

from trader2.brokers.tinvest.transport import TInvestHttpTransport

INSTRUMENTS_SERVICE = "/tinkoff.public.invest.api.contract.v1.InstrumentsService"
SHARES_PATH = f"{INSTRUMENTS_SERVICE}/Shares"
BONDS_PATH = f"{INSTRUMENTS_SERVICE}/Bonds"
ETFS_PATH = f"{INSTRUMENTS_SERVICE}/Etfs"
CURRENCIES_PATH = f"{INSTRUMENTS_SERVICE}/Currencies"
FUTURES_PATH = f"{INSTRUMENTS_SERVICE}/Futures"
STRUCTURED_NOTES_PATH = f"{INSTRUMENTS_SERVICE}/StructuredNotes"
DFAS_PATH = f"{INSTRUMENTS_SERVICE}/Dfas"
INDICATIVES_PATH = f"{INSTRUMENTS_SERVICE}/Indicatives"
OPTIONS_BY_PATH = f"{INSTRUMENTS_SERVICE}/OptionsBy"
FIND_INSTRUMENT_PATH = f"{INSTRUMENTS_SERVICE}/FindInstrument"
FUTURES_MARGIN_PATH = f"{INSTRUMENTS_SERVICE}/GetFuturesMargin"


class InstrumentMappingError(ValueError):
    pass


class InstrumentFamily(StrEnum):
    SHARE = "share"
    BOND = "bond"
    ETF = "etf"
    CURRENCY = "currency"
    FUTURE = "future"
    OPTION = "option"
    STRUCTURED_NOTE = "structured_note"
    DFA = "dfa"
    INDICATIVE = "indicative"


@dataclass(frozen=True, slots=True)
class TInvestInstrumentIdentity:
    ticker: str
    class_code: str
    uid: str
    figi: str | None
    position_uid: str | None = None


@dataclass(frozen=True, slots=True)
class TInvestInstrumentRecord:
    """Typed provider-facing reference record for every T-Invest instrument family.

    `nautilus_instrument_id` is present only when the instrument has an exact runtime
    mapping already accepted by Trader 2.0. Provider coverage does not depend on
    Nautilus supporting every reference-data object.
    """

    family: InstrumentFamily
    identity: TInvestInstrumentIdentity
    name: str
    currency: str | None
    lot: int | None
    min_price_increment: Decimal | None
    api_trade_available: bool
    buy_available: bool | None
    sell_available: bool | None
    short_enabled: bool | None
    exchange: str | None
    isin: str | None
    country_of_risk: str | None
    asset_uid: str | None
    basic_asset_uid: str | None
    first_trade_ns: int | None
    expiration_ns: int | None
    nautilus_instrument_id: InstrumentId | None


RuntimeInstrument = Equity | FuturesContract


def _quotation(value: Any, *, field: str) -> Decimal:
    if not isinstance(value, dict):
        raise InstrumentMappingError(f"{field} is missing")
    try:
        units = Decimal(str(value.get("units", "0")))
        nano = Decimal(str(value.get("nano", 0))) / Decimal("1000000000")
    except Exception as exc:
        raise InstrumentMappingError(f"{field} is invalid") from exc
    return units + nano


def _optional_quotation(value: Any) -> Decimal | None:
    if value is None:
        return None
    try:
        result = _quotation(value, field="quotation")
    except InstrumentMappingError:
        return None
    return result


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


def _optional_iso_ns(value: Any) -> int | None:
    if not isinstance(value, str) or not value:
        return None
    try:
        return _iso_ns(value, field="timestamp")
    except InstrumentMappingError:
        return None


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
    """Complete T-Invest instrument/reference catalogue with exact runtime mappings where qualified."""

    _LIST_ENDPOINTS: tuple[tuple[InstrumentFamily, str, bool], ...] = (
        (InstrumentFamily.SHARE, SHARES_PATH, True),
        (InstrumentFamily.BOND, BONDS_PATH, True),
        (InstrumentFamily.ETF, ETFS_PATH, True),
        (InstrumentFamily.CURRENCY, CURRENCIES_PATH, True),
        (InstrumentFamily.FUTURE, FUTURES_PATH, True),
        (InstrumentFamily.STRUCTURED_NOTE, STRUCTURED_NOTES_PATH, True),
        (InstrumentFamily.DFA, DFAS_PATH, False),
        (InstrumentFamily.INDICATIVE, INDICATIVES_PATH, False),
    )

    def __init__(self, transport: TInvestHttpTransport) -> None:
        self._transport = transport
        self._records_by_uid: dict[str, TInvestInstrumentRecord] = {}
        self._runtime_by_uid: dict[str, RuntimeInstrument] = {}
        self._runtime_by_id: dict[InstrumentId, RuntimeInstrument] = {}

    async def load(self) -> None:
        common = {
            "instrumentStatus": "INSTRUMENT_STATUS_BASE",
            "instrumentExchange": "INSTRUMENT_EXCHANGE_UNSPECIFIED",
        }
        records: list[TInvestInstrumentRecord] = []
        runtimes: list[RuntimeInstrument] = []

        for family, path, uses_common_filter in self._LIST_ENDPOINTS:
            payload = common if uses_common_filter else {}
            response = await self._transport.post(path, dict(payload))
            for raw in response.get("instruments") or []:
                if not isinstance(raw, dict):
                    raise InstrumentMappingError(f"{family.value} catalogue returned non-object instrument")
                runtime: RuntimeInstrument | None = None
                if family is InstrumentFamily.SHARE and raw.get("apiTradeAvailableFlag") is True:
                    runtime = self._map_share(raw)
                elif family is InstrumentFamily.FUTURE and raw.get("apiTradeAvailableFlag") is True:
                    margin = await self._transport.post(
                        FUTURES_MARGIN_PATH,
                        {"instrumentId": self._require_text(raw, "uid")},
                    )
                    runtime = self._map_future(raw, margin)
                record = self._map_record(family, raw, runtime)
                records.append(record)
                if runtime is not None:
                    runtimes.append(runtime)

        self._records_by_uid = {record.identity.uid: record for record in records}
        self._runtime_by_uid = {
            str(item.info["tinvest"]["instrument_uid"]): item for item in runtimes
        }
        self._runtime_by_id = {item.id: item for item in runtimes}

    def all_records(self) -> tuple[TInvestInstrumentRecord, ...]:
        return tuple(self._records_by_uid.values())

    def records_by_family(self, family: InstrumentFamily) -> tuple[TInvestInstrumentRecord, ...]:
        return tuple(record for record in self._records_by_uid.values() if record.family is family)

    def record_by_uid(self, uid: str) -> TInvestInstrumentRecord | None:
        return self._records_by_uid.get(uid)

    def all(self) -> tuple[RuntimeInstrument, ...]:
        """Return exact Nautilus runtime mappings currently accepted by ADR/qualification."""
        return tuple(self._runtime_by_id.values())

    def by_uid(self, uid: str) -> RuntimeInstrument | None:
        return self._runtime_by_uid.get(uid)

    def by_id(self, instrument_id: InstrumentId) -> RuntimeInstrument | None:
        return self._runtime_by_id.get(instrument_id)

    async def find(
        self,
        query: str,
        *,
        instrument_kind: str = "INSTRUMENT_TYPE_UNSPECIFIED",
        api_trade_available_only: bool = False,
    ) -> tuple[TInvestInstrumentRecord, ...]:
        if not query.strip():
            raise ValueError("query must not be empty")
        response = await self._transport.post(
            FIND_INSTRUMENT_PATH,
            {
                "query": query.strip(),
                "instrumentKind": instrument_kind,
                "apiTradeAvailableFlag": api_trade_available_only,
            },
        )
        result: list[TInvestInstrumentRecord] = []
        for raw in response.get("instruments") or []:
            if not isinstance(raw, dict):
                continue
            result.append(self._map_record(self._infer_family(raw), raw, None))
        return tuple(result)

    async def options_by(
        self,
        *,
        basic_asset_uid: str | None = None,
        basic_asset_position_uid: str | None = None,
        basic_instrument_id: str | None = None,
    ) -> tuple[TInvestInstrumentRecord, ...]:
        payload: dict[str, Any] = {}
        if basic_asset_uid:
            payload["basicAssetUid"] = basic_asset_uid
        if basic_asset_position_uid:
            payload["basicAssetPositionUid"] = basic_asset_position_uid
        if basic_instrument_id:
            payload["basicInstrumentId"] = basic_instrument_id
        if not payload:
            raise ValueError("OptionsBy requires a basic asset/instrument identifier")
        response = await self._transport.post(OPTIONS_BY_PATH, payload)
        options = response.get("instruments") or response.get("options") or []
        result = tuple(
            self._map_record(InstrumentFamily.OPTION, raw, None)
            for raw in options
            if isinstance(raw, dict)
        )
        return result

    @staticmethod
    def _require_text(raw: dict[str, Any], field: str) -> str:
        value = raw.get(field)
        if not isinstance(value, str) or not value.strip():
            raise InstrumentMappingError(f"{field} is missing")
        return value.strip()

    @staticmethod
    def _optional_text(raw: dict[str, Any], field: str) -> str | None:
        value = raw.get(field)
        return value.strip() if isinstance(value, str) and value.strip() else None

    @staticmethod
    def _optional_int(raw: dict[str, Any], field: str) -> int | None:
        value = raw.get(field)
        if value is None:
            return None
        try:
            return int(value)
        except (TypeError, ValueError):
            return None

    def _map_record(
        self,
        family: InstrumentFamily,
        raw: dict[str, Any],
        runtime: RuntimeInstrument | None,
    ) -> TInvestInstrumentRecord:
        uid = self._require_text(raw, "uid")
        ticker = self._optional_text(raw, "ticker") or uid
        class_code = self._optional_text(raw, "classCode") or ""
        return TInvestInstrumentRecord(
            family=family,
            identity=TInvestInstrumentIdentity(
                ticker=ticker,
                class_code=class_code,
                uid=uid,
                figi=self._optional_text(raw, "figi"),
                position_uid=self._optional_text(raw, "positionUid"),
            ),
            name=self._optional_text(raw, "name") or ticker,
            currency=self._optional_text(raw, "currency"),
            lot=self._optional_int(raw, "lot"),
            min_price_increment=_optional_quotation(raw.get("minPriceIncrement")),
            api_trade_available=bool(raw.get("apiTradeAvailableFlag", False)),
            buy_available=raw.get("buyAvailableFlag") if isinstance(raw.get("buyAvailableFlag"), bool) else None,
            sell_available=raw.get("sellAvailableFlag") if isinstance(raw.get("sellAvailableFlag"), bool) else None,
            short_enabled=raw.get("shortEnabledFlag") if isinstance(raw.get("shortEnabledFlag"), bool) else None,
            exchange=self._optional_text(raw, "exchange"),
            isin=self._optional_text(raw, "isin"),
            country_of_risk=self._optional_text(raw, "countryOfRisk"),
            asset_uid=self._optional_text(raw, "assetUid"),
            basic_asset_uid=(
                self._optional_text(raw, "basicAssetPositionUid")
                or self._optional_text(raw, "basicAssetUid")
            ),
            first_trade_ns=_optional_iso_ns(raw.get("firstTradeDate")),
            expiration_ns=_optional_iso_ns(raw.get("expirationDate") or raw.get("maturityDate")),
            nautilus_instrument_id=runtime.id if runtime is not None else None,
        )

    @staticmethod
    def _infer_family(raw: dict[str, Any]) -> InstrumentFamily:
        raw_type = str(raw.get("instrumentType") or raw.get("instrumentKind") or raw.get("type") or "").upper()
        mapping = {
            "BOND": InstrumentFamily.BOND,
            "SHARE": InstrumentFamily.SHARE,
            "CURRENCY": InstrumentFamily.CURRENCY,
            "ETF": InstrumentFamily.ETF,
            "FUTURES": InstrumentFamily.FUTURE,
            "FUTURE": InstrumentFamily.FUTURE,
            "SP": InstrumentFamily.STRUCTURED_NOTE,
            "OPTION": InstrumentFamily.OPTION,
            "INDEX": InstrumentFamily.INDICATIVE,
            "COMMODITY": InstrumentFamily.INDICATIVE,
            "DFA": InstrumentFamily.DFA,
        }
        for suffix, family in mapping.items():
            if raw_type.endswith(suffix):
                return family
        return InstrumentFamily.INDICATIVE

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
