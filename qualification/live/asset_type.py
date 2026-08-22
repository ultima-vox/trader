from __future__ import annotations

from nautilus_trader.model.enums import AssetClass


class AssetTypeMappingError(ValueError):
    pass


def future_asset_class(asset_type: str) -> AssetClass:
    """Map T-Invest futures assetType to Nautilus AssetClass.

    T-Invest REST currently emits enum-like values such as TYPE_COMMODITY.
    Older/unprefixed spellings are accepted to keep the qualification helper
    tolerant to representation differences, but unknown values fail closed.
    """

    normalized = asset_type.strip().upper()
    if normalized.startswith("TYPE_"):
        normalized = normalized.removeprefix("TYPE_")

    mapping = {
        "SECURITY": AssetClass.EQUITY,
        "INDEX": AssetClass.INDEX,
        "CURRENCY": AssetClass.FX,
        "COMMODITY": AssetClass.COMMODITY,
    }
    try:
        return mapping[normalized]
    except KeyError as exc:
        raise AssetTypeMappingError(
            f"unsupported T-Invest future asset_type={asset_type!r}"
        ) from exc
