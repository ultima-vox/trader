from __future__ import annotations

import os
from dataclasses import dataclass
from enum import StrEnum


class TradingEnvironment(StrEnum):
    SANDBOX = "sandbox"
    PAPER = "paper"
    LIVE = "live"


class MutationDisabledError(RuntimeError):
    """Raised when broker mutations are not enabled for the active environment."""


@dataclass(frozen=True, slots=True)
class RuntimeConfig:
    environment: TradingEnvironment
    tinvest_token: str
    live_mutations_enabled: bool = False

    @property
    def broker_mutations_allowed(self) -> bool:
        if self.environment is TradingEnvironment.SANDBOX:
            return True
        if self.environment is TradingEnvironment.LIVE:
            return self.live_mutations_enabled
        return False

    def require_broker_mutations(self) -> None:
        if self.broker_mutations_allowed:
            return
        if self.environment is TradingEnvironment.LIVE:
            raise MutationDisabledError(
                "live broker mutations are fail-closed; explicit activation is required"
            )
        raise MutationDisabledError(
            f"broker mutations are not available in {self.environment.value!r} environment"
        )

    @classmethod
    def from_env(cls) -> "RuntimeConfig":
        raw_environment = os.getenv("TRADER_ENV", "sandbox").strip().lower()
        try:
            environment = TradingEnvironment(raw_environment)
        except ValueError as exc:
            raise ValueError(f"unsupported TRADER_ENV={raw_environment!r}") from exc

        token = os.getenv("TINVEST_TOKEN", "").strip()
        if not token:
            raise ValueError("TINVEST_TOKEN is required")

        live_enabled = os.getenv("TRADER_ENABLE_LIVE_MUTATIONS", "0").strip() == "1"

        return cls(
            environment=environment,
            tinvest_token=token,
            live_mutations_enabled=live_enabled,
        )
