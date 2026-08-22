from __future__ import annotations

import os
import sys
import time
from datetime import datetime, timedelta, timezone

from nautilus_trader.model.data import BarType

from qualification.live.q1_tinvest import FUTURES_PATH
from qualification.live.q1_tinvest import SHARES_PATH
from qualification.live.q1_tinvest import QualificationError
from qualification.live.q1_tinvest import _iso_to_ns
from qualification.live.q1_tinvest import _post
from qualification.live.q1_tinvest import _quotation
from qualification.live.q1_tinvest import _require_text
from qualification.live.q1_tinvest import _select_future
from qualification.live.q1_tinvest import _select_sber
from qualification.poc.q1_instruments import TInvestShareSpec
from qualification.poc.q1_instruments import to_nautilus_equity
from qualification.poc.q2_market_data import MarketDataMappingError
from qualification.poc.q2_market_data import normalize_book_snapshot
from qualification.poc.q2_market_data import to_nautilus_bar
from qualification.poc.q2_market_data import to_nautilus_trade_tick

GET_ORDER_BOOK_PATH = "/tinkoff.public.invest.api.contract.v1.MarketDataService/GetOrderBook"
GET_CANDLES_PATH = "/tinkoff.public.invest.api.contract.v1.MarketDataService/GetCandles"
GET_LAST_TRADES_PATH = "/tinkoff.public.invest.api.contract.v1.MarketDataService/GetLastTrades"
GET_TRADING_STATUS_PATH = "/tinkoff.public.invest.api.contract.v1.MarketDataService/GetTradingStatus"


def _iso(dt: datetime) -> str:
    return dt.astimezone(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")


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
        share_uid = _require_text(share, "uid")
        future_uid = _require_text(future, "uid")

        share_spec = TInvestShareSpec(
            ticker=_require_text(share, "ticker"),
            class_code=_require_text(share, "classCode"),
            instrument_uid=share_uid,
            figi=share.get("figi") or None,
            currency=_require_text(share, "currency"),
            lot=int(share.get("lot", 0)),
            min_price_increment=_quotation(share.get("minPriceIncrement"), field="share.minPriceIncrement"),
            api_trade_available=share.get("apiTradeAvailableFlag") is True,
        )
        share_nt = to_nautilus_equity(share_spec)

        now = datetime.now(timezone.utc)
        candles_response = _post(
            token,
            GET_CANDLES_PATH,
            {
                "instrumentId": share_uid,
                "from": _iso(now - timedelta(days=10)),
                "to": _iso(now),
                "interval": "CANDLE_INTERVAL_DAY",
                "limit": 20,
            },
        )
        candles = candles_response.get("candles", [])
        complete_candles = [item for item in candles if item.get("isComplete") is True]
        if not complete_candles:
            raise QualificationError("no completed SBER daily candle returned")
        candle = complete_candles[-1]
        bar_type = BarType.from_str(f"{share_nt.id}-1-DAY-LAST-EXTERNAL")
        bar = to_nautilus_bar(
            bar_type=bar_type,
            open_price=_quotation(candle.get("open"), field="candle.open"),
            high_price=_quotation(candle.get("high"), field="candle.high"),
            low_price=_quotation(candle.get("low"), field="candle.low"),
            close_price=_quotation(candle.get("close"), field="candle.close"),
            volume_lots=int(candle.get("volume", 0)),
            lot_size=share_spec.lot,
            ts_event=_iso_to_ns(candle.get("time"), field="candle.time"),
            ts_init=time.time_ns(),
        )

        order_book = _post(token, GET_ORDER_BOOK_PATH, {"instrumentId": share_uid, "depth": 10})
        book_time = order_book.get("time")
        if not book_time:
            raise QualificationError("GetOrderBook returned no event time")
        book = normalize_book_snapshot(
            bids=order_book.get("bids", []),
            asks=order_book.get("asks", []),
            lot_size=share_spec.lot,
            depth=10,
            ts_event=_iso_to_ns(book_time, field="order_book.time"),
            quotation_parser=_quotation,
            is_consistent=order_book.get("isConsistent"),
        )
        if not book.bids and not book.asks:
            raise QualificationError("GetOrderBook returned no levels; refusing to fabricate depth")

        status = _post(token, GET_TRADING_STATUS_PATH, {"instrumentId": share_uid})
        trading_status = status.get("tradingStatus")
        if not trading_status:
            raise QualificationError("GetTradingStatus returned no tradingStatus")

        trades_response = _post(
            token,
            GET_LAST_TRADES_PATH,
            {
                "instrumentId": share_uid,
                "from": _iso(now - timedelta(hours=1)),
                "to": _iso(now),
                "tradeSource": "TRADE_SOURCE_ALL",
            },
        )
        trades = trades_response.get("trades", [])
        trade_tick = None
        if trades:
            trade = trades[-1]
            trade_tick = to_nautilus_trade_tick(
                instrument_id=share_nt.id,
                instrument_uid=share_uid,
                price=_quotation(trade.get("price"), field="trade.price"),
                quantity_lots=int(trade.get("quantity", 0)),
                lot_size=share_spec.lot,
                direction=_require_text(trade, "direction"),
                trade_source=trade.get("tradeSource") or "TRADE_SOURCE_UNSPECIFIED",
                ts_event=_iso_to_ns(trade.get("time"), field="trade.time"),
                ts_init=time.time_ns(),
            )

        print("Q2 LIVE SNAPSHOT QUALIFICATION")
        print("==============================")
        print(f"SBER:   uid={share_uid} lot={share_spec.lot} nautilus_id={share_nt.id}")
        print(
            f"BAR:    time={candle.get('time')} close={bar.close} "
            f"volume_lots={candle.get('volume')} volume_units={bar.volume}"
        )
        print(
            f"BOOK:   bids={len(book.bids)} asks={len(book.asks)} "
            f"consistent={book.is_consistent} time={book_time}"
        )
        if book.bids:
            print(f"        best_bid={book.bids[0].price} qty_units={book.bids[0].quantity_units}")
        if book.asks:
            print(f"        best_ask={book.asks[0].price} qty_units={book.asks[0].quantity_units}")
        print(f"STATUS: {trading_status}")
        if trade_tick is None:
            print("TRADE:  no public trade in the last hour (expected outside an active session); stream test still required")
        else:
            print(
                f"TRADE:  price={trade_tick.price} size_units={trade_tick.size} "
                f"derived_trade_id={trade_tick.trade_id}"
            )
        print(f"FUTURE selected for next stream test: {_require_text(future, 'ticker')} uid={future_uid}")
        print("PASS: unary market-data semantics map without unit/timestamp fabrication")
        print("PENDING: bidirectional/server-side stream -> Nautilus DataEngine + forced reconnect")
        return 0
    except (QualificationError, MarketDataMappingError, ValueError, KeyError, TypeError) as exc:
        print(f"FAIL: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
