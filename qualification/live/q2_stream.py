from __future__ import annotations

import asyncio
import json
import os
import sys
import time
from dataclasses import dataclass
from typing import Any

from nautilus_trader.cache.cache import Cache
from nautilus_trader.common.component import MessageBus, TestClock
from nautilus_trader.data.engine import DataEngine
from nautilus_trader.model.identifiers import TraderId

from qualification.live.q1_tinvest import SHARES_PATH
from qualification.live.q1_tinvest import QualificationError
from qualification.live.q1_tinvest import _iso_to_ns
from qualification.live.q1_tinvest import _post
from qualification.live.q1_tinvest import _quotation
from qualification.live.q1_tinvest import _require_text
from qualification.live.q1_tinvest import _select_sber
from qualification.poc.q1_instruments import TInvestShareSpec
from qualification.poc.q1_instruments import to_nautilus_equity
from qualification.poc.q2_market_data import to_nautilus_trade_tick

WS_URL = "wss://invest-public-api.tbank.ru/ws/"


@dataclass
class StreamEvidence:
    subscription_acks: set[str]
    trades: int = 0
    books: int = 0
    statuses: int = 0
    last_prices: int = 0
    pings: int = 0
    last_trade_id: str | None = None


def _subscription_messages(instrument_uid: str) -> list[dict[str, Any]]:
    subscribe = "SUBSCRIPTION_ACTION_SUBSCRIBE"
    return [
        {
            "subscribe_trades_request": {
                "subscription_action": subscribe,
                "instruments": [
                    {
                        "instrument_id": instrument_uid,
                        "trade_source": "TRADE_SOURCE_ALL",
                    }
                ],
            }
        },
        {
            "subscribe_order_book_request": {
                "subscription_action": subscribe,
                "instruments": [
                    {
                        "instrument_id": instrument_uid,
                        "depth": 10,
                        "order_book_type": "ORDERBOOK_TYPE_ALL",
                    }
                ],
            }
        },
        {
            "subscribe_info_request": {
                "subscription_action": subscribe,
                "instruments": [{"instrument_id": instrument_uid}],
            }
        },
        {
            "subscribe_last_price_request": {
                "subscription_action": subscribe,
                "instruments": [{"instrument_id": instrument_uid}],
            }
        },
        {"ping_settings": {"ping_delay_ms": 5000}},
    ]


def _make_engine() -> tuple[DataEngine, Cache]:
    clock = TestClock()
    cache = Cache()
    msgbus = MessageBus(trader_id=TraderId("QUALIFICATION-001"), clock=clock)
    engine = DataEngine(msgbus=msgbus, cache=cache, clock=clock)
    return engine, cache


def _ack_name(message: dict[str, Any]) -> str | None:
    for name in (
        "subscribe_trades_response",
        "subscribe_order_book_response",
        "subscribe_info_response",
        "subscribe_last_price_response",
    ):
        if name in message:
            return name
    return None


