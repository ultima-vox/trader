# T-Invest capability matrix for Trader 2.0

Status: **ACTIVE PRODUCT CONTRACT**

This document defines the provider surface Trader 2.0 intends to support from T-Invest. The goal is product completeness, not a narrow PoC. A capability may be unavailable because of account permissions, environment limitations, provider rollout or sandbox differences; that state must be reported explicitly by the adapter.

## Capability states

- `required` — core Trader 2.0 provider capability; runtime work is incomplete until implemented.
- `optional-permission` — implemented when the provider/account exposes it; absence must not prevent runtime startup.
- `deprecated-do-not-use` — provider has a replacement; production code must use the replacement.
- `environment-limited` — supported where T-Invest exposes it, with explicit sandbox/live capability differences.

## Instruments and reference data

| Capability | T-Invest surface | Trader 2.0 status |
| --- | --- | --- |
| Shares | `Shares`, `ShareBy` | required |
| Bonds | `Bonds`, `BondBy` | required |
| ETFs/funds | `Etfs`, `EtfBy` | required |
| Currencies | `Currencies`, `CurrencyBy` | required |
| Futures | `Futures`, `FutureBy`, `GetFuturesMargin` | required |
| Options | `OptionsBy`, `OptionBy` | required |
| Legacy options list | `Options` | deprecated-do-not-use |
| Structured notes | `StructuredNotes`, `StructuredNoteBy` | required |
| Digital financial assets | `Dfas`, `DfaBy` | required |
| Indicatives | `Indicatives` | required |
| Generic instrument lookup | `GetInstrumentBy`, `FindInstrument` | required |
| Assets | `GetAssets`, `GetAssetBy` | required |
| Brands | `GetBrands`, `GetBrandBy` | required |
| Countries | `GetCountries` | required |
| Trading schedules | `TradingSchedules` | required |
| Dividends | `GetDividends` | required |
| Bond coupons | `GetBondCoupons` | required |
| Bond accrued interest | `GetAccruedInterests` | required |
| Bond events | `GetBondEvents` | required |
| Risk rates | `GetRiskRates` | required |
| Fundamentals | `GetAssetFundamentals` | required |
| Issuer report calendar | `GetAssetReports` | required |
| Analyst consensus | `GetConsensusForecasts` | required |
| Forecast detail | `GetForecastBy` | required |
| Insider deals | `GetInsiderDeals` | required |
| News | `News` | required |
| Favorites | `GetFavorites`, `EditFavorites` | required for product UX |
| Favorite groups | create/get/delete group methods | required for product UX |

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
