from __future__ import annotations

import argparse
import json
import os
import sys
from dataclasses import asdict, dataclass
from decimal import Decimal
from pathlib import Path

from qualification.live.q1_tinvest import QualificationError
from qualification.live.q1_tinvest import _require_text
from qualification.live.q3_sandbox import CANCEL_ORDER
from qualification.live.q3_sandbox import GET_ORDERS
from qualification.live.q3_sandbox import GET_PORTFOLIO
from qualification.live.q3_sandbox import GET_POSITIONS
from qualification.live.q3_sandbox import POST_ORDER
from qualification.live.q3_sandbox import _last_price
from qualification.live.q3_sandbox import _new_request_id
from qualification.live.q3_sandbox import _quotation_payload
from qualification.live.q3_sandbox import _resolve_sber
from qualification.live.q3_sandbox import _round_to_tick
from qualification.live.q3_sandbox import _sandbox_post
from qualification.live.q3_sandbox import _state

DEFAULT_STATE = Path(".qualification-q4-state.json")


class AmbiguousMutationOutcome(RuntimeError):
    """Raised by the harness after a request was dispatched but its response is intentionally hidden."""


@dataclass
class PersistedState:
    account_id: str
    instrument_uid: str
    client_request_id: str
    broker_order_id: str
    expected_kind: str


def _write_state(path: Path, state: PersistedState) -> None:
    path.write_text(json.dumps(asdict(state), indent=2) + "\n", encoding="utf-8")
    try:
        os.chmod(path, 0o600)
    except OSError:
        pass


def _read_state(path: Path) -> PersistedState:
    payload = json.loads(path.read_text(encoding="utf-8"))
    return PersistedState(**payload)


def _resting_limit_price(token: str, instrument_uid: str, tick: Decimal) -> Decimal:
    last = _last_price(token, instrument_uid)
    return _round_to_tick(last * Decimal("0.70"), tick)


def _submit_resting_order(token: str, account_id: str, instrument_uid: str, tick: Decimal, request_id: str) -> dict:
    price = _resting_limit_price(token, instrument_uid, tick)
    return _sandbox_post(
        token,
        POST_ORDER,
        {
            "quantity": "1",
            "price": _quotation_payload(price),
            "direction": "ORDER_DIRECTION_BUY",
            "accountId": account_id,
            "orderType": "ORDER_TYPE_LIMIT",
            "orderId": request_id,
            "instrumentId": instrument_uid,
            "timeInForce": "TIME_IN_FORCE_DAY",
            "priceType": "PRICE_TYPE_CURRENCY",
        },
    )


def _broker_snapshot(token: str, account_id: str) -> tuple[list[dict], dict, dict]:
    orders = _sandbox_post(token, GET_ORDERS, {"accountId": account_id}).get("orders") or []
    portfolio = _sandbox_post(token, GET_PORTFOLIO, {"accountId": account_id})
    positions = _sandbox_post(token, GET_POSITIONS, {"accountId": account_id})
    return orders, portfolio, positions


def _find_order_by_request_id(orders: list[dict], request_id: str) -> dict | None:
    matches = [item for item in orders if item.get("orderRequestId") == request_id]
    if len(matches) > 1:
        raise QualificationError(f"multiple active broker orders match request_id={request_id}")
    return matches[0] if matches else None


def _prepare_restart(token: str, account_id: str, state_path: Path) -> None:
    _, instrument_uid, tick = _resolve_sber(token)
    request_id = _new_request_id()
    order = _submit_resting_order(token, account_id, instrument_uid, tick, request_id)
    broker_order_id = _require_text(order, "orderId")
    authoritative = _state(token, account_id, broker_order_id)
    status = authoritative.get("executionReportStatus")
    if status != "EXECUTION_REPORT_STATUS_NEW":
        raise QualificationError(f"expected resting NEW order, got {status}")
    state = PersistedState(
        account_id=account_id,
        instrument_uid=instrument_uid,
        client_request_id=request_id,
        broker_order_id=broker_order_id,
        expected_kind="resting_limit",
    )
    _write_state(state_path, state)
    print("Q4 RESTART PREPARED")
    print(f"  account={account_id}")
    print(f"  client_request_id={request_id}")
    print(f"  broker_order_id={broker_order_id}")
    print(f"  persisted={state_path}")
    print("STOP HERE. Start a new process and run --resume-restart with the same state file.")