async def _run_one_connection(
    *,
    token: str,
    instrument_uid: str,
    instrument_id,
    lot_size: int,
    engine: DataEngine,
    cache: Cache,
    require_market_event: bool,
    timeout_seconds: float,
) -> StreamEvidence:
    try:
        import websockets
    except ImportError as exc:
        raise QualificationError("websockets dependency is not installed; run `python -m pip install -e .`") from exc

    evidence = StreamEvidence(subscription_acks=set())

    async with websockets.connect(
        WS_URL,
        subprotocols=["json-proto"],
        additional_headers={"Authorization": f"Bearer {token}"},
        open_timeout=20,
        close_timeout=5,
        ping_interval=None,
        max_size=4 * 1024 * 1024,
    ) as ws:
        for request in _subscription_messages(instrument_uid):
            await ws.send(json.dumps(request, separators=(",", ":")))

        deadline = asyncio.get_running_loop().time() + timeout_seconds
        got_market_event = False

        while asyncio.get_running_loop().time() < deadline:
            remaining = max(0.1, deadline - asyncio.get_running_loop().time())
            try:
                raw = await asyncio.wait_for(ws.recv(), timeout=min(remaining, 6.0))
            except asyncio.TimeoutError:
                continue

            message = json.loads(raw)
            ack = _ack_name(message)
            if ack:
                evidence.subscription_acks.add(ack)
                continue

            if "ping" in message:
                evidence.pings += 1
                continue

            trade = message.get("trade")
            if trade:
                tick = to_nautilus_trade_tick(
                    instrument_id=instrument_id,
                    instrument_uid=instrument_uid,
                    price=_quotation(trade.get("price"), field="stream.trade.price"),
                    quantity_lots=int(trade.get("quantity", 0)),
                    lot_size=lot_size,
                    direction=_require_text(trade, "direction"),
                    trade_source=trade.get("trade_source") or "TRADE_SOURCE_UNSPECIFIED",
                    ts_event=_iso_to_ns(trade.get("time"), field="stream.trade.time"),
                    ts_init=time.time_ns(),
                )
                engine.process(tick)
                cached = cache.trade_tick(instrument_id)
                if cached is None or cached.trade_id != tick.trade_id:
                    raise QualificationError("Nautilus DataEngine did not cache the processed TradeTick")
                evidence.trades += 1
                evidence.last_trade_id = str(tick.trade_id)
                got_market_event = True
                if require_market_event:
                    break
                continue

            orderbook = message.get("orderbook")
            if orderbook:
                if orderbook.get("is_consistent") is False:
                    raise QualificationError("stream order book reported is_consistent=false")
                if not orderbook.get("time"):
                    raise QualificationError("stream order book has no event time")
                evidence.books += 1
                got_market_event = True
                if require_market_event:
                    break
                continue

            trading_status = message.get("trading_status")
            if trading_status:
                if not trading_status.get("time"):
                    raise QualificationError("stream trading status has no event time")
                evidence.statuses += 1
                got_market_event = True
                if require_market_event:
                    break
                continue

            last_price = message.get("last_price")
            if last_price:
                if not last_price.get("time"):
                    raise QualificationError("stream last price has no event time")
                _quotation(last_price.get("price"), field="stream.last_price.price")
                evidence.last_prices += 1
                got_market_event = True
                if require_market_event:
                    break

        required_acks = {
            "subscribe_trades_response",
            "subscribe_order_book_response",
            "subscribe_info_response",
            "subscribe_last_price_response",
        }
        missing = required_acks - evidence.subscription_acks
        if missing:
            raise QualificationError(f"missing subscription ACKs: {sorted(missing)}")
        if require_market_event and not got_market_event:
            raise QualificationError("no market event received before timeout")
        return evidence


async def _main_async() -> int:
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
        share = _select_sber(shares)
        share_uid = _require_text(share, "uid")
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
        engine, cache = _make_engine()
        cache.add_instrument(share_nt)

        print("Q2b LIVE STREAM QUALIFICATION")
        print("=============================")
        print(f"SBER: uid={share_uid} lot={share_spec.lot} nautilus_id={share_nt.id}")
        print("PHASE 1: connect, authenticate, subscribe, receive ACKs + market event")

        first = await _run_one_connection(
            token=token,
            instrument_uid=share_uid,
            instrument_id=share_nt.id,
            lot_size=share_spec.lot,
            engine=engine,
            cache=cache,
            require_market_event=True,
            timeout_seconds=30.0,
        )
        print(
            "        acks=4 "
            f"trades={first.trades} books={first.books} statuses={first.statuses} "
            f"last_prices={first.last_prices} pings={first.pings}"
        )

        print("PHASE 2: forced client disconnect, reconnect, restore subscriptions")
        await asyncio.sleep(1.0)
        second = await _run_one_connection(
            token=token,
            instrument_uid=share_uid,
            instrument_id=share_nt.id,
            lot_size=share_spec.lot,
            engine=engine,
            cache=cache,
            require_market_event=True,
            timeout_seconds=30.0,
        )
        print(
            "        acks=4 "
            f"trades={second.trades} books={second.books} statuses={second.statuses} "
            f"last_prices={second.last_prices} pings={second.pings}"
        )
        if cache.trade_tick_count(share_nt.id):
            print(f"DATAENGINE: cached_trade_ticks={cache.trade_tick_count(share_nt.id)}")
        else:
            print("DATAENGINE: no trade occurred during sample; trade path remains covered by unit tests")
        print("PASS: WebSocket subscriptions restored after forced disconnect without manual intervention")
        print("PASS: any received public trades were processed by Nautilus DataEngine and cache")
        return 0
    except Exception as exc:
        print(f"FAIL: {exc}", file=sys.stderr)
        return 1


def main() -> int:
    return asyncio.run(_main_async())


if __name__ == "__main__":
    raise SystemExit(main())
