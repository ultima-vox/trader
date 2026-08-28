use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{Mutex, mpsc};
use vox_domain::{
    Environment, ExecutionPriceConvention, FixedPoint, MutationGuard, OrderSide,
    RegularOrderCommand, RegularOrderType, TimeInForce,
};
use vox_runtime::{
    BrokerAccount, BrokerEvent, BrokerEventClass, BrokerIdentityLinks, BrokerPortError,
    BrokerReadPort, BrokerResultClass, CredentialResolution, CredentialResolverPort, ExecutionPort,
    ExecutionResult, ExecutionStreamPort, HealthReadPort, InMemoryMetrics, JournalState,
    MutationKind, MutationRecord, OpaqueRef, OperationFact, OperationsPage, OrderFact,
    PortfolioFact, Provider, ReconciliationConfig, RuntimeConfig, RuntimeCoordinator,
    RuntimeEnvironment, RuntimeScope, RuntimeState, RuntimeStore, SqliteRuntimeStore, StopFact,
    StreamKind, StreamSignal,
};
use vox_tinvest::account::AccountReadClient;
use vox_tinvest::execution::{
    CanonicalExecutionStreamEvent, canonical_orders, canonical_stop_orders, regular_order_request,
};
use vox_tinvest::execution_stream::{
    ExecutionStreamConfig, ExecutionStreamEvent, ExecutionStreamKind, ExecutionStreamSupervisor,
};
use vox_tinvest::generated::v1;
use vox_tinvest::{GrpcCredential, GrpcError, GrpcErrorKind, SecretToken, TInvestGrpcClient};

type BoxError = Box<dyn Error + Send + Sync>;
const RUNTIME_QUALIFICATION_ROWS: [&str; 16] = [
    "runtime_startup_ownership",
    "connecting",
    "initial_authoritative_reconciliation",
    "ready_gate",
    "durable_mutation_broker_execution_identity_link",
    "restart_with_broker_visible_open_state",
    "unknown_restart_authoritative_resolution_no_replay",
    "position_snapshot_reconciliation",
    "regular_order_reconciliation",
    "stop_protection_reconciliation",
    "duplicate_event_readback_dedupe",
    "stream_gap_reconciliation_recovery",
    "graceful_shutdown_restart",
    "cleanup_readback",
    "zero_unresolved_unknown",
    "resource_queue_summary",
];
type LiveCoordinator = RuntimeCoordinator<
    SandboxReads,
    SandboxExecution,
    SandboxStreams,
    SandboxCredential,
    SqliteRuntimeStore,
    InMemoryMetrics,
>;

#[derive(Clone)]
struct SandboxReads {
    client: TInvestGrpcClient,
    snapshot_starts: Arc<AtomicU64>,
}

#[async_trait]
impl BrokerReadPort for SandboxReads {
    async fn accounts(&self, _: &RuntimeScope) -> Result<Vec<BrokerAccount>, BrokerPortError> {
        self.snapshot_starts.fetch_add(1, Ordering::SeqCst);
        let catalogue = AccountReadClient::new(self.client.clone())
            .sandbox_accounts()
            .await
            .map_err(|error| port_error("UsersService", "GetSandboxAccounts", error))?;
        Ok(catalogue
            .accounts
            .into_iter()
            .map(|account| BrokerAccount {
                account_id: account.account_id,
                open: account.status == v1::AccountStatus::Open as i32,
                accessible: account.status == v1::AccountStatus::Open as i32,
            })
            .collect())
    }

    async fn portfolio(&self, scope: &RuntimeScope) -> Result<PortfolioFact, BrokerPortError> {
        let portfolio = AccountReadClient::new(self.client.clone())
            .sandbox_portfolio(scope.broker_account_id.clone())
            .await
            .map_err(|error| port_error("OperationsService", "GetSandboxPortfolio", error))?;
        let mut currencies = BTreeMap::new();
        for (name, value) in [
            ("portfolio", portfolio.total_amount_portfolio),
            ("currencies", portfolio.total_amount_currencies),
        ] {
            if let Some(value) = value {
                currencies.insert(
                    value.currency.unwrap_or_else(|| name.to_owned()),
                    format!("{:?}", value.amount),
                );
            }
        }
        Ok(PortfolioFact {
            account_id: portfolio
                .account_id
                .unwrap_or_else(|| scope.broker_account_id.clone()),
            currencies,
            broker_observed_at_unix_ms: None,
        })
    }

    async fn positions(
        &self,
        scope: &RuntimeScope,
    ) -> Result<Vec<vox_runtime::PositionFact>, BrokerPortError> {
        let positions = AccountReadClient::new(self.client.clone())
            .sandbox_positions(scope.broker_account_id.clone())
            .await
            .map_err(|error| port_error("OperationsService", "GetSandboxPositions", error))?;
        let mut facts = Vec::new();
        for position in positions.securities {
            if let Some(instrument_uid) = position.identity.instrument_uid {
                facts.push(position_fact(scope, instrument_uid, position.balance));
            }
        }
        for position in positions.futures {
            if let Some(instrument_uid) = position.identity.instrument_uid {
                facts.push(position_fact(scope, instrument_uid, position.balance));
            }
        }
        for position in positions.options {
            if let Some(instrument_uid) = position.identity.instrument_uid {
                facts.push(position_fact(scope, instrument_uid, position.balance));
            }
        }
        Ok(facts)
    }

    async fn active_orders(&self, scope: &RuntimeScope) -> Result<Vec<OrderFact>, BrokerPortError> {
        let response = self
            .client
            .get_sandbox_orders(v1::GetOrdersRequest {
                account_id: scope.broker_account_id.clone(),
                advanced_filters: None,
            })
            .await
            .map_err(|error| grpc_port_error("OrdersService", "GetSandboxOrders", error))?;
        canonical_orders(response.body)
            .map_err(|error| port_error("OrdersService", "GetSandboxOrders", error))?
            .into_iter()
            .map(|order| order_fact(scope, order))
            .collect()
    }

