from __future__ import annotations

import os
from dataclasses import dataclass
from enum import StrEnum


class TradingEnvironment(StrEnum):
    SANDBOX = "sandbox"
    PAPER = "paper"
    LIVE = "live"


@dataclass(frozen=True, slots=True)
class RuntimeConfig:
    environment: TradingEnvironment
    tinvest_token: str
    live_mutations_enabled: bool = False

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
        if environment is TradingEnvironment.LIVE and not live_enabled:
            raise ValueError(
                "live environment is fail-closed; set TRADER_ENABLE_LIVE_MUTATIONS=1 only after explicit activation"
            )

        return cls(
            environment=environment,
            tinvest_token=token,
            live_mutations_enabled=live_enabled,
        )
