# T-Invest capability matrix for Trader 2.0

Status: **ACTIVE PRODUCT CONTRACT**

This document defines the provider surface Trader 2.0 intends to support from T-Invest. The goal is product completeness, not a narrow PoC. A capability may be unavailable because of account permissions, environment limitations, provider rollout or sandbox differences; that state must be reported explicitly by the adapter.

## Capability states

- `required` — core Trader 2.0 provider capability; runtime work is incomplete until implemented.
- `optional-permission` — implemented when the provider/account exposes it; absence must not prevent runtime startup.
- `deprecated-do-not-use` — provider has a replacement; production code must use the replacement.
- `environment-limited` — supported where T-Invest exposes it, with explicit sandbox/live capability differences.

## Instruments and reference data — method inventory

Official `InstrumentsService` contract checked 2026-08-24. `reference` below means `vox_tinvest::reference`. Unit evidence uses complete method-specific current-contract fixtures, identity checks, exact decimals, enum evolution, pagination, capabilities and mutation classification. Expanded live qualification covers every non-deprecated safe read and is opt-in; favorites mutations are never part of live qualification. "Live eligible" means covered by that runner, not credentialed evidence already captured.

| Service | Method | Class | Requirements | Rust module/state | Routing | Exact Nautilus target | Tests/live evidence | Deprecated replacement | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| InstrumentsService | `TradingSchedules` | read | token | `reference` / supported | `TRADER_ONLY` | — | historical rejection, legal-window, boundary, deterministic merge, partial-failure unit tests; read-only live eligible | — | UTC intervals retained; historical ranges rejected; future ranges split into provider-legal <=14-day windows |
| InstrumentsService | `BondBy` | read | token | `reference` / supported | `TRADER_AND_NAUTILUS` | `nautilus_model::instruments::Bond` | unit; read-only live eligible | — | mapping only when exact |
| InstrumentsService | `Bonds` | read | token | `reference` / supported | `TRADER_AND_NAUTILUS` | `nautilus_model::instruments::Bond` | every-family unit; read-only live eligible | — | provider catalogue retained |
| InstrumentsService | `GetBondCoupons` | read | token | `reference` / supported | `TRADER_ONLY` | — | DTO unit; read-only live eligible | — | exact money/quotation |
| InstrumentsService | `GetBondEvents` | read | token; rollout may vary | `reference` / supported | `TRADER_ONLY` | — | DTO unit; read-only live eligible | — | provider event enum retained |
| InstrumentsService | `CurrencyBy` | read | token | `reference` / supported | `TRADER_AND_NAUTILUS` | `nautilus_model::instruments::CurrencyPair` | unit; read-only live eligible | — | mapping only when pair semantics faithful |
| InstrumentsService | `Currencies` | read | token | `reference` / supported | `TRADER_AND_NAUTILUS` | `nautilus_model::instruments::CurrencyPair` | every-family unit; read-only live eligible | — | provider catalogue retained |
| InstrumentsService | `EtfBy` | read | token | `reference` / supported | `TRADER_AND_NAUTILUS` | `nautilus_model::instruments::Equity` | unit; read-only live eligible | — | fund identity retained before mapping |
| InstrumentsService | `Etfs` | read | token | `reference` / supported | `TRADER_AND_NAUTILUS` | `nautilus_model::instruments::Equity` | every-family unit; read-only live eligible | — | fund identity retained before mapping |
| InstrumentsService | `FutureBy` | read | token | `reference` / supported | `TRADER_AND_NAUTILUS` | `nautilus_model::instruments::FuturesContract` | exact economics unit; read-only live eligible | — | margin cross-check required before mapping |
| InstrumentsService | `Futures` | read | token | `reference` / supported | `TRADER_AND_NAUTILUS` | `nautilus_model::instruments::FuturesContract` | every-family/exact economics unit; Q1 live evidence | — | no approximated tick economics |
| InstrumentsService | `OptionBy` | read | token | `reference` / supported | `TRADER_AND_NAUTILUS` | `nautilus_model::instruments::OptionsContract` | option fail-closed unit; read-only live eligible | — | strike/expiry/type required before mapping |
| InstrumentsService | `Options` | read | do not call | `reference` / deprecated | `TRADER_ONLY` | — | registry unit | `OptionsBy` | no production method exposed |
| InstrumentsService | `OptionsBy` | read | token; basic asset filter | `reference` / supported | `TRADER_AND_NAUTILUS` | `nautilus_model::instruments::OptionsContract` | every-family/option unit; read-only live eligible | — | current list method |
| InstrumentsService | `ShareBy` | read | token | `reference` / supported | `TRADER_AND_NAUTILUS` | `nautilus_model::instruments::Equity` | identity unit; read-only live eligible | — | exact lot/tick required before mapping |
| InstrumentsService | `Shares` | read | token | `reference` / supported | `TRADER_AND_NAUTILUS` | `nautilus_model::instruments::Equity` | every-family unit; Q1 live evidence | — | provider catalogue retained |
| InstrumentsService | `Indicatives` | read | token | `reference` / supported | `TRADER_ONLY` | — | family/index-weight unit; read-only live eligible | — | index/commodity catalogue retained |
| InstrumentsService | `DfaBy` | read | token; rollout may vary | `reference` / supported | `TRADER_ONLY` | — | family DTO unit; read-only live eligible | — | no forced Nautilus type |
| InstrumentsService | `Dfas` | read | token; rollout may vary | `reference` / supported | `TRADER_ONLY` | — | every-family DTO unit; read-only live eligible | — | exact nominal/yield retained |
| InstrumentsService | `GetAccruedInterests` | read | token | `reference` / supported | `TRADER_ONLY` | — | DTO unit; read-only live eligible | — | exact quotation |
| InstrumentsService | `GetFuturesMargin` | read | token | `reference` / supported | `TRADER_ONLY` | — | exact margin/economics unit; Q1 live evidence | — | catalogue tick must match margin tick |
| InstrumentsService | `GetInstrumentBy` | read | token | `reference` / supported | `TRADER_ONLY` | — | by-ID/identity unit; read-only live eligible | — | UID/FIGI/ticker/class/position distinct |
| InstrumentsService | `GetDividends` | read | token | `reference` / supported | `TRADER_ONLY` | — | DTO unit; read-only live eligible | — | exact money/quotation |
| InstrumentsService | `GetAssetBy` | read | token | `reference` / supported | `TRADER_ONLY` | — | DTO unit; read-only live eligible | — | derivatives excluded by provider |
| InstrumentsService | `GetAssets` | read | token | `reference` / supported | `TRADER_ONLY` | — | DTO unit; read-only live eligible | — | optional type/status filters |
| InstrumentsService | `GetFavorites` | read | token; account | `reference` / supported | `TRADER_ONLY` | — | identity DTO unit; read-only live eligible | — | optional group ID |
| InstrumentsService | `EditFavorites` | mutation | account; explicit mutation authorization | `reference` / supported | `TRADER_ONLY` | — | single-attempt/environment unit; no live run | — | never retried |
| InstrumentsService | `CreateFavoriteGroup` | mutation | account; explicit mutation authorization | `reference` / supported | `TRADER_ONLY` | — | single-attempt/environment unit; no live run | — | never retried |
| InstrumentsService | `DeleteFavoriteGroup` | mutation | account; explicit mutation authorization | `reference` / supported | `TRADER_ONLY` | — | single-attempt/environment unit; no live run | — | never retried |
| InstrumentsService | `GetFavoriteGroups` | read | token; account | `reference` / supported | `TRADER_ONLY` | — | DTO unit; read-only live eligible | — | filters preserve IDs |
| InstrumentsService | `GetCountries` | read | token | `reference` / supported | `TRADER_ONLY` | — | DTO unit; read-only live eligible | — | provider spelling `alfa_*` retained |
| InstrumentsService | `FindInstrument` | read | token | `reference` / supported | `TRADER_ONLY` | — | search/identity unit; read-only live eligible | — | optional kind/tradable filters |
| InstrumentsService | `GetBrands` | read | token | `reference` / supported | `TRADER_ONLY` | — | pagination unit; read-only live eligible | — | page traversal adapter-owned |
| InstrumentsService | `GetBrandBy` | read | token | `reference` / supported | `TRADER_ONLY` | — | DTO unit; read-only live eligible | — | brand UID lookup |
| InstrumentsService | `GetAssetFundamentals` | read | token; rollout/tariff may vary | `reference` / supported | `TRADER_ONLY` | — | exact-decimal/capability unit; read-only live eligible | — | provider double fields retained lexically; no float API |
| InstrumentsService | `GetAssetReports` | read | token; rollout/tariff may vary | `reference` / supported | `TRADER_ONLY` | — | period/DTO unit; read-only live eligible | — | issuer calendar |
| InstrumentsService | `GetConsensusForecasts` | read | token; rollout/tariff may vary | `reference` / supported | `TRADER_ONLY` | — | pagination/enum unit; read-only live eligible | — | page traversal adapter-owned |
| InstrumentsService | `GetForecastBy` | read | token; rollout/tariff may vary | `reference` / supported | `TRADER_ONLY` | — | DTO/enum unit; read-only live eligible | — | recommendation unknowns retained |
| InstrumentsService | `GetInsiderDeals` | read | token; rollout/tariff may vary | `reference` / supported | `TRADER_ONLY` | — | cursor/exact-decimal unit; read-only live eligible | — | string cursor adapter-owned |
| InstrumentsService | `StructuredNoteBy` | read | token; rollout may vary | `reference` / supported | `TRADER_ONLY` | — | family DTO unit; read-only live eligible | — | no forced Nautilus type |
| InstrumentsService | `StructuredNotes` | read | token; rollout may vary | `reference` / supported | `TRADER_ONLY` | — | every-family DTO unit; read-only live eligible | — | exact provider catalogue retained |
| InstrumentsService | `News` | read | token; rollout/tariff may vary | `reference` / supported | `TRADER_ONLY` | — | cursor unit; read-only live eligible | — | int64 cursor; safe-read retries |
| InstrumentsService | `GetRiskRates` | read | token; account risk profile | `reference` / supported | `TRADER_ONLY` | — | exact rate/capability unit; read-only live eligible | — | per-instrument errors preserved |

