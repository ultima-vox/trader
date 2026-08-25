use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::io;
use std::time::Duration;

use vox_domain::{
    CancelOrderCommand, CancelStopOrderCommand, ClientRequestId, Environment, FixedPoint,
    MutationAuthorization, MutationEvidence, MutationEvidenceStore, MutationGuard, OrderSide,
    PositionSide, ProtectionPlan, ProviderOrderIdentityKind, RegularOrderCommand, RegularOrderType,
    ReplaceOrderCommand, StopLossProtection, StoreError, TakeProfitProtection, TimeInForce,
    TrailingDistance, TrailingDistanceMode,
};
use vox_tinvest::account::ProviderTimestamp;
use vox_tinvest::execution::{
    CanonicalMaxLots, CanonicalOrderPrice, CanonicalOrderState, ProtectionRequestContext,
    ProtectionRequestIds, async_regular_order_request, cancel_order_request,
    cancel_stop_order_request, canonical_orders, canonical_stop_orders, protection_requests,
    regular_order_request, replace_order_request,
};
use vox_tinvest::execution_dispatch::{ExecutionMutationDispatcher, ExecutionRoute};
use vox_tinvest::execution_qualification::{
    QualificationEvidence, SandboxQualificationLedger, qualify_ambiguous_dispatch_guard,
};
use vox_tinvest::execution_stream::{
    ExecutionStreamConfig, ExecutionStreamEvent, ExecutionStreamKind, ExecutionStreamSupervisor,
};
use vox_tinvest::generated::v1;
use vox_tinvest::{GrpcCredential, GrpcErrorKind, SecretToken, TInvestGrpcClient};

type BoxError = Box<dyn Error + Send + Sync>;

#[derive(Default)]
struct MemoryEvidenceStore(BTreeMap<ClientRequestId, MutationEvidence>);

impl MutationEvidenceStore for MemoryEvidenceStore {
    fn load(&self, id: &ClientRequestId) -> Result<Option<MutationEvidence>, StoreError> {
        Ok(self.0.get(id).cloned())
    }

    fn persist(&mut self, evidence: &MutationEvidence) -> Result<(), StoreError> {
        self.0
            .insert(evidence.client_request_id().clone(), evidence.clone());
        Ok(())
    }

    fn claim_dispatch(&mut self, evidence: &MutationEvidence) -> Result<bool, StoreError> {
        if self.0.contains_key(evidence.client_request_id()) {
            return Ok(false);
        }
        self.persist(evidence)?;
        Ok(true)
    }

    fn resolve_unknown(
        &mut self,
        expected: &MutationEvidence,
        resolved: &MutationEvidence,
    ) -> Result<bool, StoreError> {
        if self.0.get(expected.client_request_id()) != Some(expected) {
            return Ok(false);
        }
        self.persist(resolved)?;
        Ok(true)
    }
}

#[derive(Clone)]
struct InstrumentFixture {
    uid: String,
    current_price: FixedPoint,
    tick: FixedPoint,
    lot_size: i64,
}

#[derive(Default)]
struct QualificationState {
    account_id: Option<String>,
    instrument: Option<InstrumentFixture>,
    baseline_order_ids: BTreeSet<String>,
    baseline_stop_ids: BTreeSet<String>,
    baseline_instrument_balance: Option<i64>,
    logical_order_ids: BTreeSet<String>,
    limit_order_id: Option<String>,
    limit_request_id: Option<String>,
    net_open_lots: i64,
}

struct SandboxRunner {
    client: TInvestGrpcClient,
    dispatcher: ExecutionMutationDispatcher<MemoryEvidenceStore>,
    state: QualificationState,
}

impl SandboxRunner {
    fn new(client: TInvestGrpcClient) -> Result<Self, BoxError> {
        if client.environment() != Environment::Sandbox {
            return Err(failure(
                "execution qualification client is not sandbox-bound",
            ));
        }
        Ok(Self {
            client,
            dispatcher: ExecutionMutationDispatcher::new(MemoryEvidenceStore::default()),
            state: QualificationState::default(),
        })
    }

