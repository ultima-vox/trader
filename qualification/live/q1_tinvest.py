from __future__ import annotations

import json
import os
import sys
import urllib.error
import urllib.request
from calendar import timegm
from datetime import datetime, timezone
from decimal import Decimal
from typing import Any

from nautilus_trader.model.enums import AssetClass

from qualification.poc.q1_instruments import MappingError
from qualification.poc.q1_instruments import TInvestFutureSpec
from qualification.poc.q1_instruments import TInvestShareSpec
from qualification.poc.q1_instruments import future_money_per_point
from qualification.poc.q1_instruments import to_nautilus_equity
from qualification.poc.q1_instruments import to_nautilus_future

BASE_URL = "https://invest-public-api.tbank.ru/rest"
SHARES_PATH = "/tinkoff.public.invest.api.contract.v1.InstrumentsService/Shares"
FUTURES_PATH = "/tinkoff.public.invest.api.contract.v1.InstrumentsService/Futures"
FUTURES_MARGIN_PATH = "/tinkoff.public.invest.api.contract.v1.InstrumentsService/GetFuturesMargin"


class QualificationError(RuntimeError):
    pass


def _post(token: str, path: str, payload: dict[str, Any]) -> dict[str, Any]:
    body = json.dumps(payload).encode("utf-8")
    request = urllib.request.Request(
        BASE_URL + path,
        data=body,
        method="POST",
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
            "Accept": "application/json",
            "User-Agent": "trader2-nautilus-qualification/0.0.1",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        payload_text = exc.read().decode("utf-8", errors="replace")
        raise QualificationError(f"T-Invest HTTP {exc.code} for {path}: {payload_text}") from exc
    except urllib.error.URLError as exc:
        raise QualificationError(f"T-Invest connection failed for {path}: {exc}") from exc


def _quotation(value: dict[str, Any] | None, *, field: str) -> Decimal:
    if value is None:
        raise QualificationError(f"missing quotation: {field}")
    units = Decimal(str(value.get("units", "0")))
    nano = Decimal(str(value.get("nano", 0)))
    return units + nano / Decimal("1000000000")


def _iso_to_ns(value: str | None, *, field: str) -> int:
    if not value:
        raise QualificationError(f"missing timestamp: {field}")
    dt = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    dt = dt.astimezone(timezone.utc)
    return timegm(dt.utctimetuple()) * 1_000_000_000 + dt.microsecond * 1_000


def _future_asset_class(asset_type: str) -> AssetClass:
    normalized = asset_type.strip().upper()
    if normalized.startswith("TYPE_"):
        normalized = normalized.removeprefix("TYPE_")

    mapping = {
        "SECURITY": AssetClass.EQUITY,
        "INDEX": AssetClass.INDEX,
        "CURRENCY": AssetClass.FX,
        "COMMODITY": AssetClass.COMMODITY,
    }

    try:
        return mapping[normalized]
    except KeyError as exc:
        raise QualificationError(f"unsupported T-Invest future asset_type={asset_type!r}") from exc


def _select_sber(instruments: list[dict[str, Any]]) -> dict[str, Any]:
    candidates = [
        item
        for item in instruments
        if item.get("ticker") == "SBER"
        and item.get("classCode") == "TQBR"
        and item.get("apiTradeAvailableFlag") is True
    ]
    if len(candidates) != 1:
        raise QualificationError(f"expected exactly one tradeable SBER/TQBR, found {len(candidates)}")
    return candidates[0]


def _select_future(instruments: list[dict[str, Any]]) -> dict[str, Any]:
    now = datetime.now(timezone.utc)
    candidates: list[tuple[datetime, dict[str, Any]]] = []
    for item in instruments:
        if item.get("classCode") != "SPBFUT" or item.get("apiTradeAvailableFlag") is not True:
            continue
        expiration = item.get("expirationDate")
        first_trade = item.get("firstTradeDate")
        if not expiration or not first_trade:
            continue
        expiration_dt = datetime.fromisoformat(expiration.replace("Z", "+00:00"))
        first_trade_dt = datetime.fromisoformat(first_trade.replace("Z", "+00:00"))
        if first_trade_dt <= now < expiration_dt:
            candidates.append((expiration_dt, item))
    if not candidates:
        raise QualificationError("no active API-tradeable SPBFUT future found")
    candidates.sort(key=lambda pair: (pair[0], pair[1].get("ticker", "")))
    return candidates[0][1]


def _require_text(item: dict[str, Any], key: str) -> str:
    value = item.get(key)
    if not isinstance(value, str) or not value:
        raise QualificationError(f"missing/invalid {key!r} on {item.get('ticker', '<unknown>')}")
    return value


def main() -> int:
    token = os.environ.get("TINVEST_TOKEN")
    if not token:
        print("FAIL: TINVEST_TOKEN is not set", file=sys.stderr)
        return 2

    try:
        shares = _post(
            token,
            SHARES_PATH,
            {
                "instrumentStatus": "INSTRUMENT_STATUS_BASE",
                "instrumentExchange": "INSTRUMENT_EXCHANGE_UNSPECIFIED",
            },
        ).get("instruments", [])
        futures = _post(
            token,
            FUTURES_PATH,
            {
                "instrumentStatus": "INSTRUMENT_STATUS_BASE",
                "instrumentExchange": "INSTRUMENT_EXCHANGE_UNSPECIFIED",
            },
        ).get("instruments", [])

        share = _select_sber(shares)
        future = _select_future(futures)
        future_uid = _require_text(future, "uid")
        margin = _post(token, FUTURES_MARGIN_PATH, {"instrumentId": future_uid})

        catalog_tick = _quotation(future.get("minPriceIncrement"), field="future.minPriceIncrement")
        margin_tick = _quotation(margin.get("minPriceIncrement"), field="margin.minPriceIncrement")
        tick_amount = _quotation(
            margin.get("minPriceIncrementAmount"),
            field="margin.minPriceIncrementAmount",
        )
        if catalog_tick != margin_tick:
            raise QualificationError(
                f"catalog/margin tick mismatch for {future.get('ticker')}: "
                f"catalog={catalog_tick} margin={margin_tick}; refusing to guess"
            )

        share_spec = TInvestShareSpec(
            ticker=_require_text(share, "ticker"),
            class_code=_require_text(share, "classCode"),
            instrument_uid=_require_text(share, "uid"),
            figi=share.get("figi") or None,
            currency=_require_text(share, "currency"),
            lot=int(share.get("lot", 0)),
            min_price_increment=_quotation(share.get("minPriceIncrement"), field="share.minPriceIncrement"),
            api_trade_available=share.get("apiTradeAvailableFlag") is True,
        )
        future_spec = TInvestFutureSpec(
            ticker=_require_text(future, "ticker"),
            class_code=_require_text(future, "classCode"),
            instrument_uid=future_uid,
            figi=future.get("figi") or None,
            currency=_require_text(future, "currency"),
            lot=int(future.get("lot", 0)),
            min_price_increment=margin_tick,
            min_price_increment_amount=tick_amount,
            underlying=_require_text(future, "basicAsset"),
            asset_class=_future_asset_class(_require_text(future, "assetType")),
            activation_ns=_iso_to_ns(future.get("firstTradeDate"), field="future.firstTradeDate"),
            expiration_ns=_iso_to_ns(future.get("expirationDate"), field="future.expirationDate"),
            api_trade_available=future.get("apiTradeAvailableFlag") is True,
            exchange_mic=None,
        )

        share_nt = to_nautilus_equity(share_spec)
        future_nt = to_nautilus_future(future_spec)

        initial_margin_buy = margin.get("initialMarginOnBuy") or {}
        initial_margin_sell = margin.get("initialMarginOnSell") or {}

        print("Q1 LIVE QUALIFICATION")
        print("=====================")
        print(f"SBER:   {share_spec.ticker}/{share_spec.class_code} uid={share_spec.instrument_uid}")
        print(
            f"        lot={share_spec.lot} tick={share_spec.min_price_increment} "
            f"nautilus_id={share_nt.id}"
        )
        print(
            f"FUTURE: {future_spec.ticker}/{future_spec.class_code} uid={future_spec.instrument_uid} "
            f"asset_type={future.get('assetType')}"
        )
        print(
            f"        tick={future_spec.min_price_increment} "
            f"tick_amount={future_spec.min_price_increment_amount} "
            f"money_per_point={future_money_per_point(future_spec)}"
        )
        print(
            f"        lot={future_spec.lot} multiplier={future_nt.multiplier} "
            f"nautilus_id={future_nt.id}"
        )
        print(
            "        initial_margin_buy="
            f"{initial_margin_buy.get('units', '0')}.{str(initial_margin_buy.get('nano', 0)).zfill(9)} "
            f"initial_margin_sell={initial_margin_sell.get('units', '0')}.{str(initial_margin_sell.get('nano', 0)).zfill(9)}"
        )
        print("PASS: live T-Invest instrument metadata mapped into Nautilus without approximation")
        return 0
    except (QualificationError, MappingError, ValueError, KeyError, TypeError) as exc:
        print(f"FAIL: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
