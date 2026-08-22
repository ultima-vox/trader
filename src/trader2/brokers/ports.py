from __future__ import annotations

from dataclasses import dataclass
from enum import StrEnum
from typing import Protocol, Sequence


class MutationOutcome(StrEnum):
    NOT_DISPATCHED = "not_dispatched"
    ACCEPTED = "accepted"
    REJECTED = "rejected"
    UNKNOWN = "unknown"


@dataclass(frozen=True, slots=True)
class BrokerIdentity:
    client_request_id: str
    client_order_id: str | None = None
    broker_order_id: str | None = None
    exchange_order_id: str | None = None


@dataclass(frozen=True, slots=True)
class OrderMutationResult:
    outcome: MutationOutcome
    identity: BrokerIdentity
    broker_status: str | None = None
    reason_code: str | None = None


@dataclass(frozen=True, slots=True)
class BrokerOrderReport:
    identity: BrokerIdentity
    status: str
    filled_quantity: str
    remaining_quantity: str | None = None


@dataclass(frozen=True, slots=True)
class BrokerAccountSnapshot:
    account_id: str
    order_reports: Sequence[BrokerOrderReport]
    position_count: int
    observed_at_ns: int


class InstrumentProvider(Protocol):
    async def load(self) -> None: ...


class MarketDataClient(Protocol):
    async def connect(self) -> None: ...
    async def disconnect(self) -> None: ...
    async def restore_subscriptions(self) -> None: ...


class ExecutionClient(Protocol):
    async def submit(self, command: object) -> OrderMutationResult: ...
    async def cancel(self, command: object) -> OrderMutationResult: ...
    async def replace(self, command: object) -> OrderMutationResult: ...


class ReconciliationProvider(Protocol):
    async def snapshot(self, account_id: str) -> BrokerAccountSnapshot: ...