    async fn account_readiness(&mut self) -> Result<String, BoxError> {
        let accounts = self
            .client
            .get_sandbox_accounts(v1::GetAccountsRequest::default())
            .await?
            .body
            .accounts;
        let account_id = accounts
            .into_iter()
            .find(|account| {
                account.status == v1::AccountStatus::Open as i32 && !account.id.trim().is_empty()
            })
            .map(|account| account.id)
            .ok_or_else(|| failure("sandbox has no open account"))?;

        let orders = self
            .client
            .get_sandbox_orders(v1::GetOrdersRequest {
                account_id: account_id.clone(),
                advanced_filters: None,
            })
            .await?
            .body;
        self.state.baseline_order_ids = orders
            .orders
            .into_iter()
            .filter_map(|order| nonempty(order.order_id))
            .collect();
        let stops = self
            .client
            .get_sandbox_stop_orders(active_stops_request(&account_id))
            .await?
            .body;
        self.state.baseline_stop_ids = stops
            .stop_orders
            .into_iter()
            .filter_map(|stop| nonempty(stop.stop_order_id))
            .collect();

        let shares = self
            .client
            .shares(v1::InstrumentsRequest {
                instrument_status: Some(v1::InstrumentStatus::Base as i32),
                instrument_exchange: None,
            })
            .await?
            .body
            .instruments;
        let mut candidates = shares
            .into_iter()
            .filter(|share| {
                share.api_trade_available_flag
                    && share.buy_available_flag
                    && share.sell_available_flag
                    && !share.uid.trim().is_empty()
                    && share.lot > 0
                    && share.min_price_increment.is_some()
            })
            .collect::<Vec<_>>();
        candidates.sort_by_key(|share| {
            (
                share.ticker != "SBER",
                share.class_code != "TQBR",
                share.ticker.clone(),
                share.uid.clone(),
            )
        });
        candidates.truncate(100);
        if candidates.is_empty() {
            return Err(failure("no API-tradeable share with exact tick metadata"));
        }
        let prices = self
            .client
            .get_last_prices(v1::GetLastPricesRequest {
                instrument_id: candidates.iter().map(|share| share.uid.clone()).collect(),
                last_price_type: v1::LastPriceType::LastPriceExchange as i32,
                ..Default::default()
            })
            .await?
            .body
            .last_prices
            .into_iter()
            .filter_map(|price| price.price.map(|value| (price.instrument_uid, value)))
            .collect::<BTreeMap<_, _>>();
        let (share, price) = candidates
            .into_iter()
            .filter_map(|share| prices.get(&share.uid).cloned().map(|price| (share, price)))
            .min_by_key(|(share, _)| {
                (
                    share.ticker != "SBER",
                    share.class_code != "TQBR",
                    share.ticker.clone(),
                    share.uid.clone(),
                )
            })
            .ok_or_else(|| failure("tradeable share catalogue has no authoritative last price"))?;
        let current_price = fixed(price)?;
        let tick = fixed(
            share
                .min_price_increment
                .ok_or_else(|| failure("selected share omitted price increment"))?,
        )?;
        if current_price.total_nanos() <= 0 || tick.total_nanos() <= 0 {
            return Err(failure("selected share price/tick is non-positive"));
        }
        self.state.account_id = Some(account_id.clone());
        self.state.instrument = Some(InstrumentFixture {
            uid: share.uid.clone(),
            current_price,
            tick,
            lot_size: i64::from(share.lot),
        });
        self.state.baseline_instrument_balance =
            Some(self.instrument_balance(&account_id, &share.uid).await?);
        Ok(format!(
            "sandbox account {account_id}; instrument {} selected from generated catalogue",
            share.uid
        ))
    }

    async fn max_lots(&self) -> Result<String, BoxError> {
        let (account, instrument) = self.context()?;
        let response = self
            .client
            .get_sandbox_max_lots(v1::GetMaxLotsRequest {
                account_id: account.to_owned(),
                instrument_id: instrument.uid.clone(),
                price: Some(quotation(instrument.current_price)?),
            })
            .await?
            .body;
        let canonical = CanonicalMaxLots::try_from(response)?;
        Ok(format!(
            "provider capacity read; currency={:?}; buy={:?}; sell={:?}",
            canonical.currency, canonical.buy, canonical.sell
        ))
    }

    async fn pre_trade_estimate(&self) -> Result<String, BoxError> {
        let (account, instrument) = self.context()?;
        let response = self
            .client
            .get_sandbox_order_price(v1::GetOrderPriceRequest {
                account_id: account.to_owned(),
                instrument_id: instrument.uid.clone(),
                price: Some(quotation(instrument.current_price)?),
                direction: v1::OrderDirection::Buy as i32,
                quantity: 1,
            })
            .await?
            .body;
        let canonical = CanonicalOrderPrice::try_from(response)?;
        Ok(format!(
            "provider estimate read; lots={}; total_present={}",
            canonical.lots_requested,
            canonical.total_order_amount.is_some()
        ))
    }

    fn context(&self) -> Result<(&str, &InstrumentFixture), BoxError> {
        Ok((
            self.state
                .account_id
                .as_deref()
                .ok_or_else(|| failure("account readiness failed"))?,
            self.state
                .instrument
                .as_ref()
                .ok_or_else(|| failure("instrument readiness failed"))?,
        ))
    }

    fn owned_context(&self) -> Result<(String, InstrumentFixture), BoxError> {
        let (account, instrument) = self.context()?;
        Ok((account.to_owned(), instrument.clone()))
    }

    async fn instrument_balance(
        &self,
        account: &str,
        instrument_uid: &str,
    ) -> Result<i64, BoxError> {
        Ok(self
            .client
            .get_sandbox_positions(v1::PositionsRequest {
                account_id: account.to_owned(),
            })
            .await?
            .body
            .securities
            .into_iter()
            .find(|position| position.instrument_uid == instrument_uid)
            .map_or(0, |position| position.balance))
    }

