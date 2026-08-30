//! #11 execution/capital streams over existing T-Invest gRPC client.

use std::collections::BTreeSet;
use std::future::Future;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{Mutex, mpsc};
use vox_runtime::{
    BrokerEvent, BrokerEventClass, BrokerExecutionState, BrokerPortError, BrokerResultClass,
    ExecutionStreamPort, OrderExecutionStatus, ProviderStatusCause, RuntimeScope,
    StopExecutionStatus, StreamKind, StreamSignal,
};

use crate::execution::CanonicalExecutionStreamEvent;
use crate::execution_stream::{
    ExecutionStreamConfig, ExecutionStreamEvent, ExecutionStreamKind, ExecutionStreamSupervisor,
};
use crate::generated::v1;
use crate::{GrpcError, GrpcErrorKind, GrpcServerStream, TInvestGrpcClient};

const CREDENTIAL_SESSION_MAX_AGE: Duration = Duration::from_secs(30 * 60);

pub struct TInvestRuntimeStreamAdapter {
    client: TInvestGrpcClient,
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

impl TInvestRuntimeStreamAdapter {
    #[must_use]
    pub(crate) fn new(client: TInvestGrpcClient) -> Self {
        Self {
            client,
            tasks: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl ExecutionStreamPort for TInvestRuntimeStreamAdapter {
    async fn connect(
        &self,
        scope: &RuntimeScope,
        runtime_epoch: u64,
        output: mpsc::Sender<StreamSignal>,
    ) -> Result<BTreeSet<StreamKind>, BrokerPortError> {
        self.disconnect().await?;
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
        let mut orders = supervisor
            .start(
                ExecutionStreamKind::OrderState,
                vec![scope.broker_account_id.clone()],
            )
            .map_err(|error| port_error("OrdersStreamService", "OrderStateStream", error))?;
        await_order_ack(&mut orders).await?;

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

        let order_output = output.clone();
        let order_account = account_id.clone();
        let order_task = tokio::spawn(async move {
            while let Some(event) = orders.recv().await {
                if let Some(signal) = stream_signal(event, &order_account, runtime_epoch)
                    && order_output.send(signal).await.is_err()
                {
                    return;
                }
            }
        });
        let portfolio_task = spawn_portfolio(portfolio, output.clone(), runtime_epoch);
        let positions_task = spawn_positions(positions, output.clone(), runtime_epoch);
        let operations_task = spawn_operations(operations, output.clone(), runtime_epoch);
        // Stored sessions deliberately have bounded lifetime. Reconnect resolves CredentialRef
        // again, so rotation cannot leave an old bearer active indefinitely.
        let refresh_task = tokio::spawn(async move {
            tokio::time::sleep(CREDENTIAL_SESSION_MAX_AGE).await;
            let _ = output
                .send(StreamSignal::Disconnected {
                    stream: StreamKind::OrderState,
                    reason: "stored credential session reached refresh boundary".to_owned(),
                })
                .await;
        });
        *self.tasks.lock().await = vec![
            order_task,
            portfolio_task,
            positions_task,
            operations_task,
            refresh_task,
        ];
        Ok(BTreeSet::from([
            StreamKind::OrderState,
            StreamKind::Positions,
            StreamKind::Portfolio,
            StreamKind::Operations,
        ]))
    }

    async fn disconnect(&self) -> Result<(), BrokerPortError> {
        for task in self.tasks.lock().await.drain(..) {
            task.abort();
            let _ = task.await;
        }
        Ok(())
    }
}

async fn await_order_ack(
    handle: &mut crate::execution_stream::ExecutionStreamHandle,
) -> Result<(), BrokerPortError> {
    tokio::time::timeout(Duration::from_secs(20), async {
        while let Some(event) = handle.recv().await {
            match event {
                ExecutionStreamEvent::Evidence(CanonicalExecutionStreamEvent::Subscription {
                    ..
                }) => return Ok(()),
                ExecutionStreamEvent::Fault(error) => return Err(error.to_string()),
                _ => {}
            }
        }
        Err("OrderStateStream closed before subscription ACK".to_owned())
    })
    .await
    .map_err(|_| transient("OrdersStreamService", "OrderStateStream", "ACK timeout"))?
    .map_err(|message| permanent("OrdersStreamService", "OrderStateStream", message))
}

async fn await_portfolio_ack(
    stream: &mut GrpcServerStream<v1::PortfolioStreamResponse>,
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
    stream: &mut GrpcServerStream<v1::PositionsStreamResponse>,
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
    stream: &mut GrpcServerStream<v1::OperationsStreamResponse>,
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
        .map_err(|_| transient("OperationsStreamService", method, "ACK timeout"))?
        .map_err(|message| permanent("OperationsStreamService", method, message))
}

fn spawn_portfolio(
    mut stream: GrpcServerStream<v1::PortfolioStreamResponse>,
    output: mpsc::Sender<StreamSignal>,
    epoch: u64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut revision = 0_u64;
        loop {
            match tokio::time::timeout(Duration::from_secs(15), stream.message()).await {
                Ok(Ok(Some(message))) => {
                    if let Some(v1::portfolio_stream_response::Payload::Portfolio(value)) =
                        message.payload
                    {
                        revision = revision.saturating_add(1);
                        if output
                            .send(capital_event(
                                value.account_id,
                                BrokerEventClass::Portfolio,
                                format!("portfolio:{epoch}:{revision}"),
                                epoch,
                            ))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                }
                _ => {
                    let _ = output
                        .send(StreamSignal::Disconnected {
                            stream: StreamKind::Portfolio,
                            reason: "PortfolioStream closed or faulted".to_owned(),
                        })
                        .await;
                    return;
                }
            }
        }
    })
}

fn spawn_positions(
    mut stream: GrpcServerStream<v1::PositionsStreamResponse>,
    output: mpsc::Sender<StreamSignal>,
    epoch: u64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut revision = 0_u64;
        loop {
            match tokio::time::timeout(Duration::from_secs(15), stream.message()).await {
                Ok(Ok(Some(message))) => {
                    let account = match message.payload {
                        Some(v1::positions_stream_response::Payload::Position(value)) => {
                            Some(value.account_id)
                        }
                        Some(v1::positions_stream_response::Payload::InitialPositions(value)) => {
                            Some(value.account_id)
                        }
                        _ => None,
                    };
                    if let Some(account_id) = account {
                        revision = revision.saturating_add(1);
                        if output
                            .send(capital_event(
                                account_id,
                                BrokerEventClass::Position,
                                format!("position:{epoch}:{revision}"),
                                epoch,
                            ))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                }
                _ => {
                    let _ = output
                        .send(StreamSignal::Disconnected {
                            stream: StreamKind::Positions,
                            reason: "PositionsStream closed or faulted".to_owned(),
                        })
                        .await;
                    return;
                }
            }
        }
    })
}

fn spawn_operations(
    mut stream: GrpcServerStream<v1::OperationsStreamResponse>,
    output: mpsc::Sender<StreamSignal>,
    epoch: u64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut revision = 0_u64;
        loop {
            match tokio::time::timeout(Duration::from_secs(15), stream.message()).await {
                Ok(Ok(Some(message))) => {
                    if let Some(v1::operations_stream_response::Payload::Operation(value)) =
                        message.payload
                    {
                        revision = revision.saturating_add(1);
                        let id = if value.id.trim().is_empty() {
                            format!("operation:{epoch}:{revision}")
                        } else {
                            format!("operation:{}", value.id)
                        };
                        if output
                            .send(capital_event(
                                value.broker_account_id,
                                BrokerEventClass::Operation,
                                id,
                                epoch,
                            ))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                }
                _ => {
                    let _ = output
                        .send(StreamSignal::Disconnected {
                            stream: StreamKind::Operations,
                            reason: "OperationsStream closed or faulted".to_owned(),
                        })
                        .await;
                    return;
                }
            }
        }
    })
}

fn capital_event(
    account_id: String,
    event_class: BrokerEventClass,
    stable_event_id: String,
    runtime_epoch: u64,
) -> StreamSignal {
    StreamSignal::Event(BrokerEvent {
        account_id,
        event_class,
        stable_event_id,
        broker_order_id: None,
        broker_stop_order_id: None,
        logical_request_id: None,
        execution_state: None,
        runtime_epoch,
    })
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
                    execution_state: Some(BrokerExecutionState::Order {
                        status: order_status(order.execution_status),
                        status_cause: order.status_cause.map(|code| ProviderStatusCause { code }),
                    }),
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
                    execution_state: Some(BrokerExecutionState::Stop {
                        status: stop_status(stop.status),
                        status_cause: None,
                    }),
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

fn order_status(value: i32) -> OrderExecutionStatus {
    match v1::OrderExecutionReportStatus::try_from(value) {
        Ok(v1::OrderExecutionReportStatus::ExecutionReportStatusNew) => OrderExecutionStatus::New,
        Ok(v1::OrderExecutionReportStatus::ExecutionReportStatusPartiallyfill) => {
            OrderExecutionStatus::PartiallyFilled
        }
        Ok(v1::OrderExecutionReportStatus::ExecutionReportStatusFill) => {
            OrderExecutionStatus::Filled
        }
        Ok(v1::OrderExecutionReportStatus::ExecutionReportStatusCancelled) => {
            OrderExecutionStatus::Cancelled
        }
        Ok(v1::OrderExecutionReportStatus::ExecutionReportStatusRejected) => {
            OrderExecutionStatus::Rejected
        }
        _ => OrderExecutionStatus::UnknownProviderStatus(value),
    }
}

fn stop_status(value: i32) -> StopExecutionStatus {
    match v1::StopOrderStatusOption::try_from(value) {
        Ok(v1::StopOrderStatusOption::StopOrderStatusActive) => StopExecutionStatus::Active,
        Ok(v1::StopOrderStatusOption::StopOrderStatusExecuted) => StopExecutionStatus::Executed,
        Ok(v1::StopOrderStatusOption::StopOrderStatusCanceled) => StopExecutionStatus::Canceled,
        Ok(v1::StopOrderStatusOption::StopOrderStatusExpired) => StopExecutionStatus::Expired,
        _ => StopExecutionStatus::UnknownProviderStatus(value),
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

fn port_error(
    service: &'static str,
    method: &'static str,
    error: impl core::fmt::Display,
) -> BrokerPortError {
    permanent(service, method, error.to_string())
}

fn transient(
    service: &'static str,
    method: &'static str,
    message: impl Into<String>,
) -> BrokerPortError {
    BrokerPortError {
        service,
        method,
        class: BrokerResultClass::Transient,
        message: message.into(),
        retry_after: None,
    }
}

fn permanent(
    service: &'static str,
    method: &'static str,
    message: impl Into<String>,
) -> BrokerPortError {
    BrokerPortError {
        service,
        method,
        class: BrokerResultClass::Permanent,
        message: message.into(),
        retry_after: None,
    }
}
