from __future__ import annotations

import pytest

from trader2.brokers.tinvest.transport import TInvestHttpTransport


def test_transport_does_not_expose_token_in_repr() -> None:
    transport = TInvestHttpTransport(token="super-secret-token")

    assert "super-secret-token" not in repr(transport)


def test_transport_rejects_empty_token() -> None:
    with pytest.raises(ValueError, match="token"):
        TInvestHttpTransport(token="   ")


def test_transport_rejects_non_https_base_url() -> None:
    with pytest.raises(ValueError, match="HTTPS"):
        TInvestHttpTransport(token="test", base_url="http://example.invalid/rest")


def test_transport_rejects_non_positive_timeout() -> None:
    with pytest.raises(ValueError, match="timeout_seconds"):
        TInvestHttpTransport(token="test", timeout_seconds=0)
