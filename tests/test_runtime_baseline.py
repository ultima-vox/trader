from __future__ import annotations

import pytest

from trader2.config import RuntimeConfig, TradingEnvironment
from trader2.runtime.readiness import ReadinessState, RuntimeReadiness


def test_default_environment_is_sandbox(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("TRADER_ENV", raising=False)
    monkeypatch.delenv("TRADER_ENABLE_LIVE_MUTATIONS", raising=False)
    monkeypatch.setenv("TINVEST_TOKEN", "test-token")

    config = RuntimeConfig.from_env()

    assert config.environment is TradingEnvironment.SANDBOX
    assert config.live_mutations_enabled is False


def test_live_environment_is_fail_closed(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("TRADER_ENV", "live")
    monkeypatch.setenv("TINVEST_TOKEN", "test-token")
    monkeypatch.delenv("TRADER_ENABLE_LIVE_MUTATIONS", raising=False)

    with pytest.raises(ValueError, match="fail-closed"):
        RuntimeConfig.from_env()


def test_runtime_cannot_open_new_exposure_before_reconciliation() -> None:
    readiness = RuntimeReadiness()
    assert readiness.can_open_new_exposure is False

    readiness.transition(ReadinessState.CONNECTING)
    readiness.transition(ReadinessState.RECONCILING)
    assert readiness.can_open_new_exposure is False

    readiness.transition(ReadinessState.READY)
    assert readiness.can_open_new_exposure is True


def test_invalid_readiness_transition_is_rejected() -> None:
    readiness = RuntimeReadiness()
    with pytest.raises(ValueError, match="invalid readiness transition"):
        readiness.transition(ReadinessState.READY)