def _resume_restart(token: str, state_path: Path, cleanup: bool) -> None:
    state = _read_state(state_path)
    orders, portfolio, positions = _broker_snapshot(token, state.account_id)
    order = _find_order_by_request_id(orders, state.client_request_id)
    authoritative = _state(token, state.account_id, state.broker_order_id)
    status = authoritative.get("executionReportStatus")
    if order is None and status == "EXECUTION_REPORT_STATUS_NEW":
        raise QualificationError("broker order is NEW but absent from GetSandboxOrders")
    if order is not None and _require_text(order, "orderId") != state.broker_order_id:
        raise QualificationError("request-id reconciliation resolved to a different broker order")

    print("Q4 RESTART RECONCILIATION")
    print(f"  persisted_client_request_id={state.client_request_id}")
    print(f"  persisted_broker_order_id={state.broker_order_id}")
    print(f"  authoritative_status={status}")
    print(f"  active_orders={len(orders)} portfolio_positions={len(portfolio.get('positions') or [])}")
    print(
        "  securities="
        f"{len(positions.get('securities') or [])} futures={len(positions.get('futures') or [])}"
    )
    print("PASS: fresh process reconstructed broker-authoritative order/account state without resubmitting the mutation")

    if cleanup and status == "EXECUTION_REPORT_STATUS_NEW":
        _sandbox_post(
            token,
            CANCEL_ORDER,
            {
                "accountId": state.account_id,
                "orderId": state.broker_order_id,
                "orderIdType": "ORDER_ID_TYPE_EXCHANGE",
            },
        )
        final = _state(token, state.account_id, state.broker_order_id)
        print(f"CLEANUP: final_status={final.get('executionReportStatus')}")


def _unknown_after_dispatch(token: str, account_id: str, cleanup: bool) -> None:
    _, instrument_uid, tick = _resolve_sber(token)
    request_id = _new_request_id()
    print("Q4 UNKNOWN-OUTCOME QUALIFICATION")
    print(f"DISPATCH request_id={request_id}")

    broker_response: dict | None = None
    try:
        # The HTTP call really reaches SandboxService. The harness deliberately discards
        # the response before the adapter can classify the broker result.
        broker_response = _submit_resting_order(token, account_id, instrument_uid, tick, request_id)
        raise AmbiguousMutationOutcome("simulated response loss after dispatch")
    except AmbiguousMutationOutcome as exc:
        print(f"LOCAL OUTCOME: UNKNOWN ({exc})")

    # Do not use broker_response for reconciliation: it represents the response that was lost.
    del broker_response
    orders, _, _ = _broker_snapshot(token, account_id)
    reconciled = _find_order_by_request_id(orders, request_id)
    if reconciled is None:
        raise QualificationError(
            "UNKNOWN could not be reconciled from GetSandboxOrders by orderRequestId; "
            "do not infer rejection"
        )

    broker_order_id = _require_text(reconciled, "orderId")
    authoritative = _state(token, account_id, broker_order_id)
    status = authoritative.get("executionReportStatus")
    print(f"RECONCILED broker_order_id={broker_order_id} status={status}")
    if status not in {
        "EXECUTION_REPORT_STATUS_NEW",
        "EXECUTION_REPORT_STATUS_PARTIALLYFILL",
        "EXECUTION_REPORT_STATUS_FILL",
    }:
        raise QualificationError(f"unexpected authoritative result after UNKNOWN: {status}")
    print("PASS: post-dispatch ambiguity remained UNKNOWN until broker evidence resolved it")
    print("PASS: no blind retry was performed")

    if cleanup and status in {"EXECUTION_REPORT_STATUS_NEW", "EXECUTION_REPORT_STATUS_PARTIALLYFILL"}:
        _sandbox_post(
            token,
            CANCEL_ORDER,
            {
                "accountId": account_id,
                "orderId": broker_order_id,
                "orderIdType": "ORDER_ID_TYPE_EXCHANGE",
            },
        )
        final = _state(token, account_id, broker_order_id)
        print(f"CLEANUP: final_status={final.get('executionReportStatus')}")


def main() -> int:
    parser = argparse.ArgumentParser(description="Q4 T-Invest Sandbox reconciliation qualification")
    parser.add_argument("--execute", action="store_true", help="allow SandboxService mutations")
    parser.add_argument("--account", help="existing sandbox account ID")
    parser.add_argument("--state-file", type=Path, default=DEFAULT_STATE)
    parser.add_argument("--prepare-restart", action="store_true")
    parser.add_argument("--resume-restart", action="store_true")
    parser.add_argument("--unknown-after-dispatch", action="store_true")
    parser.add_argument("--cleanup", action="store_true")
    args = parser.parse_args()

    token = os.environ.get("TINVEST_TOKEN")
    if not token:
        print("FAIL: TINVEST_TOKEN is not set", file=sys.stderr)
        return 2

    try:
        actions = sum(bool(x) for x in (args.prepare_restart, args.resume_restart, args.unknown_after_dispatch))
        if actions != 1:
            raise QualificationError("select exactly one of --prepare-restart, --resume-restart, --unknown-after-dispatch")
        if (args.prepare_restart or args.unknown_after_dispatch or args.cleanup) and not args.execute:
            raise QualificationError("mutation disabled: rerun with --execute for SandboxService writes")

        if args.resume_restart:
            _resume_restart(token, args.state_file, args.cleanup)
            return 0

        if not args.account:
            raise QualificationError("--account ACCOUNT_ID is required for this action")

        if args.prepare_restart:
            _prepare_restart(token, args.account, args.state_file)
        else:
            _unknown_after_dispatch(token, args.account, args.cleanup)
        return 0
    except Exception as exc:
        print(f"FAIL: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