    async fn post_regular(
        &mut self,
        side: OrderSide,
        order_type: RegularOrderType,
        quantity: i64,
        price: Option<FixedPoint>,
    ) -> Result<CanonicalOrderState, BoxError> {
        let (account, instrument) = self.owned_context()?;
        let request_id = logical_id();
        let request = regular_order_request(&RegularOrderCommand {
            account_id: account.clone(),
            instrument_id: instrument.uid,
            client_request_id: request_id.clone(),
            quantity_lots: quantity,
            price,
            side,
            order_type,
            time_in_force: (order_type == RegularOrderType::Limit).then_some(TimeInForce::Day),
            confirm_margin_trade: false,
        })?;
        let acknowledged = self
            .dispatcher
            .post_order(
                &self.client,
                authorization()?,
                ExecutionRoute::Sandbox,
                request,
            )
            .await?;
        self.state.logical_order_ids.insert(request_id.clone());
        let broker_id = nonempty(acknowledged.response.body.order_id)
            .ok_or_else(|| failure("PostSandboxOrder omitted broker order id"))?;
        let state = if order_type == RegularOrderType::Market {
            self.wait_for_terminal_order(&account, &broker_id).await
        } else {
            self.wait_for_order(&account, &broker_id, ProviderOrderIdentityKind::BrokerOrder)
                .await
        };
        state.map_err(|_| {
            failure(format!(
                "order {broker_id} was acknowledged but authoritative state was unavailable"
            ))
        })
    }