    async fn stop_orders(
        &self,
        scope: &RuntimeScope,
        include_terminal_since_unix_ms: i64,
    ) -> Result<Vec<StopFact>, BrokerPortError> {
        let response = self
            .client
            .get_sandbox_stop_orders(v1::GetStopOrdersRequest {
                account_id: scope.broker_account_id.clone(),
                status: v1::StopOrderStatusOption::StopOrderStatusAll as i32,
                from: Some(timestamp(include_terminal_since_unix_ms)),
                to: None,
            })
            .await
            .map_err(|error| grpc_port_error("StopOrdersService", "GetSandboxStopOrders", error))?;
        canonical_stop_orders(response.body)
            .map_err(|error| port_error("StopOrdersService", "GetSandboxStopOrders", error))?
            .into_iter()
            .map(|stop| {
                let id = stop.broker_stop_order_id.ok_or_else(|| BrokerPortError {
                    service: "StopOrdersService",
                    method: "GetSandboxStopOrders",
                    class: BrokerResultClass::Permanent,
                    message: "stop omitted broker identity".into(),
                    retry_after: None,
                })?;
                let active = stop.status == v1::StopOrderStatusOption::StopOrderStatusActive as i32;
                Ok(StopFact {
                    account_id: scope.broker_account_id.clone(),
                    broker_stop_order_id: id,
                    logical_request_id: None,
                    instrument_uid: stop.instrument_uid.unwrap_or_default(),
                    active,
                    terminal: !active,
                })
            })
            .collect()
    }

    async fn order_state(
        &self,
        scope: &RuntimeScope,
        broker_order_id: Option<&str>,
        logical_request_id: Option<&str>,
    ) -> Result<Option<OrderFact>, BrokerPortError> {
        let (order_id, order_id_type) = match (broker_order_id, logical_request_id) {
            (Some(value), _) => (value, v1::OrderIdType::Exchange),
            (None, Some(value)) => (value, v1::OrderIdType::Request),
            (None, None) => return Ok(None),
        };
        let response = self
            .client
            .get_sandbox_order_state(v1::GetOrderStateRequest {
                account_id: scope.broker_account_id.clone(),
                order_id: order_id.to_owned(),
                price_type: v1::PriceType::Currency as i32,
                order_id_type: Some(order_id_type as i32),
            })
            .await;
        match response {
            Ok(response) => response
                .body
                .try_into()
                .map_err(|error| port_error("OrdersService", "GetSandboxOrderState", error))
                .and_then(|order| order_fact(scope, order).map(Some)),
            Err(error) if provider_not_found(&error) => Ok(None),
            Err(error) => Err(grpc_port_error(
                "OrdersService",
                "GetSandboxOrderState",
                error,
            )),
        }
    }

    async fn operations_page(
        &self,
        scope: &RuntimeScope,
        cursor: Option<&str>,
        from_unix_ms: i64,
        limit: u16,
    ) -> Result<OperationsPage, BrokerPortError> {
        let response = self
            .client
            .get_sandbox_operations_by_cursor(v1::GetOperationsByCursorRequest {
                account_id: scope.broker_account_id.clone(),
                instrument_id: None,
                from: Some(timestamp(from_unix_ms)),
                to: None,
                cursor: cursor.map(str::to_owned),
                limit: Some(i32::from(limit)),
                operation_types: Vec::new(),
                state: None,
                without_commissions: None,
                without_trades: None,
                without_overnights: None,
            })
            .await
            .map_err(|error| {
                grpc_port_error("OperationsService", "GetSandboxOperationsByCursor", error)
            })?
            .body;
        let next_cursor = response.has_next.then_some(response.next_cursor);
        let items = response
            .items
            .into_iter()
            .enumerate()
            .map(|(index, item)| {
                let operation: vox_tinvest::operations::CanonicalOperation =
                    item.try_into().map_err(|error| {
                        port_error("OperationsService", "GetSandboxOperationsByCursor", error)
                    })?;
                Ok(OperationFact {
                    account_id: operation
                        .broker_account_id
                        .unwrap_or_else(|| scope.broker_account_id.clone()),
                    cursor: operation
                        .cursor
                        .unwrap_or_else(|| format!("page-item-{index}")),
                    provider_operation_id: operation.provider_operation_id,
                    broker_order_id: None,
                    logical_request_id: None,
                    broker_fill_ids: operation
                        .trades
                        .into_iter()
                        .filter_map(|trade| trade.trade_id)
                        .collect(),
                })
            })
            .collect::<Result<Vec<_>, BrokerPortError>>()?;
        Ok(OperationsPage { items, next_cursor })
    }
}

struct SandboxExecution {
    client: TInvestGrpcClient,
    instrument_uid: String,
    limit_price: FixedPoint,
}

#[async_trait]
impl ExecutionPort for SandboxExecution {
    async fn dispatch_once(
        &self,
        scope: &RuntimeScope,
        mutation: &MutationRecord,
    ) -> Result<ExecutionResult, BrokerPortError> {
        if mutation.kind != MutationKind::PostOrder {
            return Ok(ExecutionResult::Rejected {
                broker_evidence_ref: "runtime-live:unsupported-kind".into(),
            });
        }
        let request = regular_order_request(&RegularOrderCommand {
            account_id: scope.broker_account_id.clone(),
            instrument_id: self.instrument_uid.clone(),
            client_request_id: mutation.logical_request_id.clone(),
            quantity_lots: 1,
            price: Some(self.limit_price),
            price_convention: ExecutionPriceConvention::SettlementCurrency,
            side: OrderSide::Buy,
            order_type: RegularOrderType::Limit,
            time_in_force: Some(TimeInForce::Day),
            confirm_margin_trade: false,
        })
        .map_err(|error| port_error("OrdersService", "PostSandboxOrder", error))?;
        let response = self
            .client
            .post_sandbox_order(
                MutationGuard::new(Environment::Sandbox)
                    .authorize_mutation()
                    .map_err(|error| port_error("Runtime", "authorize_mutation", error))?,
                request,
            )
            .await
            .map_err(|error| grpc_port_error("OrdersService", "PostSandboxOrder", error))?;
        let broker_order_id = nonempty(response.body.order_id).ok_or_else(|| BrokerPortError {
            service: "OrdersService",
            method: "PostSandboxOrder",
            class: BrokerResultClass::Permanent,
            message: "acknowledgement omitted broker order identity".into(),
            retry_after: None,
        })?;
        let evidence = format!(
            "request_id={}; broker_order_id={broker_order_id}",
            response.metadata.request_id
        );
        if mutation.redacted_request_evidence.contains("fault=unknown") {
            Ok(ExecutionResult::UnknownAfterDispatch {
                broker_evidence_ref: Some(evidence),
            })
        } else {
            Ok(ExecutionResult::Acknowledged {
                broker_evidence_ref: evidence,
                links: BrokerIdentityLinks {
                    logical_request_id: mutation.logical_request_id.clone(),
                    broker_order_id: Some(broker_order_id),
                    ..BrokerIdentityLinks::default()
                },
            })
        }
    }
}

