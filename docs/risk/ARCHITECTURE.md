# Risk foundation

Issue #21 owns Vox pre-trade decisions, policy state, and reservations. `vox-runtime`
owns mutation fencing and broker reconciliation. Production composition always installs
`ProductionRiskAdapter`; no capital command reaches an execution port without a typed risk
admission.

## Dispatch ordering

1. Resolve exact connection/account and current #17 execution-authorization revision.
2. Read latest broker-authoritative #11 reconciliation snapshot.
3. Read T-Invest instrument/lot/trading-status constraints and applicable four-way
   `GetMaxLots` limits. Provider margin limits never enable Vox margin policy.
4. Evaluate typed `APPROVE`, `RESIZE`, `REJECT`, `REDUCE_ONLY`, or `HALT` decision.
5. Atomically persist decision plus exposure reservation with `BEGIN IMMEDIATE`.
6. Revalidate policy, authorization, runtime, reconciliation, position, and order watermarks.
7. Persist #11 mutation intent and `UNKNOWN_AFTER_DISPATCH` fence; only then call broker.

Any proven failure before broker dispatch releases reservation. Queue saturation and stale
approval are pre-dispatch failures. Broker rejection releases it. ACK retains it until
broker-authoritative order/fill reconciliation. Ambiguous dispatch moves it to
`UNKNOWN_HELD`; cancel request alone never releases capacity.

Replay uses `(account_id, logical_request_id)` and returns original atomic approval only when
command action, instrument, and requested signed quantity match and no runtime journal entry
exists. Changed replay semantics fail closed.

## Broker-first facts

- `GetPortfolio` / `GetPositions` / `GetOrders` feed shared reconciliation snapshot.
- `GetPortfolio.daily_yield` remains broker day P&L with provider meaning. Vox does not label it
  realized or unrealized P&L and defines no custom split.
- Instrument UID lookup supplies exact lot and side availability. `GetTradingStatus` supplies
  current API and order-type availability.
- `GetMaxLots` keeps buy-own, buy-margin, sell-own, and sell-margin limits distinct. Market buy
  also uses provider `max_market_lots`. `confirm_margin_trade=false` selects own-funds/own-position
  limits even when larger margin limits exist.
- `GetOrderPrice` is used only as a limit-order estimate when an exposure/notional policy needs
  it. It is never execution price or market-data freshness.
- `GetMarginAttributes` values remain exact broker fields. Configured margin utilization is a Vox
  derived metric, version 1:

  `corrected_margin / liquid_portfolio * 1_000_000 ppm`

  Both operands come from one broker response and must share currency. Integer division rounds
  down. Missing/non-positive inputs block new exposure. Sandbox values qualify plumbing and
  deterministic policy only, not production margin economics.
- `GetWithdrawLimits` is not used as buying power.

Instrument constraint calls are coalesced through a one-second cache; risk broker queries have a
four-decision concurrency gate. Request-specific `GetMaxLots`, `GetOrderPrice`, and margin calls
remain bounded and are made only when relevant.

## Quantity and state rules

Signed projected lots are:

`broker position + open orders + unresolved UNKNOWN + active reservations + request`

Reduction classification is quantity-aware. Crossing through zero has a reducing portion plus
new exposure; `REDUCE_ONLY` blocks latter. `HALTED` and `KILL_SWITCH` block new capital actions but
allow cancel/protection cleanup and explicitly modeled emergency reductions. Cleanup still needs
valid credentials, connection ownership, and epoch.

Account policy starts `REDUCE_ONLY`. State changes use optimistic policy revision, audited reason,
`ChangeRiskPolicy` RBAC, and durable SQLite WAL/FULL persistence. Current production transport is
account-scoped. Global/strategy state row types exist for future caller identity/config layers;
they grant no bypass while unattached.

## Missing/stale input

No safety-critical value becomes zero by default. Existing portfolio gross/net/instrument marked
valuation is unavailable in current #11 snapshot, so enabling those limits blocks new exposure
with `CRITICAL_INPUT_MISSING`. A reconciliation timestamp is not a quote timestamp; market-data
freshness policy therefore blocks new exposure until a real shared quote watermark is attached.
Optional analytics do not block when no policy depends on them.

Every decision stores policy and account-state watermark. Production dispatch rechecks policy,
execution authorization, runtime epoch, reconciliation, positions, orders, command semantics, and
new UNKNOWN conflicts immediately before durable dispatch fencing.
