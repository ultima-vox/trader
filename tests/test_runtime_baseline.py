from __future__ import annotations

import pytest

from trader2.config import MutationDisabledError, RuntimeConfig, TradingEnvironment
from trader2.runtime.readiness import ReadinessState, RuntimeReadiness


def test_default_environment_is_sandbox(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("TRADER_ENV", raising=False)
    monkeypatch.delenv("TRADER_ENABLE_LIVE_MUTATIONS", raising=False)
    monkeypatch.setenv("TINVEST_TOKEN", "test-token")

    config = RuntimeConfig.from_env()

    assert config.environment is TradingEnvironment.SANDBOX
    assert config.live_mutations_enabled is False
    assert config.broker_mutations_allowed is True


def test_live_environment_allows_read_only_but_mutations_fail_closed(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("TRADER_ENV", "live")
    monkeypatch.setenv("TINVEST_TOKEN", "test-token")
    monkeypatch.delenv("TRADER_ENABLE_LIVE_MUTATIONS", raising=False)

    config = RuntimeConfig.from_env()

    assert config.environment is TradingEnvironment.LIVE
    assert config.broker_mutations_allowed is False
    with pytest.raises(MutationDisabledError, match="fail-closed"):
        config.require_broker_mutations()


def test_live_mutations_require_explicit_activation(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("TRADER_ENV", "live")
    monkeypatch.setenv("TINVEST_TOKEN", "test-token")
    monkeypatch.setenv("TRADER_ENABLE_LIVE_MUTATIONS", "1")

    config = RuntimeConfig.from_env()

    assert config.broker_mutations_allowed is True
    config.require_broker_mutations()


def test_paper_environment_cannot_mutate_broker(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("TRADER_ENV", "paper")
    monkeypatch.setenv("TINVEST_TOKEN", "test-token")
    monkeypatch.setenv("TRADER_ENABLE_LIVE_MUTATIONS", "1")

    config = RuntimeConfig.from_env()

    assert config.broker_mutations_allowed is False
    with pytest.raises(MutationDisabledError, match="paper"):
        config.require_broker_mutations()


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