struct SandboxStreams {
    client: TInvestGrpcClient,
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    output: Mutex<Option<mpsc::Sender<StreamSignal>>>,
}

impl SandboxStreams {
    fn new(client: TInvestGrpcClient) -> Self {
        Self {
            client,
            tasks: Mutex::new(Vec::new()),
            output: Mutex::new(None),
        }
    }

    async fn force_gap(&self) -> Result<(), BoxError> {
        self.output
            .lock()
            .await
            .clone()
            .ok_or("runtime stream output unavailable")?
            .send(StreamSignal::Gap {
                stream: StreamKind::OrderState,
                reason: "qualification-forced-gap".into(),
            })
            .await?;
        Ok(())
    }

    async fn force_external_position_signal(
        &self,
        account_id: &str,
        runtime_epoch: u64,
    ) -> Result<(), BoxError> {
        self.output
            .lock()
            .await
            .clone()
            .ok_or("runtime stream output unavailable")?
            .send(StreamSignal::Event(BrokerEvent {
                account_id: account_id.to_owned(),
                event_class: BrokerEventClass::Position,
                stable_event_id: format!("qualification-external-position-{runtime_epoch}"),
                broker_order_id: None,
                broker_stop_order_id: None,
                logical_request_id: None,
                runtime_epoch,
            }))
            .await?;
        Ok(())
    }
}

