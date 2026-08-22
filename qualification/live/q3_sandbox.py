from __future__ import annotations

import argparse
import os
import sys
import uuid
from decimal import Decimal, ROUND_DOWN
from typing import Any

from qualification.live.q1_tinvest import SHARES_PATH
from qualification.live.q1_tinvest import QualificationError
from qualification.live.q1_tinvest import _post
from qualification.live.q1_tinvest import _quotation
from qualification.live.q1_tinvest import _require_text
from qualification.live.q1_tinvest import _select_sber

OPEN_ACCOUNT = "/tinkoff.public.invest.api.contract.v1.SandboxService/OpenSandboxAccount"
PAY_IN = "/tinkoff.public.invest.api.contract.v1.SandboxService/SandboxPayIn"
POST_ORDER = "/tinkoff.public.invest.api.contract.v1.SandboxService/PostSandboxOrder"
GET_ORDER_STATE = "/tinkoff.public.invest.api.contract.v1.SandboxService/GetSandboxOrderState"
GET_ORDERS = "/tinkoff.public.invest.api.contract.v1.SandboxService/GetSandboxOrders"
REPLACE_ORDER = "/tinkoff.public.invest.api.contract.v1.SandboxService/ReplaceSandboxOrder"
CANCEL_ORDER = "/tinkoff.public.invest.api.contract.v1.SandboxService/CancelSandboxOrder"
GET_PORTFOLIO = "/tinkoff.public.invest.api.contract.v1.SandboxService/GetSandboxPortfolio"
GET_POSITIONS = "/tinkoff.public.invest.api.contract.v1.SandboxService/GetSandboxPositions"
GET_LAST_PRICES = "/tinkoff.public.invest.api.contract.v1.MarketDataService/GetLastPrices"


def _quotation_payload(value: Decimal) -> dict[str, Any]:
    sign = -1 if value < 0 else 1
    value = abs(value)
    units = int(value)
    nano = int((value - Decimal(units)) * Decimal("1000000000"))
    return {"units": str(units * sign), "nano": nano * sign}


def _round_to_tick(value: Decimal, tick: Decimal) -> Decimal:
    if tick <= 0:
        raise QualificationError("invalid tick")
    steps = (value / tick).to_integral_value(rounding=ROUND_DOWN)
    return steps * tick


def _new_request_id() -> str:
    return str(uuid.uuid4())


def _require_execute(args: argparse.Namespace) -> None:
    if not args.execute:
        raise QualificationError("mutation disabled: rerun with --execute to use T-Invest Sandbox")


def _sandbox_post(token: str, path: str, payload: dict[str, Any]) -> dict[str, Any]:
    # SandboxService methods are intentionally used. The runner never calls production OrdersService mutations.
    return _post(token, path, payload)


def _bootstrap(token: str) -> str:
    account = _sandbox_post(token, OPEN_ACCOUNT, {"name": "Trader 2 qualification"})
    account_id = _require_text(account, "accountId")
    pay_in = _sandbox_post(
        token,
        PAY_IN,
        {
            "accountId": account_id,
            "amount": {"currency": "rub", "units": "100000", "nano": 0},
        },
    )
    print(f"SANDBOX ACCOUNT: {account_id}")
    print(f"PAY IN: {pay_in.get('balance')}")
    print("Persist this only as qualification state; it is not a real brokerage account.")
    return account_id


def _resolve_sber(token: str) -> tuple[dict[str, Any], str, Decimal]:
    shares = _post(
        token,
        SHARES_PATH,
        {
            "instrumentStatus": "INSTRUMENT_STATUS_BASE",
            "instrumentExchange": "INSTRUMENT_EXCHANGE_UNSPECIFIED",
        },
    ).get("instruments", [])
    share = _select_sber(shares)
    uid = _require_text(share, "uid")
    tick = _quotation(share.get("minPriceIncrement"), field="share.minPriceIncrement")
    return share, uid, tick


def _last_price(token: str, instrument_uid: str) -> Decimal:
    response = _post(token, GET_LAST_PRICES, {"instrumentId": [instrument_uid]})
    prices = response.get("lastPrices") or []
    if not prices:
        raise QualificationError("GetLastPrices returned no SBER price")
    return _quotation(prices[0].get("price"), field="last_price.price")


def _state(token: str, account_id: str, order_id: str) -> dict[str, Any]:
    return _sandbox_post(
        token,
        GET_ORDER_STATE,
        {
            "accountId": account_id,
            "orderId": order_id,
            "orderIdType": "ORDER_ID_TYPE_EXCHANGE",
        },
    )


def _snapshot(token: str, account_id: str) -> None:
    orders = _sandbox_post(token, GET_ORDERS, {"accountId": account_id})
    portfolio = _sandbox_post(token, GET_PORTFOLIO, {"accountId": account_id})
    positions = _sandbox_post(token, GET_POSITIONS, {"accountId": account_id})
    print(f"BROKER SNAPSHOT: active_orders={len(orders.get('orders') or [])}")
    print(f"PORTFOLIO positions={len(portfolio.get('positions') or [])}")
    print(f"POSITIONS securities={len(positions.get('securities') or [])} futures={len(positions.get('futures') or [])}")


