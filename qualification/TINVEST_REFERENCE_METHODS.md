# T-Invest InstrumentsService safe-read audit

Contract snapshot: official T-Bank `invest/invest-contracts`, commit `762e720e27164213f41cac0b226c5698c2ae8199` (2026-07-31), checked 2026-08-24. `prost`/`tonic-build` generate all provider messages and 43 RPC client methods from vendored `instruments.proto`/`common.proto`; no handwritten reference wire DTOs remain. Proto `optional` and message presence stays `Option`; repeated fields use empty collections; enum wire numbers remain forward-compatible. Vox mapping validates required identity/economics and fails closed. Official OpenAPI at same revision cross-checks REST compatibility only.

Live status below means runner coverage, not fabricated credential evidence. `QUALIFIED` requires successful decode; permission/tariff/environment errors become explicit `GATED`; missing provider-owned feature samples become `UNAVAILABLE`; provider-sourced detail 404 remains failure. Paginated methods request page 2 when metadata/cursor says more.

| # | Method | Request source and constraints | Expected response | Capability and deterministic coverage |
|---:|---|---|---|---|
| 1 | TradingSchedules | UTC provider-current-or-future range; closed windows <=14 days; long ranges chunked | `TradingSchedulesResponse` | token; typed historical/partial/conflict errors; boundary/merge tests |
| 2 | BondBy | UID from `Bonds`; ticker requires class code | `InstrumentResponse` | token; ID enum/request-shape tests |
| 3 | Bonds | explicit status/exchange filters | `InstrumentsResponse` | token; optional complete instrument wire fixture |
| 4 | GetBondCoupons | bond UID from `Bonds`; optional from/to | `BondCouponsResponse` | token; complete coupon and omission fixtures |
| 5 | GetBondEvents | bond UID from `Bonds`; optional from/to; preserved event enum | `BondEventsResponse` | rollout may gate; complete audit/economics fixture |
| 6 | CurrencyBy | UID from `Currencies`; ticker requires class code | `InstrumentResponse` | token; shared validated lookup |
| 7 | Currencies | explicit status/exchange filters | `InstrumentsResponse` | token; complete instrument wire fixture |
| 8 | EtfBy | UID from `Etfs`; ticker requires class code | `InstrumentResponse` | token; shared validated lookup |
| 9 | Etfs | explicit status/exchange filters | `InstrumentsResponse` | token; complete instrument wire fixture |
| 10 | FutureBy | UID from `Futures`; ticker requires class code | `InstrumentResponse` | token; shared validated lookup |
| 11 | Futures | explicit status/exchange filters | `InstrumentsResponse` | token; complete instrument wire fixture |
| 12 | OptionBy | UID from same-run `OptionsBy` | `InstrumentResponse` | no option sample => `UNAVAILABLE`; option economics fail closed |
| 13 | OptionsBy | provider share asset UID/position UID/instrument UID | `InstrumentsResponse` | token; optional filter/response fixture |
| 14 | ShareBy | UID from `Shares`; ticker requires class code | `InstrumentResponse` | token; shared validated lookup |
| 15 | Shares | explicit status/exchange filters | `InstrumentsResponse` | token; identity promotion fails closed |
| 16 | Indicatives | empty request | `IndicativesResponse` | rollout may gate; index composition and unknown kind preserved |
| 17 | DfaBy | UID from same-run `Dfas` | `DfaResponse` | no DFA sample => `UNAVAILABLE`; rollout may gate |
| 18 | Dfas | empty request | `DfasResponse` | rollout may gate; DFA economics optional/exact |
| 19 | GetAccruedInterests | bond UID from `Bonds`; required from/to | `AccruedInterestsResponse` | token; typed required-period request; exact quotation/omission fixture |
| 20 | GetFuturesMargin | future UID from `Futures` | generated `GetFuturesMarginResponse` | generated-protobuf omitted-margin fixture; canonical trading validation fails closed |
| 21 | GetInstrumentBy | UID from `Shares` | `InstrumentResponse` | token; shared validated lookup |
| 22 | GetDividends | share UID from `Shares`; optional from/to | `DividendsResponse` | token; complete money/timestamp fixture |
| 23 | GetAssetBy | UID from same-run `GetAssets` | `AssetResponse` | token; extension fields retained |
| 24 | GetAssets | optional type/status filters | `AssetsResponse` | token; incomplete rows decode independently |
| 25 | GetFavorites | optional group ID; read only | `FavoritesResponse` | account/permission gate; current favorite fields fixture |
| 26 | GetFavoriteGroups | share UID from `Shares`; optional exclusions | `FavoriteGroupsResponse` | account/permission gate; no mutation |
| 27 | GetCountries | empty request | `CountriesResponse` | token; omission fixture |
| 28 | FindInstrument | ticker from `Shares`; known kind filter | `FindInstrumentResponse` | token; candle dates and unknown kind preserved |
| 29 | GetBrands | provider paging; page 2 when present | `BrandsResponse` | token; proto3-zero pagination test |
| 30 | GetBrandBy | UID from same-run accumulated `GetBrands` pages | `BrandResponse` | no brand sample => `UNAVAILABLE` |
| 31 | GetAssetFundamentals | 1..=100 non-empty asset UIDs from `GetAssets` | `FundamentalsResponse` | tariff/rollout gate; exact decimal and unrelated-field isolation tests |
| 32 | GetAssetReports | share UID from `Shares`; optional from/to | `AssetReportsResponse` | tariff/rollout gate; complete timestamp fixture |
| 33 | GetConsensusForecasts | provider paging; page 2 when present | `ConsensusForecastsResponse` | tariff/rollout gate; pagination and omission tests |
| 34 | GetForecastBy | consensus `asset_uid` -> `GetAssetBy` -> deterministic bounded `AssetFull.instruments[].uid` | generated `GetForecastResponse` | no candidates => typed inconsistency; candidate 404 probes bounded; record UID never reused |
| 35 | GetInsiderDeals | share UID from `Shares`; positive limit; next cursor page | `InsiderDealsResponse` | tariff/rollout gate; cursor and exact decimal fixture |
| 36 | StructuredNoteBy | UID from same-run `StructuredNotes` | `InstrumentResponse` | no sample => `UNAVAILABLE`; rollout may gate |
| 37 | StructuredNotes | explicit status/exchange filters | `InstrumentsResponse` | rollout may gate; optional instrument wire fixture |
| 38 | News | positive limit; next numeric cursor page | `NewsResponse` | tariff/rollout gate; exact int64/timestamp/nested fixture |
| 39 | GetRiskRates | non-empty provider share/future UIDs | `RiskRatesResponse` | account profile gate; exact quotation/omission fixture |

Deprecated `Options` and four mutation methods remain outside 39 safe-read qualification. Live runner never calls mutations.
