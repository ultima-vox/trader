from nautilus_trader.model.enums import AssetClass

from qualification.live.q1_tinvest import _future_asset_class


def test_tinvest_prefixed_future_asset_types_map_to_nautilus() -> None:
    assert _future_asset_class("TYPE_COMMODITY") == AssetClass.COMMODITY
    assert _future_asset_class("TYPE_CURRENCY") == AssetClass.FX
    assert _future_asset_class("TYPE_INDEX") == AssetClass.INDEX
    assert _future_asset_class("TYPE_SECURITY") == AssetClass.EQUITY


def test_tinvest_unprefixed_future_asset_types_remain_supported() -> None:
    assert _future_asset_class("commodity") == AssetClass.COMMODITY
    assert _future_asset_class("currency") == AssetClass.FX
    assert _future_asset_class("index") == AssetClass.INDEX
    assert _future_asset_class("security") == AssetClass.EQUITY