    async fn wait_for_order(
        &self,
        account: &str,
        order_id: &str,
        identity: ProviderOrderIdentityKind,
    ) -> Result<CanonicalOrderState, BoxError> {
        let mut last_error = None;
        for _ in 0..8 {
            match self
                .client
                .get_sandbox_order_state(v1::GetOrderStateRequest {
                    account_id: account.to_owned(),
                    order_id: order_id.to_owned(),
                    price_type: v1::PriceType::Unspecified as i32,
                    order_id_type: Some(match identity {
                        ProviderOrderIdentityKind::BrokerOrder => v1::OrderIdType::Exchange as i32,
                        ProviderOrderIdentityKind::ClientRequest => v1::OrderIdType::Request as i32,
                    }),
                })
                .await
            {
                Ok(response) => return Ok(response.body.try_into()?),
                Err(error) => last_error = Some(error.to_string()),
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        Err(failure(format!(
            "authoritative order lookup exhausted: {}",
            last_error.unwrap_or_else(|| "no provider response".to_owned())
        )))
    }

    async fn wait_for_terminal_order(
        &self,
        account: &str,
        broker_id: &str,
    ) -> Result<CanonicalOrderState, BoxError> {
        let mut last = None;
        for _ in 0..8 {
            let state = self
                .wait_for_order(account, broker_id, ProviderOrderIdentityKind::BrokerOrder)
                .await?;
            if terminal(state.execution_status) {
                return Ok(state);
            }
            last = Some(state);
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        last.ok_or_else(|| failure("market order produced no authoritative state"))
    }

    async fn cancel_broker_order(&mut self, broker_id: &str) -> Result<(), BoxError> {
        let (account, _) = self.owned_context()?;
        let request = cancel_order_request(&CancelOrderCommand {
            account_id: account,
            order_id: broker_id.to_owned(),
            order_id_kind: Some(ProviderOrderIdentityKind::BrokerOrder),
        })?;
        self.dispatcher
            .cancel_order(
                &self.client,
                authorization()?,
                ExecutionRoute::Sandbox,
                ClientRequestId::new(logical_id())?,
                request,
            )
            .await?;
        Ok(())
    }

    async fn market_order_lifecycle(&mut self) -> Result<String, BoxError> {
        let buy = self
            .post_regular(OrderSide::Buy, RegularOrderType::Market, 1, None)
            .await?;
        self.state.net_open_lots += buy.lots_executed;
        let buy_id = buy
            .broker_order_id
            .as_deref()
            .ok_or_else(|| failure("market order state omitted broker identity"))?;
        if !terminal(buy.execution_status) {
            self.cancel_broker_order(buy_id).await?;
        }
        if buy.lots_executed > 0 {
            let sell = self
                .post_regular(
                    OrderSide::Sell,
                    RegularOrderType::Market,
                    buy.lots_executed,
                    None,
                )
                .await?;
            self.state.net_open_lots -= sell.lots_executed;
            if let Some(id) = sell.broker_order_id.as_deref()
                && !terminal(sell.execution_status)
            {
                self.cancel_broker_order(id).await?;
            }
        }
        if self.state.net_open_lots != 0 {
            return Err(failure("market round trip did not restore flat position"));
        }
        Ok(format!(
            "market order acknowledged; executed lots={} and exposure flattened",
            buy.lots_executed
        ))
    }

    async fn limit_order_lifecycle(&mut self) -> Result<String, BoxError> {
        let (_, instrument) = self.owned_context()?;
        let price = far_below(&instrument)?;
        let order = self
            .post_regular(OrderSide::Buy, RegularOrderType::Limit, 1, Some(price))
            .await?;
        if order.lots_executed != 0 {
            self.state.net_open_lots += order.lots_executed;
            return Err(failure(
                "far limit unexpectedly executed; cleanup must flatten it",
            ));
        }
        self.state.limit_order_id = order.broker_order_id.clone();
        self.state.limit_request_id = order.client_request_id.clone();
        Ok(format!(
            "limit accepted as broker={} request={}",
            order.broker_order_id.as_deref().unwrap_or("missing"),
            order.client_request_id.as_deref().unwrap_or("missing")
        ))
    }

    async fn order_state_and_list(&self) -> Result<String, BoxError> {
        let (account, _) = self.context()?;
        let broker_id = self
            .state
            .limit_order_id
            .as_deref()
            .ok_or_else(|| failure("limit order unavailable"))?;
        let point = self
            .wait_for_order(account, broker_id, ProviderOrderIdentityKind::BrokerOrder)
            .await?;
        let listed = canonical_orders(
            self.client
                .get_sandbox_orders(v1::GetOrdersRequest {
                    account_id: account.to_owned(),
                    advanced_filters: None,
                })
                .await?
                .body,
        )?;
        if point.broker_order_id.as_deref() != Some(broker_id)
            || !listed
                .iter()
                .any(|order| order.broker_order_id.as_deref() == Some(broker_id))
        {
            return Err(failure("point/list order identity mismatch"));
        }
        Ok("point state and active list preserve broker identity".to_owned())
    }

    async fn replace_lifecycle(&mut self) -> Result<String, BoxError> {
        let (account, instrument) = self.owned_context()?;
        let existing = self
            .state
            .limit_order_id
            .clone()
            .ok_or_else(|| failure("limit order unavailable"))?;
        let replacement_id = logical_id();
        let request = replace_order_request(&ReplaceOrderCommand {
            account_id: account,
            existing_order_id: existing,
            existing_order_id_kind: Some(ProviderOrderIdentityKind::BrokerOrder),
            replacement_request_id: replacement_id.clone(),
            quantity_lots: 1,
            price: one_tick_higher(far_below(&instrument)?, instrument.tick)?,
            confirm_margin_trade: false,
        })?;
        let response = self
            .dispatcher
            .replace_order(
                &self.client,
                authorization()?,
                ExecutionRoute::Sandbox,
                request,
            )
            .await?;
        self.state.logical_order_ids.insert(replacement_id);
        self.state.limit_order_id = nonempty(response.response.body.order_id);
        let broker_id = self
            .state
            .limit_order_id
            .as_deref()
            .ok_or_else(|| failure("ReplaceSandboxOrder omitted broker identity"))?;
        Ok(format!("replace acknowledged as broker={broker_id}"))
    }

    async fn cancel_lifecycle(&mut self) -> Result<String, BoxError> {
        let broker_id = self
            .state
            .limit_order_id
            .clone()
            .ok_or_else(|| failure("replaced limit order unavailable"))?;
        self.cancel_broker_order(&broker_id).await?;
        let (account, _) = self.context()?;
        let active = canonical_orders(
            self.client
                .get_sandbox_orders(v1::GetOrdersRequest {
                    account_id: account.to_owned(),
                    advanced_filters: None,
                })
                .await?
                .body,
        )?;
        if active
            .iter()
            .any(|order| order.broker_order_id.as_deref() == Some(&broker_id))
        {
            return Err(failure("cancelled limit remains active"));
        }
        self.state.limit_order_id = None;
        Ok("cancel acknowledged and order absent from active list".to_owned())
    }

    async fn async_order_lifecycle(&mut self) -> Result<String, BoxError> {
        let (account, instrument) = self.owned_context()?;
        let request_id = logical_id();
        let request = async_regular_order_request(&RegularOrderCommand {
            account_id: account.clone(),
            instrument_id: instrument.uid.clone(),
            client_request_id: request_id.clone(),
            quantity_lots: 1,
            price: Some(far_below(&instrument)?),
            side: OrderSide::Buy,
            order_type: RegularOrderType::Limit,
            time_in_force: Some(TimeInForce::Day),
            confirm_margin_trade: false,
        })?;
        self.dispatcher
            .post_order_async(
                &self.client,
                authorization()?,
                ExecutionRoute::Sandbox,
                request,
            )
            .await?;
        self.state.logical_order_ids.insert(request_id.clone());
        let mut state = None;
        let mut last_point_error = None;
        for _ in 0..20 {
            match self
                .client
                .get_sandbox_order_state(v1::GetOrderStateRequest {
                    account_id: account.clone(),
                    order_id: request_id.clone(),
                    price_type: v1::PriceType::Unspecified as i32,
                    order_id_type: Some(v1::OrderIdType::Request as i32),
                })
                .await
            {
                Ok(response) => {
                    state = Some(response.body.try_into()?);
                    break;
                }
                Err(error) => last_point_error = Some(error.to_string()),
            }
            let listed = canonical_orders(
                self.client
                    .get_sandbox_orders(v1::GetOrdersRequest {
                        account_id: account.clone(),
                        advanced_filters: None,
                    })
                    .await?
                    .body,
            )?;
            state = listed
                .into_iter()
                .find(|order| order.client_request_id.as_deref() == Some(&request_id));
            if state.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let state = state.ok_or_else(|| {
            failure(format!(
                "async acknowledgement did not materialize by request identity: {}",
                last_point_error.unwrap_or_else(|| "no provider response".to_owned())
            ))
        })?;
        let broker_id = state
            .broker_order_id
            .ok_or_else(|| failure("async order readback omitted broker identity"))?;
        if state.lots_executed != 0 {
            self.state.net_open_lots += state.lots_executed;
            return Err(failure("far async limit unexpectedly executed"));
        }
        self.cancel_broker_order(&broker_id).await?;
        Ok(format!(
            "async intent reconciled request={request_id} broker={broker_id}"
        ))
    }

    async fn broker_idempotency_evidence(&mut self) -> Result<String, BoxError> {
        let (account, instrument) = self.owned_context()?;
        let request_id = logical_id();
        let request = regular_order_request(&RegularOrderCommand {
            account_id: account.clone(),
            instrument_id: instrument.uid.clone(),
            client_request_id: request_id.clone(),
            quantity_lots: 1,
            price: Some(far_below(&instrument)?),
            side: OrderSide::Buy,
            order_type: RegularOrderType::Limit,
            time_in_force: Some(TimeInForce::Day),
            confirm_margin_trade: false,
        })?;
        let first = self
            .client
            .post_sandbox_order(authorization()?, request.clone())
            .await?
            .body;
        let broker_id = nonempty(first.order_id)
            .ok_or_else(|| failure("idempotency response omitted broker identity"))?;
        match self
            .client
            .post_sandbox_order(authorization()?, request)
            .await
        {
            Err(error)
                if matches!(
                    &error.kind,
                    GrpcErrorKind::Provider(provider) if provider.has_provider_code("30057")
                ) => {}
            Err(error) => {
                return Err(failure(format!(
                    "duplicate replay returned unexpected provider result: {error}"
                )));
            }
            Ok(_) => {
                return Err(failure(
                    "duplicate replay unexpectedly returned success instead of provider 30057",
                ));
            }
        }
        self.state.logical_order_ids.insert(request_id);
        let listed = canonical_orders(
            self.client
                .get_sandbox_orders(v1::GetOrdersRequest {
                    account_id: account.clone(),
                    advanced_filters: None,
                })
                .await?
                .body,
        )?;
        if listed
            .iter()
            .filter(|order| order.broker_order_id.as_deref() == Some(&broker_id))
            .count()
            != 1
        {
            return Err(failure(
                "provider 30057 was not backed by exactly one authoritative active order",
            ));
        }
        self.cancel_broker_order(&broker_id).await?;
        Ok(format!(
            "duplicate replay returned documented 30057; one broker order {broker_id}"
        ))
    }

    fn ambiguous_dispatch_fault(&self) -> Result<String, BoxError> {
        qualify_ambiguous_dispatch_guard()?;
        Ok("persisted UNKNOWN blocks duplicate dispatch until reconciliation".to_owned())
    }

    async fn protection_lifecycle(&mut self, plan: ProtectionPlan) -> Result<String, BoxError> {
        let (account, instrument) = self.owned_context()?;
        let requests = protection_requests(
            &plan,
            &ProtectionRequestContext {
                account_id: account.clone(),
                instrument_id: instrument.uid,
                quantity_lots: 1,
                position_side: PositionSide::Long,
                expire_at: Some(ProviderTimestamp {
                    seconds: time::OffsetDateTime::now_utc().unix_timestamp() + 3_600,
                    nanos: 0,
                }),
                confirm_margin_trade: false,
                request_ids: ProtectionRequestIds {
                    stop_loss: Some(logical_id()),
                    take_profit: Some(logical_id()),
                },
            },
        )?;
        let expected = requests.len();
        let mut broker_ids = Vec::with_capacity(expected);
        for request in requests {
            let response = self
                .dispatcher
                .post_stop_order(
                    &self.client,
                    authorization()?,
                    ExecutionRoute::Sandbox,
                    request,
                )
                .await?;
            broker_ids.push(
                nonempty(response.response.body.stop_order_id)
                    .ok_or_else(|| failure("PostSandboxStopOrder omitted broker identity"))?,
            );
        }
        let active = canonical_stop_orders(
            self.client
                .get_sandbox_stop_orders(active_stops_request(&account))
                .await?
                .body,
        )?;
        if broker_ids.iter().any(|id| {
            !active
                .iter()
                .any(|stop| stop.broker_stop_order_id.as_deref() == Some(id))
        }) {
            return Err(failure(
                "posted protection absent from authoritative active list",
            ));
        }
        for broker_id in &broker_ids {
            let request = cancel_stop_order_request(&CancelStopOrderCommand {
                account_id: account.clone(),
                broker_stop_order_id: broker_id.clone(),
            })?;
            self.dispatcher
                .cancel_stop_order(
                    &self.client,
                    authorization()?,
                    ExecutionRoute::Sandbox,
                    ClientRequestId::new(logical_id())?,
                    request,
                )
                .await?;
        }
        let remaining = canonical_stop_orders(
            self.client
                .get_sandbox_stop_orders(active_stops_request(&account))
                .await?
                .body,
        )?;
        if broker_ids.iter().any(|id| {
            remaining
                .iter()
                .any(|stop| stop.broker_stop_order_id.as_deref() == Some(id))
        }) {
            return Err(failure("cancelled protection remains active"));
        }
        Ok(format!(
            "{expected} protection leg(s) posted, read back, cancelled"
        ))
    }

    async fn protection_variant(&mut self, variant: ProtectionVariant) -> Result<String, BoxError> {
        let (_, instrument) = self.owned_context()?;
        self.protection_lifecycle(protection_plan(&instrument, variant)?)
            .await
    }

    async fn stream_health(&self, kind: ExecutionStreamKind) -> Result<String, BoxError> {
        let (account, _) = self.context()?;
        let supervisor = ExecutionStreamSupervisor::new(
            self.client.clone(),
            ExecutionStreamConfig {
                event_capacity: 32,
                stale_timeout: Duration::from_secs(20),
                ping_delay_ms: 5_000,
                ..ExecutionStreamConfig::default()
            },
        )?;
        let mut handle = supervisor.start(kind, vec![account.to_owned()])?;
        let result = tokio::time::timeout(Duration::from_secs(25), async {
            loop {
                match handle.recv().await {
                    Some(ExecutionStreamEvent::Evidence(
                        vox_tinvest::execution::CanonicalExecutionStreamEvent::Subscription {
                            accounts,
                            provider_error_code,
                            ..
                        },
                    )) if accounts.iter().any(|value| value == account)
                        && provider_error_code.is_none() =>
                    {
                        return Ok::<_, BoxError>(());
                    }
                    Some(ExecutionStreamEvent::Fault(error)) => return Err(Box::new(error).into()),
                    Some(ExecutionStreamEvent::Stopped) | None => {
                        return Err(failure("execution stream stopped before subscription ACK"));
                    }
                    _ => {}
                }
            }
        })
        .await;
        handle.stop();
        match result {
            Ok(Ok(())) => Ok(format!(
                "{kind:?} subscription ACK received for sandbox account"
            )),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(failure("execution stream subscription ACK timed out")),
        }
    }

    async fn cleanup(&mut self) -> Result<String, BoxError> {
        let (account, _) = self.owned_context()?;
        let active_orders = self
            .client
            .get_sandbox_orders(v1::GetOrdersRequest {
                account_id: account.clone(),
                advanced_filters: None,
            })
            .await?
            .body
            .orders;
        let mut cleanup_errors = Vec::new();
        for order in active_orders {
            if !self.state.baseline_order_ids.contains(&order.order_id) {
                let request = v1::CancelOrderRequest {
                    account_id: account.clone(),
                    order_id: order.order_id.clone(),
                    order_id_type: Some(v1::OrderIdType::Exchange as i32),
                };
                if let Err(error) = self
                    .client
                    .cancel_sandbox_order(authorization()?, request)
                    .await
                {
                    cleanup_errors.push(format!("order {} cancel: {error}", order.order_id));
                }
            }
        }
        let active_stops = self
            .client
            .get_sandbox_stop_orders(active_stops_request(&account))
            .await?
            .body
            .stop_orders;
        for stop in active_stops {
            if !self.state.baseline_stop_ids.contains(&stop.stop_order_id) {
                let request = v1::CancelStopOrderRequest {
                    account_id: account.clone(),
                    stop_order_id: stop.stop_order_id.clone(),
                };
                if let Err(error) = self
                    .client
                    .cancel_sandbox_stop_order(authorization()?, request)
                    .await
                {
                    cleanup_errors.push(format!("stop {} cancel: {error}", stop.stop_order_id));
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        let (_, instrument) = self.owned_context()?;
        let baseline_balance = self
            .state
            .baseline_instrument_balance
            .ok_or_else(|| failure("baseline instrument balance unavailable"))?;
        let current_balance = self.instrument_balance(&account, &instrument.uid).await?;
        let balance_delta = current_balance - baseline_balance;
        if balance_delta != 0 {
            let side = if balance_delta > 0 {
                OrderSide::Sell
            } else {
                OrderSide::Buy
            };
            let units = balance_delta.unsigned_abs();
            let lot_size = u64::try_from(instrument.lot_size)?;
            if units % lot_size != 0 {
                cleanup_errors.push(format!(
                    "position delta {balance_delta} is not divisible by lot size {}",
                    instrument.lot_size
                ));
            }
            let quantity = units / lot_size;
            match i64::try_from(quantity) {
                Ok(quantity) => match self
                    .post_regular(side, RegularOrderType::Market, quantity, None)
                    .await
                {
                    Ok(state) => {
                        if side == OrderSide::Sell {
                            self.state.net_open_lots -= state.lots_executed;
                        } else {
                            self.state.net_open_lots += state.lots_executed;
                        }
                    }
                    Err(error) => cleanup_errors.push(format!("exposure flatten: {error}")),
                },
                Err(error) => cleanup_errors.push(format!("exposure quantity: {error}")),
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        let remaining_orders = self
            .client
            .get_sandbox_orders(v1::GetOrdersRequest {
                account_id: account.clone(),
                advanced_filters: None,
            })
            .await?
            .body
            .orders
            .into_iter()
            .filter(|order| !self.state.baseline_order_ids.contains(&order.order_id))
            .map(|order| order.order_id)
            .collect::<Vec<_>>();
        let remaining_stops = self
            .client
            .get_sandbox_stop_orders(active_stops_request(&account))
            .await?
            .body
            .stop_orders
            .into_iter()
            .filter(|stop| !self.state.baseline_stop_ids.contains(&stop.stop_order_id))
            .map(|stop| stop.stop_order_id)
            .collect::<Vec<_>>();
        let final_balance = self.instrument_balance(&account, &instrument.uid).await?;
        if !remaining_orders.is_empty()
            || !remaining_stops.is_empty()
            || final_balance != baseline_balance
            || !cleanup_errors.is_empty()
        {
            return Err(failure(format!(
                "cleanup incomplete: orders={remaining_orders:?}; stops={remaining_stops:?}; balance={final_balance}; baseline={baseline_balance}; errors={cleanup_errors:?}",
            )));
        }
        Ok("all qualification-created orders/stops absent; net lots zero".to_owned())
    }
}

fn failure(message: impl Into<String>) -> BoxError {
    Box::new(io::Error::other(message.into()))
}

fn nonempty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn fixed(value: v1::Quotation) -> Result<FixedPoint, BoxError> {
    Ok(FixedPoint::from_units_nano(value.units, value.nano)?)
}

fn quotation(value: FixedPoint) -> Result<v1::Quotation, BoxError> {
    let (units, nano) = value.units_nano();
    Ok(v1::Quotation {
        units: i64::try_from(units).map_err(|_| failure("price units exceed int64"))?,
        nano,
    })
}

fn active_stops_request(account_id: &str) -> v1::GetStopOrdersRequest {
    v1::GetStopOrdersRequest {
        account_id: account_id.to_owned(),
        status: v1::StopOrderStatusOption::StopOrderStatusActive as i32,
        from: None,
        to: None,
    }
}

fn authorization() -> Result<MutationAuthorization, BoxError> {
    Ok(MutationGuard::new(Environment::Sandbox).authorize_mutation()?)
}

fn logical_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn terminal(status: i32) -> bool {
    matches!(
        v1::OrderExecutionReportStatus::try_from(status),
        Ok(v1::OrderExecutionReportStatus::ExecutionReportStatusFill)
            | Ok(v1::OrderExecutionReportStatus::ExecutionReportStatusRejected)
            | Ok(v1::OrderExecutionReportStatus::ExecutionReportStatusCancelled)
    )
}

fn far_below(instrument: &InstrumentFixture) -> Result<FixedPoint, BoxError> {
    let ticks = instrument.current_price.total_nanos() / instrument.tick.total_nanos();
    if ticks < 4 {
        return Err(failure(
            "instrument price has insufficient positive tick range",
        ));
    }
    Ok(FixedPoint::from_total_nanos(
        (ticks / 2) * instrument.tick.total_nanos(),
    ))
}

fn one_tick_higher(value: FixedPoint, tick: FixedPoint) -> Result<FixedPoint, BoxError> {
    let total = value
        .total_nanos()
        .checked_add(tick.total_nanos())
        .ok_or_else(|| failure("price addition overflow"))?;
    Ok(FixedPoint::from_total_nanos(total))
}

fn ticks_from(value: FixedPoint, tick: FixedPoint, ticks: i128) -> Result<FixedPoint, BoxError> {
    let delta = tick
        .total_nanos()
        .checked_mul(ticks)
        .ok_or_else(|| failure("tick multiplication overflow"))?;
    let total = value
        .total_nanos()
        .checked_add(delta)
        .ok_or_else(|| failure("price addition overflow"))?;
    if total <= 0 {
        return Err(failure("derived protection price is non-positive"));
    }
    Ok(FixedPoint::from_total_nanos(total))
}

fn percentage_price(
    instrument: &InstrumentFixture,
    percentage: i128,
) -> Result<FixedPoint, BoxError> {
    let price_ticks = instrument.current_price.total_nanos() / instrument.tick.total_nanos();
    let scaled_ticks = price_ticks
        .checked_mul(percentage)
        .and_then(|value| value.checked_div(100))
        .ok_or_else(|| failure("percentage price overflow"))?;
    if scaled_ticks <= 0 {
        return Err(failure("percentage price is non-positive"));
    }
    Ok(FixedPoint::from_total_nanos(
        scaled_ticks * instrument.tick.total_nanos(),
    ))
}

#[derive(Clone, Copy)]
enum ProtectionVariant {
    Fixed,
    StopLimit,
    TakeProfit,
    TrailingRelative,
    TrailingAbsolute,
    FixedAndTakeProfit,
    TrailingAndTakeProfit,
}

fn protection_plan(
    instrument: &InstrumentFixture,
    variant: ProtectionVariant,
) -> Result<ProtectionPlan, BoxError> {
    let below = percentage_price(instrument, 95)?;
    let lower_limit = ticks_from(below, instrument.tick, -1)?;
    let above = percentage_price(instrument, 105)?;
    let fixed = StopLossProtection::Fixed {
        trigger_price: below,
        limit_price: None,
    };
    let stop_limit = StopLossProtection::Fixed {
        trigger_price: below,
        limit_price: Some(lower_limit),
    };
    let trailing_relative = StopLossProtection::Trailing {
        distance: TrailingDistance {
            value: FixedPoint::from_units_nano(1, 0)?,
            mode: TrailingDistanceMode::RelativePercent,
        },
        activation_price: None,
        protective_spread: None,
        instant_execution: Some(true),
    };
    let trailing_absolute = StopLossProtection::Trailing {
        distance: TrailingDistance {
            value: percentage_price(instrument, 5)?,
            mode: TrailingDistanceMode::AbsolutePrice,
        },
        activation_price: None,
        protective_spread: None,
        instant_execution: Some(true),
    };
    let take_profit = TakeProfitProtection {
        trigger_price: Some(above),
        limit_price: None,
        trailing: None,
    };
    Ok(match variant {
        ProtectionVariant::Fixed => ProtectionPlan {
            stop_loss: Some(fixed),
            take_profit: None,
        },
        ProtectionVariant::StopLimit => ProtectionPlan {
            stop_loss: Some(stop_limit),
            take_profit: None,
        },
        ProtectionVariant::TakeProfit => ProtectionPlan {
            stop_loss: None,
            take_profit: Some(take_profit),
        },
        ProtectionVariant::TrailingRelative => ProtectionPlan {
            stop_loss: Some(trailing_relative),
            take_profit: None,
        },
        ProtectionVariant::TrailingAbsolute => ProtectionPlan {
            stop_loss: Some(trailing_absolute),
            take_profit: None,
        },
        ProtectionVariant::FixedAndTakeProfit => ProtectionPlan {
            stop_loss: Some(fixed),
            take_profit: Some(take_profit),
        },
        ProtectionVariant::TrailingAndTakeProfit => ProtectionPlan {
            stop_loss: Some(trailing_relative),
            take_profit: Some(take_profit),
        },
    })
}

fn evidence(result: Result<String, BoxError>) -> QualificationEvidence {
    match result {
        Ok(detail) => QualificationEvidence::Qualified(detail),
        Err(error) => QualificationEvidence::Failed(error.to_string()),
    }
}

#[tokio::test]
#[ignore = "requires TINVEST_SANDBOX_TOKEN and mutates only T-Invest sandbox"]
async fn complete_execution_surface_qualifies_in_sandbox() -> Result<(), BoxError> {
    let token = std::env::var("TINVEST_SANDBOX_TOKEN")
        .map_err(|_| failure("TINVEST_SANDBOX_TOKEN is required"))?;
    let client = TInvestGrpcClient::sandbox(GrpcCredential::Sandbox(SecretToken::new(token)?))?;
    let mut runner = SandboxRunner::new(client)?;
    let mut ledger = SandboxQualificationLedger::default();

    macro_rules! qualify {
        ($row:literal, $result:expr) => {
            ledger.record($row, evidence($result))?;
        };
    }

    qualify!(
        "account_discovery_readiness",
        runner.account_readiness().await
    );
    qualify!("max_lots", runner.max_lots().await);
    qualify!("pre_trade_estimate", runner.pre_trade_estimate().await);
    qualify!(
        "market_order_lifecycle",
        runner.market_order_lifecycle().await
    );
    qualify!(
        "limit_order_lifecycle",
        runner.limit_order_lifecycle().await
    );
    qualify!(
        "async_order_lifecycle",
        runner.async_order_lifecycle().await
    );
    qualify!("order_state_and_list", runner.order_state_and_list().await);
    qualify!("replace_lifecycle", runner.replace_lifecycle().await);
    qualify!("cancel_lifecycle", runner.cancel_lifecycle().await);
    qualify!(
        "broker_idempotency_evidence",
        runner.broker_idempotency_evidence().await
    );
    qualify!(
        "ambiguous_dispatch_fault_injection",
        runner.ambiguous_dispatch_fault()
    );
    qualify!(
        "fixed_stop_only",
        runner.protection_variant(ProtectionVariant::Fixed).await
    );
    qualify!(
        "stop_limit",
        runner
            .protection_variant(ProtectionVariant::StopLimit)
            .await
    );
    qualify!(
        "take_profit_only",
        runner
            .protection_variant(ProtectionVariant::TakeProfit)
            .await
    );
    qualify!(
        "trailing_relative",
        runner
            .protection_variant(ProtectionVariant::TrailingRelative)
            .await
    );
    qualify!(
        "trailing_absolute",
        runner
            .protection_variant(ProtectionVariant::TrailingAbsolute)
            .await
    );
    qualify!(
        "fixed_stop_plus_take_profit",
        runner
            .protection_variant(ProtectionVariant::FixedAndTakeProfit)
            .await
    );
    qualify!(
        "trailing_plus_take_profit",
        runner
            .protection_variant(ProtectionVariant::TrailingAndTakeProfit)
            .await
    );
    qualify!(
        "trades_stream_health",
        runner.stream_health(ExecutionStreamKind::Trades).await
    );
    qualify!(
        "order_state_stream_health",
        runner.stream_health(ExecutionStreamKind::OrderState).await
    );
    qualify!("cleanup_readback", runner.cleanup().await);

    for line in ledger.lines()? {
        println!("{line}");
    }
    ledger.finish()?;
    Ok(())
}