def _market_fill_cycle(token: str, account_id: str, instrument_uid: str) -> None:
    buy_request_id = _new_request_id()
    print(f"MARKET BUY request_id={buy_request_id}")
    buy = _sandbox_post(
        token,
        POST_ORDER,
        {
            "quantity": "1",
            "direction": "ORDER_DIRECTION_BUY",
            "accountId": account_id,
            "orderType": "ORDER_TYPE_MARKET",
            "orderId": buy_request_id,
            "instrumentId": instrument_uid,
        },
    )
    buy_order_id = _require_text(buy, "orderId")
    print(
        f"  broker_order_id={buy_order_id} request_id={buy.get('orderRequestId')} "
        f"status={buy.get('executionReportStatus')} lots_executed={buy.get('lotsExecuted')}"
    )
    authoritative_buy = _state(token, account_id, buy_order_id)
    print(
        f"  authoritative status={authoritative_buy.get('executionReportStatus')} "
        f"lots_executed={authoritative_buy.get('lotsExecuted')}"
    )
    _snapshot(token, account_id)

    sell_request_id = _new_request_id()
    print(f"MARKET SELL flatten request_id={sell_request_id}")
    sell = _sandbox_post(
        token,
        POST_ORDER,
        {
            "quantity": "1",
            "direction": "ORDER_DIRECTION_SELL",
            "accountId": account_id,
            "orderType": "ORDER_TYPE_MARKET",
            "orderId": sell_request_id,
            "instrumentId": instrument_uid,
        },
    )
    sell_order_id = _require_text(sell, "orderId")
    authoritative_sell = _state(token, account_id, sell_order_id)
    print(
        f"  broker_order_id={sell_order_id} status={authoritative_sell.get('executionReportStatus')} "
        f"lots_executed={authoritative_sell.get('lotsExecuted')}"
    )
    _snapshot(token, account_id)


def _replace_cancel_cycle(token: str, account_id: str, instrument_uid: str, tick: Decimal) -> None:
    last = _last_price(token, instrument_uid)
    initial_price = _round_to_tick(last * Decimal("0.80"), tick)
    replacement_price = _round_to_tick(initial_price + tick, tick)
    request_id = _new_request_id()
    print(f"LIMIT BUY request_id={request_id} last={last} price={initial_price}")
    order = _sandbox_post(
        token,
        POST_ORDER,
        {
            "quantity": "1",
            "price": _quotation_payload(initial_price),
            "direction": "ORDER_DIRECTION_BUY",
            "accountId": account_id,
            "orderType": "ORDER_TYPE_LIMIT",
            "orderId": request_id,
            "instrumentId": instrument_uid,
            "timeInForce": "TIME_IN_FORCE_DAY",
            "priceType": "PRICE_TYPE_CURRENCY",
        },
    )
    broker_order_id = _require_text(order, "orderId")
    print(f"  broker_order_id={broker_order_id} status={order.get('executionReportStatus')}")

    replace_request_id = _new_request_id()
    print(f"REPLACE request_id={replace_request_id} new_price={replacement_price}")
    replaced = _sandbox_post(
        token,
        REPLACE_ORDER,
        {
            "accountId": account_id,
            "orderIdType": "ORDER_ID_TYPE_EXCHANGE",
            "orderId": broker_order_id,
            "idempotencyKey": replace_request_id,
            "quantity": "1",
            "price": _quotation_payload(replacement_price),
            "priceType": "PRICE_TYPE_CURRENCY",
        },
    )
    replacement_order_id = _require_text(replaced, "orderId")
    print(
        f"  replacement_broker_order_id={replacement_order_id} "
        f"request_id={replaced.get('orderRequestId')}"
    )

    cancel = _sandbox_post(
        token,
        CANCEL_ORDER,
        {
            "accountId": account_id,
            "orderId": replacement_order_id,
            "orderIdType": "ORDER_ID_TYPE_EXCHANGE",
        },
    )
    print(f"CANCEL broker_order_id={replacement_order_id} time={cancel.get('time')}")
    final_state = _state(token, account_id, replacement_order_id)
    print(
        f"  authoritative final_status={final_state.get('executionReportStatus')} "
        f"lots_executed={final_state.get('lotsExecuted')}"
    )
    _snapshot(token, account_id)


def main() -> int:
    parser = argparse.ArgumentParser(description="Q3 T-Invest Sandbox execution qualification")
    parser.add_argument("--execute", action="store_true", help="allow SandboxService mutations")
    parser.add_argument("--bootstrap", action="store_true", help="create and fund a fresh sandbox account")
    parser.add_argument("--account", help="existing sandbox account ID")
    parser.add_argument("--market-cycle", action="store_true", help="buy 1 lot SBER then flatten it")
    parser.add_argument("--replace-cancel", action="store_true", help="submit, replace and cancel a resting SBER order")
    parser.add_argument("--snapshot", action="store_true", help="read broker-authoritative sandbox state")
    args = parser.parse_args()

    token = os.environ.get("TINVEST_TOKEN")
    if not token:
        print("FAIL: TINVEST_TOKEN is not set", file=sys.stderr)
        return 2

    try:
        if args.bootstrap or args.market_cycle or args.replace_cancel:
            _require_execute(args)

        account_id = args.account
        if args.bootstrap:
            account_id = _bootstrap(token)

        if not account_id:
            raise QualificationError("provide --account ACCOUNT_ID or use --bootstrap")

        share, instrument_uid, tick = _resolve_sber(token)
        print(
            f"Q3 SANDBOX EXECUTION QUALIFICATION\n"
            f"SBER {_require_text(share, 'ticker')}/{_require_text(share, 'classCode')} "
            f"uid={instrument_uid} tick={tick} account={account_id}"
        )

        if args.snapshot:
            _snapshot(token, account_id)
        if args.market_cycle:
            _market_fill_cycle(token, account_id, instrument_uid)
        if args.replace_cancel:
            _replace_cancel_cycle(token, account_id, instrument_uid, tick)

        if not (args.snapshot or args.market_cycle or args.replace_cancel or args.bootstrap):
            print("No action selected. Use --snapshot, --market-cycle or --replace-cancel.")
        return 0
    except Exception as exc:
        print(f"FAIL: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
