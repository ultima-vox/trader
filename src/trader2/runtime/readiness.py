from __future__ import annotations

from dataclasses import dataclass, field
from enum import StrEnum


class ReadinessState(StrEnum):
    STARTING = "starting"
    CONNECTING = "connecting"
    RECONCILING = "reconciling"
    READY = "ready"
    DEGRADED = "degraded"
    HALTED = "halted"


_ALLOWED: dict[ReadinessState, set[ReadinessState]] = {
    ReadinessState.STARTING: {ReadinessState.CONNECTING, ReadinessState.HALTED},
    ReadinessState.CONNECTING: {
        ReadinessState.RECONCILING,
        ReadinessState.DEGRADED,
        ReadinessState.HALTED,
    },
    ReadinessState.RECONCILING: {
        ReadinessState.READY,
        ReadinessState.DEGRADED,
        ReadinessState.HALTED,
    },
    ReadinessState.READY: {
        ReadinessState.RECONCILING,
        ReadinessState.DEGRADED,
        ReadinessState.HALTED,
    },
    ReadinessState.DEGRADED: {
        ReadinessState.CONNECTING,
        ReadinessState.RECONCILING,
        ReadinessState.HALTED,
    },
    ReadinessState.HALTED: {ReadinessState.CONNECTING},
}


@dataclass(slots=True)
class RuntimeReadiness:
    state: ReadinessState = ReadinessState.STARTING
    reason: str | None = None
    history: list[tuple[ReadinessState, str | None]] = field(default_factory=list)

    @property
    def can_open_new_exposure(self) -> bool:
        return self.state is ReadinessState.READY

    def transition(self, target: ReadinessState, *, reason: str | None = None) -> None:
        if target is self.state:
            self.reason = reason
            return
        if target not in _ALLOWED[self.state]:
            raise ValueError(f"invalid readiness transition {self.state.value} -> {target.value}")
        self.history.append((self.state, self.reason))
        self.state = target
        self.reason = reason
