from __future__ import annotations

import asyncio
import json
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Any


class TInvestTransportError(RuntimeError):
    """Transport or HTTP failure at the T-Invest boundary."""


@dataclass(frozen=True, slots=True)
class TInvestHttpTransport:
    token: str
    base_url: str = "https://invest-public-api.tbank.ru/rest"
    timeout_seconds: float = 20.0

    async def post(self, path: str, payload: dict[str, Any]) -> dict[str, Any]:
        return await asyncio.to_thread(self._post_sync, path, payload)

    def _post_sync(self, path: str, payload: dict[str, Any]) -> dict[str, Any]:
        if not path.startswith("/"):
            raise ValueError("T-Invest method path must start with '/'")
        body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
        request = urllib.request.Request(
            f"{self.base_url}{path}",
            data=body,
            method="POST",
            headers={
                "Authorization": f"Bearer {self.token}",
                "Content-Type": "application/json",
                "Accept": "application/json",
            },
        )
        try:
            with urllib.request.urlopen(request, timeout=self.timeout_seconds) as response:
                raw = response.read()
        except urllib.error.HTTPError as exc:
            raw = exc.read().decode("utf-8", errors="replace")
            raise TInvestTransportError(
                f"T-Invest HTTP {exc.code} for {path}: {raw}"
            ) from exc
        except urllib.error.URLError as exc:
            raise TInvestTransportError(f"T-Invest connection failed for {path}: {exc}") from exc

        if not raw:
            return {}
        try:
            decoded = json.loads(raw)
        except json.JSONDecodeError as exc:
            raise TInvestTransportError(f"T-Invest returned invalid JSON for {path}") from exc
        if not isinstance(decoded, dict):
            raise TInvestTransportError(f"T-Invest returned non-object JSON for {path}")
        return decoded