### Instrument-type routing

Unknown future `InstrumentType` values remain explicit and default to `TRADER_ONLY`; they are never collapsed into a known family.

| Current `InstrumentType` | Vox representation | Routing decision |
| --- | --- | --- |
| `INSTRUMENT_TYPE_UNSPECIFIED` | `ProviderInstrumentType::Unspecified` | `TRADER_ONLY` |
| `INSTRUMENT_TYPE_BOND` | `ProviderInstrumentType::Bond` | `TRADER_AND_NAUTILUS` → `nautilus_model::instruments::Bond` when mapping is exact |
| `INSTRUMENT_TYPE_SHARE` | `ProviderInstrumentType::Share` | `TRADER_AND_NAUTILUS` → `nautilus_model::instruments::Equity` |
| `INSTRUMENT_TYPE_CURRENCY` | `ProviderInstrumentType::Currency` | `TRADER_AND_NAUTILUS` → `nautilus_model::instruments::CurrencyPair` only with faithful pair semantics |
| `INSTRUMENT_TYPE_ETF` | `ProviderInstrumentType::Etf` | `TRADER_AND_NAUTILUS` → `nautilus_model::instruments::Equity` |
| `INSTRUMENT_TYPE_FUTURES` | `ProviderInstrumentType::Futures` | `TRADER_AND_NAUTILUS` → `nautilus_model::instruments::FuturesContract` after exact economics checks |
| `INSTRUMENT_TYPE_SP` | `ProviderInstrumentType::StructuredNote` | `TRADER_ONLY` |
| `INSTRUMENT_TYPE_OPTION` | `ProviderInstrumentType::Option` | `TRADER_AND_NAUTILUS` → `nautilus_model::instruments::OptionsContract` after exact economics checks |
| `INSTRUMENT_TYPE_CLEARING_CERTIFICATE` | `ProviderInstrumentType::ClearingCertificate` | `TRADER_ONLY` |
| `INSTRUMENT_TYPE_INDEX` | `ProviderInstrumentType::Index` | `TRADER_ONLY` |
| `INSTRUMENT_TYPE_COMMODITY` | `ProviderInstrumentType::Commodity` | `TRADER_ONLY` |
| `INSTRUMENT_TYPE_DFA` | `ProviderInstrumentType::Dfa` | `TRADER_ONLY` |

