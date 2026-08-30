//! Capability-restricted #9/#11 read port over the existing generated T-Invest client.

use std::collections::BTreeMap;

use async_trait::async_trait;
use vox_connections::BrokerEnvironment;
use vox_runtime::{
    BrokerAccount, BrokerPortError, BrokerReadPort, BrokerResultClass, MoneyFact, OperationFact,
    OperationsPage, OrderExecutionStatus, OrderFact, PortfolioFact, PositionsFact, RuntimeScope,
    StopExecutionStatus, StopFact,
};

use crate::account::{AccountReadClient, PortfolioQuery};
use crate::execution::{canonical_orders, canonical_stop_orders};
use crate::generated::v1;
use crate::{GrpcError, GrpcErrorKind, TInvestGrpcClient};

#[derive(Clone)]
pub struct TInvestRuntimeReadAdapter {
    client: TInvestGrpcClient,
    environment: BrokerEnvironment,
}

impl TInvestRuntimeReadAdapter {
    #[must_use]
    pub(crate) const fn new(client: TInvestGrpcClient, environment: BrokerEnvironment) -> Self {
        Self {
            client,
            environment,
        }
    }
}

#[async_trait]
impl BrokerReadPort for TInvestRuntimeReadAdapter {
    async fn accounts(&self, _: &RuntimeScope) -> Result<Vec<BrokerAccount>, BrokerPortError> {
        let reads = AccountReadClient::new(self.client.clone());
        let catalogue = match self.environment {
            BrokerEnvironment::Sandbox => reads.sandbox_accounts().await,
            BrokerEnvironment::Production => reads.accounts(None).await,
        }
        .map_err(|error| port_error("UsersService", "GetAccounts", error))?;
        Ok(catalogue
            .accounts
            .into_iter()
            .map(|account| BrokerAccount {
                account_id: account.account_id,
                open: account.status == v1::AccountStatus::Open as i32,
                accessible: account.status == v1::AccountStatus::Open as i32
                    && account.access_level != v1::AccessLevel::AccountAccessLevelNoAccess as i32,
            })
            .collect())
    }

    async fn portfolio(&self, scope: &RuntimeScope) -> Result<PortfolioFact, BrokerPortError> {
        let reads = AccountReadClient::new(self.client.clone());
        let portfolio = match self.environment {
            BrokerEnvironment::Sandbox => {
                reads
                    .sandbox_portfolio(scope.broker_account_id.clone())
                    .await
            }
            BrokerEnvironment::Production => {
                reads
                    .portfolio(PortfolioQuery {
                        account_id: scope.broker_account_id.clone(),
                        currency: None,
                    })
                    .await
            }
        }
        .map_err(|error| port_error("OperationsService", "GetPortfolio", error))?;
        Ok(PortfolioFact {
            account_id: portfolio
                .account_id
                .unwrap_or_else(|| scope.broker_account_id.clone()),
            total_portfolio_valuation: portfolio.total_amount_portfolio.map(money_fact),
            total_currency_valuation: portfolio.total_amount_currencies.map(money_fact),
            cash_balances: BTreeMap::new(),
            broker_observed_at_unix_ms: None,
        })
    }

    async fn positions(&self, scope: &RuntimeScope) -> Result<PositionsFact, BrokerPortError> {
        let reads = AccountReadClient::new(self.client.clone());
        let positions = match self.environment {
            BrokerEnvironment::Sandbox => {
                reads
                    .sandbox_positions(scope.broker_account_id.clone())
                    .await
            }
            BrokerEnvironment::Production => reads.positions(scope.broker_account_id.clone()).await,
        }
        .map_err(|error| port_error("OperationsService", "GetPositions", error))?;
        let cash_balances = positions
            .money
            .iter()
            .filter_map(|money| {
                money.currency.clone().map(|currency| {
                    (
                        currency,
                        money.amount.fixed_point().total_nanos().to_string(),
                    )
                })
            })
            .collect();
        let mut instruments = Vec::new();
        for position in positions.securities {
            if let Some(instrument_uid) = position.identity.instrument_uid {
                instruments.push(vox_runtime::PositionFact {
                    account_id: scope.broker_account_id.clone(),
                    instrument_uid,
                    quantity_units: position.balance,
                    broker_observed_at_unix_ms: None,
                });
            }
        }
        for position in positions.futures {
            if let Some(instrument_uid) = position.identity.instrument_uid {
                instruments.push(vox_runtime::PositionFact {
                    account_id: scope.broker_account_id.clone(),
                    instrument_uid,
                    quantity_units: position.balance,
                    broker_observed_at_unix_ms: None,
                });
            }
        }
        for position in positions.options {
            if let Some(instrument_uid) = position.identity.instrument_uid {
                instruments.push(vox_runtime::PositionFact {
                    account_id: scope.broker_account_id.clone(),
                    instrument_uid,
                    quantity_units: position.balance,
                    broker_observed_at_unix_ms: None,
                });
            }
        }
        Ok(PositionsFact {
            instruments,
            cash_balances,
        })
    }