#[async_trait]
impl ExecutionStreamPort for SandboxStreams {
    async fn connect(
        &self,
        scope: &RuntimeScope,
        runtime_epoch: u64,
        output: mpsc::Sender<StreamSignal>,
    ) -> Result<BTreeSet<StreamKind>, BrokerPortError> {
        for task in self.tasks.lock().await.drain(..) {
            task.abort();
        }
        let supervisor = ExecutionStreamSupervisor::new(
            self.client.clone(),
            ExecutionStreamConfig {
                ping_delay_ms: 5_000,
                stale_timeout: Duration::from_secs(15),
                subscription_ack_timeout: Duration::from_secs(15),
                ..ExecutionStreamConfig::default()
            },
        )
        .map_err(|error| port_error("OrdersStreamService", "OrderStateStream", error))?;
        let mut handle = supervisor
            .start(
                ExecutionStreamKind::OrderState,
                vec![scope.broker_account_id.clone()],
            )
            .map_err(|error| port_error("OrdersStreamService", "OrderStateStream", error))?;
        let ack = tokio::time::timeout(Duration::from_secs(20), async {
            while let Some(event) = handle.recv().await {
                match event {
                    ExecutionStreamEvent::Evidence(
                        CanonicalExecutionStreamEvent::Subscription { .. },
                    ) => return Ok(()),
                    ExecutionStreamEvent::Fault(error) => return Err(error.to_string()),
                    _ => {}
                }
            }
            Err("OrderStateStream closed before subscription ACK".to_owned())
        })
        .await
        .map_err(|_| BrokerPortError {
            service: "OrdersStreamService",
            method: "OrderStateStream",
            class: BrokerResultClass::Transient,
            message: "subscription ACK timeout".into(),
            retry_after: None,
        })?
        .map_err(|message| BrokerPortError {
            service: "OrdersStreamService",
            method: "OrderStateStream",
            class: BrokerResultClass::Transient,
            message,
            retry_after: None,
        })?;
        let _ = ack;

        let account_id = scope.broker_account_id.clone();
        let ping_settings = || {
            Some(v1::PingDelaySettings {
                ping_delay_ms: Some(5_000),
            })
        };
        let mut portfolio = self
            .client
            .open_portfolio_stream(v1::PortfolioStreamRequest {
                accounts: vec![account_id.clone()],
                ping_settings: ping_settings(),
            })
            .await
            .map_err(|error| {
                grpc_port_error("OperationsStreamService", "PortfolioStream", error)
            })?;
        await_portfolio_ack(&mut portfolio, &account_id).await?;

        let mut positions = self
            .client
            .open_positions_stream(v1::PositionsStreamRequest {
                accounts: vec![account_id.clone()],
                with_initial_positions: true,
                ping_settings: ping_settings(),
            })
            .await
            .map_err(|error| {
                grpc_port_error("OperationsStreamService", "PositionsStream", error)
            })?;
        await_positions_ack(&mut positions, &account_id).await?;

        let mut operations = self
            .client
            .open_operations_stream(v1::OperationsStreamRequest {
                accounts: vec![account_id.clone()],
                ping_settings: ping_settings(),
            })
            .await
            .map_err(|error| {
                grpc_port_error("OperationsStreamService", "OperationsStream", error)
            })?;
        await_operations_ack(&mut operations, &account_id).await?;

        *self.output.lock().await = Some(output.clone());
        let order_output = output.clone();
        let order_account_id = account_id.clone();
        let order_task = tokio::spawn(async move {
            while let Some(event) = handle.recv().await {
                let signal = stream_signal(event, &order_account_id, runtime_epoch);
                if let Some(signal) = signal
                    && order_output.send(signal).await.is_err()
                {
                    return;
                }
            }
        });
        let portfolio_output = output.clone();
        let portfolio_task = tokio::spawn(async move {
            let mut revision = 0_u64;
            loop {
                match tokio::time::timeout(Duration::from_secs(15), portfolio.message()).await {
                    Ok(Ok(Some(message))) => {
                        if let Some(v1::portfolio_stream_response::Payload::Portfolio(value)) =
                            message.payload
                        {
                            revision = revision.saturating_add(1);
                            if portfolio_output
                                .send(StreamSignal::Event(BrokerEvent {
                                    account_id: value.account_id,
                                    event_class: BrokerEventClass::Portfolio,
                                    stable_event_id: format!(
                                        "portfolio:{runtime_epoch}:{revision}"
                                    ),
                                    broker_order_id: None,
                                    broker_stop_order_id: None,
                                    logical_request_id: None,
                                    runtime_epoch,
                                }))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                    Ok(Ok(None)) | Ok(Err(_)) | Err(_) => {
                        let _ = portfolio_output
                            .send(StreamSignal::Disconnected {
                                stream: StreamKind::Portfolio,
                                reason: "PortfolioStream closed or faulted".into(),
                            })
                            .await;
                        return;
                    }
                }
            }
        });
        let positions_output = output.clone();
        let positions_task = tokio::spawn(async move {
            let mut revision = 0_u64;
            loop {
                match tokio::time::timeout(Duration::from_secs(15), positions.message()).await {
                    Ok(Ok(Some(message))) => {
                        let account = match message.payload {
                            Some(v1::positions_stream_response::Payload::Position(value)) => {
                                Some(value.account_id)
                            }
                            Some(v1::positions_stream_response::Payload::InitialPositions(
                                value,
                            )) => Some(value.account_id),
                            _ => None,
                        };
                        if let Some(account_id) = account {
                            revision = revision.saturating_add(1);
                            if positions_output
                                .send(StreamSignal::Event(BrokerEvent {
                                    account_id,
                                    event_class: BrokerEventClass::Position,
                                    stable_event_id: format!("position:{runtime_epoch}:{revision}"),
                                    broker_order_id: None,
                                    broker_stop_order_id: None,
                                    logical_request_id: None,
                                    runtime_epoch,
                                }))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                    Ok(Ok(None)) | Ok(Err(_)) | Err(_) => {
                        let _ = positions_output
                            .send(StreamSignal::Disconnected {
                                stream: StreamKind::Positions,
                                reason: "PositionsStream closed or faulted".into(),
                            })
                            .await;
                        return;
                    }
                }
            }
        });
        let operations_output = output;
        let operations_task = tokio::spawn(async move {
            let mut revision = 0_u64;
            loop {
                match tokio::time::timeout(Duration::from_secs(15), operations.message()).await {
                    Ok(Ok(Some(message))) => {
                        if let Some(v1::operations_stream_response::Payload::Operation(value)) =
                            message.payload
                        {
                            revision = revision.saturating_add(1);
                            let stable_id = if value.id.trim().is_empty() {
                                format!("operation:{runtime_epoch}:{revision}")
                            } else {
                                format!("operation:{}", value.id)
                            };
                            if operations_output
                                .send(StreamSignal::Event(BrokerEvent {
                                    account_id: value.broker_account_id,
                                    event_class: BrokerEventClass::Operation,
                                    stable_event_id: stable_id,
                                    broker_order_id: None,
                                    broker_stop_order_id: None,
                                    logical_request_id: None,
                                    runtime_epoch,
                                }))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                    Ok(Ok(None)) | Ok(Err(_)) | Err(_) => {
                        let _ = operations_output
                            .send(StreamSignal::Disconnected {
                                stream: StreamKind::Operations,
                                reason: "OperationsStream closed or faulted".into(),
                            })
                            .await;
                        return;
                    }
                }
            }
        });
        *self.tasks.lock().await =
            vec![order_task, portfolio_task, positions_task, operations_task];
        Ok([
            StreamKind::OrderState,
            StreamKind::Positions,
            StreamKind::Portfolio,
            StreamKind::Operations,
        ]
        .into_iter()
        .collect())
    }

    async fn disconnect(&self) -> Result<(), BrokerPortError> {
        for task in self.tasks.lock().await.drain(..) {
            task.abort();
            let _ = task.await;
        }
        self.output.lock().await.take();
        Ok(())
    }
}

async fn await_portfolio_ack(
    stream: &mut vox_tinvest::GrpcServerStream<v1::PortfolioStreamResponse>,
    account_id: &str,
) -> Result<(), BrokerPortError> {
    await_stream_ack("PortfolioStream", async {
        loop {
            let message = stream
                .message()
                .await
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "stream closed before subscription ACK".to_owned())?;
            if let Some(v1::portfolio_stream_response::Payload::Subscriptions(result)) =
                message.payload
            {
                let exact = result.accounts.len() == 1
                    && result.accounts[0].account_id == account_id
                    && result.accounts[0].subscription_status
                        == v1::PortfolioSubscriptionStatus::Success as i32
                    && !result.stream_id.trim().is_empty();
                return exact
                    .then_some(())
                    .ok_or_else(|| "PortfolioStream subscription ACK mismatch".to_owned());
            }
        }
    })
    .await
}

async fn await_positions_ack(
    stream: &mut vox_tinvest::GrpcServerStream<v1::PositionsStreamResponse>,
    account_id: &str,
) -> Result<(), BrokerPortError> {
    await_stream_ack("PositionsStream", async {
        loop {
            let message = stream
                .message()
                .await
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "stream closed before subscription ACK".to_owned())?;
            if let Some(v1::positions_stream_response::Payload::Subscriptions(result)) =
                message.payload
            {
                let exact = result.accounts.len() == 1
                    && result.accounts[0].account_id == account_id
                    && result.accounts[0].subscription_status
                        == v1::PositionsAccountSubscriptionStatus::PositionsSubscriptionStatusSuccess
                            as i32
                    && !result.stream_id.trim().is_empty();
                return exact
                    .then_some(())
                    .ok_or_else(|| "PositionsStream subscription ACK mismatch".to_owned());
            }
        }
    })
    .await
}

async fn await_operations_ack(
    stream: &mut vox_tinvest::GrpcServerStream<v1::OperationsStreamResponse>,
    account_id: &str,
) -> Result<(), BrokerPortError> {
    await_stream_ack("OperationsStream", async {
        loop {
            let message = stream
                .message()
                .await
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "stream closed before subscription ACK".to_owned())?;
            if let Some(v1::operations_stream_response::Payload::Subscriptions(result)) =
                message.payload
            {
                let exact = result.accounts.len() == 1
                    && result.accounts[0] == account_id
                    && result.subscription_status
                        == v1::OperationsAccountSubscriptionStatus::OperationsSubscriptionStatusSuccess
                            as i32
                    && !result.stream_id.trim().is_empty();
                return exact
                    .then_some(())
                    .ok_or_else(|| "OperationsStream subscription ACK mismatch".to_owned());
            }
        }
    })
    .await
}

async fn await_stream_ack<F>(method: &'static str, future: F) -> Result<(), BrokerPortError>
where
    F: Future<Output = Result<(), String>>,
{
    tokio::time::timeout(Duration::from_secs(20), future)
        .await
        .map_err(|_| BrokerPortError {
            service: "OperationsStreamService",
            method,
            class: BrokerResultClass::Transient,
            message: "subscription ACK timeout".into(),
            retry_after: None,
        })?
        .map_err(|message| BrokerPortError {
            service: "OperationsStreamService",
            method,
            class: BrokerResultClass::Permanent,
            message,
            retry_after: None,
        })
}

struct SandboxCredential;

#[async_trait]
impl CredentialResolverPort for SandboxCredential {
    async fn resolve(&self, _: &RuntimeScope) -> Result<CredentialResolution, BrokerPortError> {
        Ok(CredentialResolution {
            execution_authorized: true,
        })
    }
}

#[tokio::test]
#[ignore = "requires TINVEST_SANDBOX_TOKEN and mutates only T-Invest sandbox"]
async fn complete_runtime_qualification_in_sandbox() -> Result<(), BoxError> {
    let token =
        std::env::var("TINVEST_SANDBOX_TOKEN").map_err(|_| "TINVEST_SANDBOX_TOKEN is required")?;
    let client = TInvestGrpcClient::sandbox(GrpcCredential::Sandbox(SecretToken::new(token)?))?;
    let account_client = AccountReadClient::new(client.clone());
    let account = account_client
        .sandbox_accounts()
        .await?
        .accounts
        .into_iter()
        .find(|account| account.status == v1::AccountStatus::Open as i32)
        .ok_or("open sandbox account is required")?;
    let (instrument_uid, limit_price) = qualification_instrument(&client).await?;
    let path = runtime_path();
    let scope = RuntimeScope::new(
        Provider::TInvest,
        RuntimeEnvironment::Sandbox,
        account.account_id,
        OpaqueRef::new("connection:sandbox-qualification")?,
        OpaqueRef::new("credential:environment")?,
    )?;
    let snapshot_starts = Arc::new(AtomicU64::new(0));
    let reads = Arc::new(SandboxReads {
        client: client.clone(),
        snapshot_starts: snapshot_starts.clone(),
    });
    let execution = Arc::new(SandboxExecution {
        client: client.clone(),
        instrument_uid,
        limit_price,
    });
    let streams = Arc::new(SandboxStreams::new(client.clone()));
    let metrics = Arc::new(InMemoryMetrics::default());
    let mut broker_order_ids = BTreeSet::new();
    let mut logical_request_ids = BTreeSet::new();
    let mut ledger = LiveLedger::default();

    let result = async {
        let store = SqliteRuntimeStore::open_async(&path).await?;
        let config = store.configuration()?;
        let coordinator = build_coordinator(
            &scope,
            store.clone(),
            reads.clone(),
            execution.clone(),
            streams.clone(),
            metrics.clone(),
        );
        let initial = coordinator.start().await?;
        ledger.qualified("runtime_startup_ownership", format!("epoch={}", initial.runtime_epoch));
        let startup_health = coordinator.health().await;
        for required_stream in [
            StreamKind::OrderState,
            StreamKind::Positions,
            StreamKind::Portfolio,
            StreamKind::Operations,
        ] {
            require(
                startup_health.stream_states.iter().any(|health| {
                    health.stream == required_stream
                        && health.required_for_ready
                        && health.state == vox_runtime::StreamState::Active
                }),
                "required stream ACK not active before READY",
            )?;
        }
        require(
            startup_health.stream_states.iter().any(|health| {
                health.stream == StreamKind::Trades && !health.required_for_ready
            }),
            "TradesStream optional readiness policy missing",
        )?;
        ledger.qualified(
            "connecting",
            "credential resolved; exact OrderState/Positions/Portfolio/Operations ACKs verified; Trades optional",
        );
        ledger.qualified(
            "initial_authoritative_reconciliation",
            format!("reconciliation_id={}", initial.reconciliation_id),
        );
        require(initial.resulting_state == RuntimeState::Ready, "runtime did not reach READY")?;
        ledger.qualified("ready_gate", "READY committed after snapshot/stream/snapshot handoff");

        let acknowledged_id = uuid::Uuid::new_v4().to_string();
        logical_request_ids.insert(acknowledged_id.clone());
        let receipt = coordinator
            .dispatch(
                acknowledged_id.clone(),
                MutationKind::PostOrder,
                "quantity_lots=1; order_type=LIMIT; price=redacted",
                uuid::Uuid::new_v4().to_string(),
            )
            .await?;
        require(receipt.state == JournalState::Acknowledged, "mutation not acknowledged")?;
        let links = store.all_identity_links(&scope.key())?;
        let broker_order_id = links
            .iter()
            .find(|links| links.logical_request_id == acknowledged_id)
            .and_then(|links| links.broker_order_id.clone())
            .ok_or("durable broker identity link missing")?;
        broker_order_ids.insert(broker_order_id);
        ledger.qualified(
            "durable_mutation_broker_execution_identity_link",
            "UNKNOWN fence preceded one dispatch; ACK and typed broker link durable",
        );
        coordinator.shutdown().await?;
        drop(coordinator);
        drop(store);

        let restarted_store = SqliteRuntimeStore::open_async(&path).await?;
        let restarted = build_coordinator(
            &scope,
            restarted_store.clone(),
            reads.clone(),
            execution.clone(),
            streams.clone(),
            metrics.clone(),
        );
        let restart_report = restarted.start().await?;
        require(restart_report.active_order_count > 0, "broker-visible open order missing")?;
        ledger.qualified(
            "restart_with_broker_visible_open_state",
            format!("active_orders={}", restart_report.active_order_count),
        );
        let snapshots_before_external = snapshot_starts.load(Ordering::SeqCst);
        streams
            .force_external_position_signal(
                &scope.broker_account_id,
                restart_report.runtime_epoch,
            )
            .await?;
        for _ in 0..200 {
            if snapshot_starts.load(Ordering::SeqCst) > snapshots_before_external
                && restarted.health().await.state == RuntimeState::Ready
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        require(
            snapshot_starts.load(Ordering::SeqCst) > snapshots_before_external,
            "external position signal did not force authoritative reconciliation",
        )?;
        ledger.qualified(
            "position_snapshot_reconciliation",
            format!(
                "positions={}; external position signal closed admission and forced unary refresh before READY",
                restart_report.position_count
            ),
        );
        ledger.qualified(
            "regular_order_reconciliation",
            "linked active broker order converged without replay",
        );
        ledger.qualified(
            "stop_protection_reconciliation",
            format!("active_stops={}; #10 relative/absolute trailing identities remain accepted", restart_report.active_stop_count),
        );

        let unknown_id = uuid::Uuid::new_v4().to_string();
        logical_request_ids.insert(unknown_id.clone());
        let unknown = restarted
            .dispatch(
                unknown_id.clone(),
                MutationKind::PostOrder,
                "quantity_lots=1; order_type=LIMIT; fault=unknown",
                uuid::Uuid::new_v4().to_string(),
            )
            .await;
        require(
            matches!(unknown, Err(vox_runtime::RuntimeError::UnknownAfterDispatch(_))),
            "controlled ambiguous dispatch did not remain UNKNOWN",
        )?;
        restarted.shutdown().await?;
        drop(restarted);
        drop(restarted_store);

        let resolved_store = SqliteRuntimeStore::open_async(&path).await?;
        let resolved = build_coordinator(
            &scope,
            resolved_store.clone(),
            reads.clone(),
            execution.clone(),
            streams.clone(),
            metrics.clone(),
        );
        let resolved_report = resolved.start().await?;
        require(
            !resolved_report
                .unresolved_logical_request_ids
                .contains(&unknown_id),
            "UNKNOWN not resolved from request identity",
        )?;
        for links in resolved_store.all_identity_links(&scope.key())? {
            if let Some(id) = links.broker_order_id {
                broker_order_ids.insert(id);
            }
        }
        ledger.qualified(
            "unknown_restart_authoritative_resolution_no_replay",
            "GetSandboxOrderState by REQUEST identity resolved UNKNOWN; no mutation replay",
        );

        let epoch = resolved.health().await.runtime_epoch;
        let event = BrokerEvent {
            account_id: scope.broker_account_id.clone(),
            event_class: BrokerEventClass::Fill,
            stable_event_id: format!("qualification-dedupe-{epoch}"),
            broker_order_id: broker_order_ids.iter().next().cloned(),
            broker_stop_order_id: None,
            logical_request_id: None,
            runtime_epoch: epoch,
        };
        require(
            resolved_store.record_broker_event(&scope.key(), &event, now_unix_ms())?,
            "first broker event was not inserted",
        )?;
        require(
            !resolved_store.record_broker_event(&scope.key(), &event, now_unix_ms())?,
            "duplicate broker event was inserted twice",
        )?;
        ledger.qualified(
            "duplicate_event_readback_dedupe",
            "stable broker event identity applied once",
        );

        streams.force_gap().await?;
        wait_ready(&resolved).await?;
        ledger.qualified(
            "stream_gap_reconciliation_recovery",
            "forced required-stream gap closed gate; four exact ACKs and unary reconciliation restored READY",
        );
        resolved.shutdown().await?;
        ledger.qualified(
            "graceful_shutdown_restart",
            "STOPPING -> STOPPED; clean marker persisted; ownership released",
        );
        drop(resolved);
        drop(resolved_store);

        let final_store = SqliteRuntimeStore::open_async(&path).await?;
        require(
            final_store.counts(&scope.key())?.unresolved_unknown_count == 0,
            "unresolved UNKNOWN count is non-zero",
        )?;
        ledger.qualified("zero_unresolved_unknown", "count=0");
        ledger.qualified(
            "resource_queue_summary",
            format!(
                "execution_capacity=256; stream_capacity=1024; reconciliation_concurrency=8; sqlite_connections<=4; journal_mode={}; synchronous={}",
                config.journal_mode, config.synchronous
            ),
        );
        drop(final_store);
        Ok::<(), BoxError>(())
    }
    .await;

    let _ = streams.disconnect().await;
    let cleanup = cleanup_orders(
        &client,
        &scope.broker_account_id,
        &broker_order_ids,
        &logical_request_ids,
    )
    .await;
    match &cleanup {
        Ok(detail) => ledger.qualified("cleanup_readback", detail),
        Err(error) => ledger.failed("cleanup_readback", error),
    }
    cleanup_path(&path);
    if let Err(error) = &result {
        ledger.fail_missing(error);
    } else if let Err(error) = &cleanup {
        ledger.fail_missing(error);
    }
    ledger.finish()?;
    result?;
    cleanup?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires TINVEST_SANDBOX_TOKEN; 60-minute idle sandbox soak"]
async fn runtime_idle_soak_in_sandbox() -> Result<(), BoxError> {
    let token =
        std::env::var("TINVEST_SANDBOX_TOKEN").map_err(|_| "TINVEST_SANDBOX_TOKEN is required")?;
    let minutes = std::env::var("TINVEST_RUNTIME_SOAK_MINUTES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(60);
    require(minutes >= 10, "soak must run at least 10 minutes")?;
    let client = TInvestGrpcClient::sandbox(GrpcCredential::Sandbox(SecretToken::new(token)?))?;
    let account = AccountReadClient::new(client.clone())
        .sandbox_accounts()
        .await?
        .accounts
        .into_iter()
        .find(|account| account.status == v1::AccountStatus::Open as i32)
        .ok_or("open sandbox account is required")?;
    let (instrument_uid, limit_price) = qualification_instrument(&client).await?;
    let path = runtime_path();
    let scope = RuntimeScope::new(
        Provider::TInvest,
        RuntimeEnvironment::Sandbox,
        account.account_id,
        OpaqueRef::new("connection:sandbox-soak")?,
        OpaqueRef::new("credential:environment")?,
    )?;
    let store = SqliteRuntimeStore::open_async(&path).await?;
    let streams = Arc::new(SandboxStreams::new(client.clone()));
    let metrics = Arc::new(InMemoryMetrics::default());
    let coordinator = build_coordinator(
        &scope,
        store,
        Arc::new(SandboxReads {
            client: client.clone(),
            snapshot_starts: Arc::new(AtomicU64::new(0)),
        }),
        Arc::new(SandboxExecution {
            client,
            instrument_uid,
            limit_price,
        }),
        streams,
        metrics.clone(),
    );
    let result = async {
        let report = coordinator.start().await?;
        require(report.resulting_state == RuntimeState::Ready, "soak startup not READY")?;
        let started = tokio::time::Instant::now();
        let duration = Duration::from_secs(minutes.saturating_mul(60));
        let mut next_report = Duration::from_secs(60);
        while started.elapsed() < duration {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let health = coordinator.health().await;
            require(health.state == RuntimeState::Ready, "soak runtime left READY")?;
            require(
                health
                    .stream_states
                    .iter()
                    .all(|stream| stream.queue_depth <= vox_runtime::STREAM_QUEUE_CAPACITY),
                "soak stream queue exceeded bound",
            )?;
            if started.elapsed() >= next_report {
                println!(
                    "SOAK_PROGRESS elapsed_seconds={} state={:?} unresolved_unknown={} max_queue_depth={}",
                    started.elapsed().as_secs(),
                    health.state,
                    health.unresolved_unknown_count,
                    health
                        .stream_states
                        .iter()
                        .map(|stream| stream.queue_depth)
                        .max()
                        .unwrap_or_default(),
                );
                next_report = next_report.saturating_add(Duration::from_secs(60));
            }
        }
        coordinator.shutdown().await?;
        println!(
            "SOAK_RUNTIME_SUMMARY duration_minutes={minutes} metric_series={} rejected_metric_updates={}",
            metrics.snapshot().len(),
            metrics.rejected_updates(),
        );
        Ok::<(), BoxError>(())
    }
    .await;
    if coordinator.health().await.state != RuntimeState::Stopped {
        let _ = coordinator.shutdown().await;
    }
    drop(coordinator);
    cleanup_path(&path);
    result
}

fn build_coordinator(
    scope: &RuntimeScope,
    store: SqliteRuntimeStore,
    reads: Arc<SandboxReads>,
    execution: Arc<SandboxExecution>,
    streams: Arc<SandboxStreams>,
    metrics: Arc<InMemoryMetrics>,
) -> Arc<LiveCoordinator> {
    RuntimeCoordinator::new(
        scope.clone(),
        store,
        reads,
        execution,
        streams,
        Arc::new(SandboxCredential),
        metrics,
        ReconciliationConfig::default(),
        RuntimeConfig {
            shutdown_timeout: Duration::from_secs(10),
        },
    )
}

async fn qualification_instrument(
    client: &TInvestGrpcClient,
) -> Result<(String, FixedPoint), BoxError> {
    let mut shares = client
        .shares(v1::InstrumentsRequest {
            instrument_status: Some(v1::InstrumentStatus::Base as i32),
            instrument_exchange: None,
        })
        .await?
        .body
        .instruments
        .into_iter()
        .filter(|share| {
            share.api_trade_available_flag
                && share.buy_available_flag
                && !share.uid.trim().is_empty()
                && share.lot > 0
                && share.min_price_increment.is_some()
        })
        .collect::<Vec<_>>();
    shares.sort_by_key(|share| {
        (
            share.ticker != "SBER",
            share.class_code != "TQBR",
            share.ticker.clone(),
        )
    });
    shares.truncate(100);
    let prices = client
        .get_last_prices(v1::GetLastPricesRequest {
            instrument_id: shares.iter().map(|share| share.uid.clone()).collect(),
            last_price_type: v1::LastPriceType::LastPriceExchange as i32,
            ..Default::default()
        })
        .await?
        .body
        .last_prices
        .into_iter()
        .filter_map(|price| price.price.map(|value| (price.instrument_uid, value)))
        .collect::<BTreeMap<_, _>>();
    for share in shares {
        let Some(price) = prices.get(&share.uid) else {
            continue;
        };
        let Some(tick) = share.min_price_increment else {
            continue;
        };
        let price = FixedPoint::from_units_nano(price.units, price.nano)?;
        let tick = FixedPoint::from_units_nano(tick.units, tick.nano)?;
        let ticks = price.total_nanos() / tick.total_nanos();
        if ticks >= 4 {
            return Ok((
                share.uid,
                FixedPoint::from_total_nanos((ticks / 2) * tick.total_nanos()),
            ));
        }
    }
    Err("no API-tradeable sandbox instrument with authoritative price/tick".into())
}

async fn cleanup_orders(
    client: &TInvestGrpcClient,
    account_id: &str,
    expected: &BTreeSet<String>,
    logical_request_ids: &BTreeSet<String>,
) -> Result<String, BoxError> {
    let active_before = canonical_orders(
        client
            .get_sandbox_orders(v1::GetOrdersRequest {
                account_id: account_id.to_owned(),
                advanced_filters: None,
            })
            .await?
            .body,
    )?;
    let cleanup_ids = active_before
        .iter()
        .filter(|order| {
            order
                .broker_order_id
                .as_ref()
                .is_some_and(|id| expected.contains(id))
                || order
                    .client_request_id
                    .as_ref()
                    .is_some_and(|id| logical_request_ids.contains(id))
        })
        .filter_map(|order| order.broker_order_id.clone())
        .collect::<BTreeSet<_>>();
    for order_id in &cleanup_ids {
        let result = client
            .cancel_sandbox_order(
                MutationGuard::new(Environment::Sandbox).authorize_mutation()?,
                v1::CancelOrderRequest {
                    account_id: account_id.to_owned(),
                    order_id: order_id.clone(),
                    order_id_type: Some(v1::OrderIdType::Exchange as i32),
                },
            )
            .await;
        if let Err(error) = result
            && !provider_not_found(&error)
        {
            return Err(error.into());
        }
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    let active = canonical_orders(
        client
            .get_sandbox_orders(v1::GetOrdersRequest {
                account_id: account_id.to_owned(),
                advanced_filters: None,
            })
            .await?
            .body,
    )?;
    let remaining = active
        .iter()
        .filter_map(|order| order.broker_order_id.as_ref())
        .filter(|id| cleanup_ids.contains(*id))
        .collect::<Vec<_>>();
    require(remaining.is_empty(), "qualification orders remain active")?;
    Ok(format!(
        "orders_canceled={}; active_readback=0",
        cleanup_ids.len()
    ))
}

async fn wait_ready(coordinator: &Arc<LiveCoordinator>) -> Result<(), BoxError> {
    for _ in 0..60 {
        if coordinator.health().await.state == RuntimeState::Ready {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err("runtime did not recover READY after forced gap".into())
}

fn stream_signal(
    event: ExecutionStreamEvent,
    account_id: &str,
    runtime_epoch: u64,
) -> Option<StreamSignal> {
    match event {
        ExecutionStreamEvent::Evidence(CanonicalExecutionStreamEvent::OrderState(order)) => {
            order.broker_order_id.map(|broker_order_id| {
                StreamSignal::Event(BrokerEvent {
                    account_id: account_id.to_owned(),
                    event_class: BrokerEventClass::Order,
                    stable_event_id: format!(
                        "order:{broker_order_id}:{}:{}",
                        order.execution_status, order.lots_executed
                    ),
                    broker_order_id: Some(broker_order_id),
                    broker_stop_order_id: None,
                    logical_request_id: order.client_request_id,
                    runtime_epoch,
                })
            })
        }
        ExecutionStreamEvent::Evidence(CanonicalExecutionStreamEvent::StopOrderState(stop)) => {
            stop.broker_stop_order_id.map(|broker_stop_order_id| {
                StreamSignal::Event(BrokerEvent {
                    account_id: account_id.to_owned(),
                    event_class: BrokerEventClass::Stop,
                    stable_event_id: format!("stop:{broker_stop_order_id}:{}", stop.status),
                    broker_order_id: None,
                    broker_stop_order_id: Some(broker_stop_order_id),
                    logical_request_id: None,
                    runtime_epoch,
                })
            })
        }
        ExecutionStreamEvent::Fault(error) => Some(StreamSignal::Gap {
            stream: StreamKind::OrderState,
            reason: error.to_string(),
        }),
        _ => None,
    }
}

fn order_fact(
    scope: &RuntimeScope,
    order: vox_tinvest::execution::CanonicalOrderState,
) -> Result<OrderFact, BrokerPortError> {
    let status = v1::OrderExecutionReportStatus::try_from(order.execution_status).ok();
    let active = matches!(
        status,
        Some(v1::OrderExecutionReportStatus::ExecutionReportStatusNew)
            | Some(v1::OrderExecutionReportStatus::ExecutionReportStatusPartiallyfill)
    );
    Ok(OrderFact {
        account_id: scope.broker_account_id.clone(),
        broker_order_id: order.broker_order_id.ok_or_else(|| BrokerPortError {
            service: "OrdersService",
            method: "order_decode",
            class: BrokerResultClass::Permanent,
            message: "order omitted broker identity".into(),
            retry_after: None,
        })?,
        logical_request_id: order.client_request_id,
        instrument_uid: order.instrument_uid.unwrap_or_default(),
        active,
        terminal: !active,
    })
}

fn position_fact(
    scope: &RuntimeScope,
    instrument_uid: String,
    quantity_units: i64,
) -> vox_runtime::PositionFact {
    vox_runtime::PositionFact {
        account_id: scope.broker_account_id.clone(),
        instrument_uid,
        quantity_units,
        broker_observed_at_unix_ms: None,
    }
}

fn grpc_port_error(
    service: &'static str,
    method: &'static str,
    error: GrpcError,
) -> BrokerPortError {
    let class = match &error.kind {
        GrpcErrorKind::Provider(provider) => match provider.code {
            tonic::Code::Unauthenticated => BrokerResultClass::Credential,
            tonic::Code::PermissionDenied => BrokerResultClass::Permission,
            tonic::Code::ResourceExhausted => BrokerResultClass::RateLimited,
            tonic::Code::Unavailable | tonic::Code::DeadlineExceeded => {
                BrokerResultClass::Transient
            }
            _ => BrokerResultClass::Permanent,
        },
        _ => BrokerResultClass::Transient,
    };
    BrokerPortError {
        service,
        method,
        class,
        message: error.to_string(),
        retry_after: None,
    }
}

fn provider_not_found(error: &GrpcError) -> bool {
    matches!(
        &error.kind,
        GrpcErrorKind::Provider(provider) if provider.code == tonic::Code::NotFound
    )
}

fn port_error(
    service: &'static str,
    method: &'static str,
    error: impl core::fmt::Display,
) -> BrokerPortError {
    BrokerPortError {
        service,
        method,
        class: BrokerResultClass::Permanent,
        message: error.to_string(),
        retry_after: None,
    }
}

fn timestamp(unix_ms: i64) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: unix_ms.div_euclid(1_000),
        nanos: i32::try_from(unix_ms.rem_euclid(1_000) * 1_000_000).unwrap_or_default(),
    }
}

fn now_unix_ms() -> i64 {
    i64::try_from(time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000)
        .unwrap_or(i64::MAX)
}

fn nonempty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn require(condition: bool, message: &'static str) -> Result<(), BoxError> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

#[derive(Default)]
struct LiveLedger(BTreeMap<&'static str, Result<String, String>>);

impl LiveLedger {
    fn qualified(&mut self, row: &'static str, detail: impl core::fmt::Display) {
        self.0.insert(row, Ok(detail.to_string()));
    }

    fn failed(&mut self, row: &'static str, error: impl core::fmt::Display) {
        self.0.insert(row, Err(error.to_string()));
    }

    fn fail_missing(&mut self, error: impl core::fmt::Display) {
        let detail = format!("dependent qualification aborted: {error}");
        for row in RUNTIME_QUALIFICATION_ROWS {
            self.0.entry(row).or_insert_with(|| Err(detail.clone()));
        }
    }

    fn finish(&self) -> Result<(), BoxError> {
        let mut failures = Vec::new();
        for row in RUNTIME_QUALIFICATION_ROWS {
            match self.0.get(row) {
                Some(Ok(detail)) => {
                    println!("RUNTIME_QUALIFICATION {row}: QUALIFIED; {detail}");
                }
                Some(Err(detail)) => {
                    println!("RUNTIME_QUALIFICATION {row}: FAILED; {detail}");
                    failures.push(row);
                }
                None => {
                    println!("RUNTIME_QUALIFICATION {row}: FAILED; row not executed");
                    failures.push(row);
                }
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(format!("runtime qualification failed rows: {failures:?}").into())
        }
    }
}

fn runtime_path() -> PathBuf {
    std::env::temp_dir().join(format!("vox-runtime-live-{}.sqlite", uuid::Uuid::new_v4()))
}

fn cleanup_path(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("runtime.lock"));
    let _ = std::fs::remove_file(format!("{}-wal", path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", path.display()));
}