## Market data

| Capability | T-Invest surface | Trader 2.0 status |
| --- | --- | --- |
| Historical candles | `GetCandles` | required |
| Last prices | `GetLastPrices` | required |
| Last/public trades | `GetLastTrades` | required |
| Order book snapshot | `GetOrderBook` | required |
| Close prices | `GetClosePrices` | required |
| Market values | `GetMarketValues` | required |
| Technical analysis | `GetTechAnalysis` | required |
| Trading status | `GetTradingStatus`, `GetTradingStatuses` | required |
| Streaming quotes/trades/books/status | `MarketDataStreamService` | required |
| Subscription recovery | adapter registry + reconnect | required |
| Order-book consistency handling | provider `is_consistent` semantics where available | required |

## Accounts and user

| Capability | T-Invest surface | Trader 2.0 status |
| --- | --- | --- |
| Accounts | `GetAccounts` | required |
| User/tariff/qualification info | `GetInfo` | required |
| Margin attributes | `GetMarginAttributes` | required |
| Additional account values | `GetAccountValues` | required |

## Portfolio, operations and reports

| Capability | T-Invest surface | Trader 2.0 status |
| --- | --- | --- |
| Portfolio | `GetPortfolio` | required |
| Positions | `GetPositions` | required |
| Withdraw limits | `GetWithdrawLimits` | required |
| Operations history | `GetOperationsByCursor` | required |
| Legacy operations | `GetOperations` | deprecated-do-not-use |
| Broker report | `GetBrokerReport` | required |
| Foreign issuer dividend report | `GetDividendsForeignIssuer` | environment-limited |
| Operations stream | `OperationsStreamService` | required where exposed |