    async fn active_orders(&self, scope: &RuntimeScope) -> Result<Vec<OrderFact>, BrokerPortError> {
        let request = v1::GetOrdersRequest {
            account_id: scope.broker_account_id.clone(),
            advanced_filters: None,
        };
        let body = match self.environment {
            BrokerEnvironment::Sandbox => self.client.get_sandbox_orders(request).await,
            BrokerEnvironment::Production => self.client.get_orders(request).await,
        }
        .map_err(|error| grpc_port_error("OrdersService", "GetOrders", error))?
        .body;
        canonical_orders(body)
            .map_err(|error| port_error("OrdersService", "GetOrders", error))?
            .into_iter()
            .map(|order| order_fact(scope, order))
            .collect()
    }

    async fn stop_orders(
        &self,
        scope: &RuntimeScope,
        include_terminal_since_unix_ms: i64,
    ) -> Result<Vec<StopFact>, BrokerPortError> {
        let request = v1::GetStopOrdersRequest {
            account_id: scope.broker_account_id.clone(),
            status: v1::StopOrderStatusOption::StopOrderStatusAll as i32,
            from: Some(timestamp(include_terminal_since_unix_ms)),
            to: None,
        };
        let body = match self.environment {
            BrokerEnvironment::Sandbox => self.client.get_sandbox_stop_orders(request).await,
            BrokerEnvironment::Production => self.client.get_stop_orders(request).await,
        }
        .map_err(|error| grpc_port_error("StopOrdersService", "GetStopOrders", error))?
        .body;
        canonical_stop_orders(body)
            .map_err(|error| port_error("StopOrdersService", "GetStopOrders", error))?
            .into_iter()
            .map(|stop| {
                Ok(StopFact {
                    account_id: scope.broker_account_id.clone(),
                    broker_stop_order_id: stop.broker_stop_order_id.ok_or_else(|| {
                        permanent(
                            "StopOrdersService",
                            "GetStopOrders",
                            "stop omitted identity",
                        )
                    })?,
                    instrument_uid: stop.instrument_uid.unwrap_or_default(),
                    status: stop_status(stop.status),
                    status_cause: None,
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
        let request = v1::GetOrderStateRequest {
            account_id: scope.broker_account_id.clone(),
            order_id: order_id.to_owned(),
            price_type: v1::PriceType::Currency as i32,
            order_id_type: Some(order_id_type as i32),
        };
        let response = match self.environment {
            BrokerEnvironment::Sandbox => self.client.get_sandbox_order_state(request).await,
            BrokerEnvironment::Production => self.client.get_order_state(request).await,
        };
        match response {
            Ok(response) => response
                .body
                .try_into()
                .map_err(|error| port_error("OrdersService", "GetOrderState", error))
                .and_then(|order| order_fact(scope, order).map(Some)),
            Err(error) if provider_not_found(&error) => Ok(None),
            Err(error) => Err(grpc_port_error("OrdersService", "GetOrderState", error)),
        }
    }

    async fn operations_page(
        &self,
        scope: &RuntimeScope,
        cursor: Option<&str>,
        from_unix_ms: i64,
        limit: u16,
    ) -> Result<OperationsPage, BrokerPortError> {
        let request = v1::GetOperationsByCursorRequest {
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
        };
        let body = match self.environment {
            BrokerEnvironment::Sandbox => {
                self.client.get_sandbox_operations_by_cursor(request).await
            }
            BrokerEnvironment::Production => self.client.get_operations_by_cursor(request).await,
        }
        .map_err(|error| grpc_port_error("OperationsService", "GetOperationsByCursor", error))?
        .body;
        let next_cursor = body.has_next.then_some(body.next_cursor);
        let items = body
            .items
            .into_iter()
            .enumerate()
            .map(|(index, item)| {
                let operation: crate::operations::CanonicalOperation =
                    item.try_into().map_err(|error| {
                        port_error("OperationsService", "GetOperationsByCursor", error)
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

fn order_fact(
    scope: &RuntimeScope,
    order: crate::execution::CanonicalOrderState,
) -> Result<OrderFact, BrokerPortError> {
    Ok(OrderFact {
        account_id: scope.broker_account_id.clone(),
        broker_order_id: order
            .broker_order_id
            .ok_or_else(|| permanent("OrdersService", "order_decode", "order omitted identity"))?,
        logical_request_id: order.client_request_id,
        instrument_uid: order.instrument_uid.unwrap_or_default(),
        status: order_status(order.execution_status),
        status_cause: None,
    })
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

fn money_fact(value: crate::canonical::CanonicalMoney) -> MoneyFact {
    MoneyFact {
        currency: value.currency.unwrap_or_default(),
        amount_nanos: value.amount.fixed_point().total_nanos().to_string(),
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
    permanent(service, method, error.to_string())
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

fn timestamp(unix_ms: i64) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: unix_ms.div_euclid(1_000),
        nanos: i32::try_from(unix_ms.rem_euclid(1_000) * 1_000_000).unwrap_or_default(),
    }
}