## Orders and execution

| Capability | T-Invest surface | Trader 2.0 status |
| --- | --- | --- |
| Submit | `PostOrder` | required |
| Async submit | `PostOrderAsync` | required |
| Cancel | `CancelOrder` | required |
| Replace | `ReplaceOrder` | required |
| Order state | `GetOrderState` | required |
| Active/order list | `GetOrders` | required |
| Max lots | `GetMaxLots` | required |
| Pre-trade order cost | `GetOrderPrice` | required |
| Order/trade stream | `OrdersStreamService` | required |
| Idempotency | provider request IDs | required |
| UNKNOWN resolution | broker-authoritative reconciliation | required |

## Stop orders

| Capability | T-Invest surface | Trader 2.0 status |
| --- | --- | --- |
| Stop loss | `PostStopOrder` | required |
| Take profit | `PostStopOrder` | required |
| Stop limit | `PostStopOrder` | required |
| Trailing/instant-execution parameters | provider fields where supported | required |
| List stop orders | `GetStopOrders` | required |
| Cancel stop order | `CancelStopOrder` | required |
| Expiration semantics | provider fields | required |

## Sandbox

Sandbox equivalents exposed by T-Invest are required for development and qualification. Sandbox/live differences are represented through the capability registry, not hidden behind fake success values.

At minimum the adapter covers sandbox accounts, pay-in, orders, async orders, replace/cancel, order state/list, portfolio, positions, operations, stop orders, max lots, order price and withdraw limits where the provider exposes them.

## Signals and Autofollow

`SignalService` capabilities are integrated when exposed to the token/account.

T-Invest Autofollow author API is `optional-permission`. Trader 2.0 should support it as a separate provider module for eligible author accounts, including strategy/instrument data, signals, deferred/stop signals and portfolio-related surfaces. An ordinary brokerage account must not fail readiness because Autofollow author permissions are absent.

## Capability registry contract

Each adapter module exposes machine-readable capability state:

- supported;
- unsupported-by-provider;
- unsupported-in-environment;
- permission-denied;
- temporarily-unavailable;
- deprecated.

A missing optional capability may degrade only the feature that depends on it. A missing core execution/reconciliation capability prevents `READY` when that capability is required for the active trading mode.

## Engineering rules

1. Provider-specific payloads do not cross the T-Invest adapter boundary.
2. No instrument type is silently dropped because Nautilus lacks a perfect canonical class; use a typed Trader 2.0 provider/reference DTO and map to Nautilus only where execution/runtime semantics require it.
3. Preserve provider identities exactly: instrument UID, FIGI, ticker/class code, client request ID, broker order ID and exchange identifiers remain distinct fields.
4. Pagination and cursor traversal are adapter responsibilities.
5. Rate-limit policy is centralized per provider/service family.
6. 429, transient 5xx, disconnect and timeout behavior uses bounded backoff and does not turn ambiguous mutations into rejection.
7. Reference/news/fundamental data may use REST; latency-sensitive market and execution events use streaming interfaces where available.
8. Full support means implementation + typed mapping + tests + error semantics + capability reporting, not merely declaring an endpoint constant.
