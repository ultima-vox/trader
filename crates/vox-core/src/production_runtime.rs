#![allow(
    clippy::result_large_err,
    reason = "production application ports must return the shared by-value ApiError envelope"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ring::digest;
use tokio::sync::Mutex;
use vox_api::application::{
    ConnectionLifecycleObserver, ExecutionCommands, RiskCommands, RiskQueries, RuntimeQueries,
};
use vox_api::binding::{AccountBindingResolver, BindingError};
use vox_api::contract::capability::{
    AttachedBackends, Capability, CapabilitySet, UnavailableCapability,
};
use vox_api::contract::execution::{
    CancelOrderRequest, CancelTarget, JournalStateDto, MutationKindDto, MutationReceiptDto,
    ReplaceOrderRequest, SubmitOrderRequest, SubmitProtectionRequest, SubmitStopOrderRequest,
};
use vox_api::contract::risk::{
    ChangeRiskStateRequest, ReservationStateDto, RiskActionKindDto, RiskDecisionDto,
    RiskLimitUsageDto, RiskOutcomeDto, RiskReasonCodeDto, RiskReasonDto, RiskReservationDto,
    RiskStateDto, RiskStatusDto, RiskValidityDto,
};
use vox_api::contract::runtime::RuntimeHealthDto;
use vox_api::contract::scope::{BrokerEnvironment, ExecutionScope, ProviderDto, TradingMode};
use vox_api::error::{ApiError, ErrorCategory, FieldError};
use vox_api::runtime_scope_from_binding;
use vox_connections::{ConnectionRepository, ExecutionPurpose, SqliteConnectionRepository};
use vox_domain::{
    CancelOrderCommand, CancelStopOrderCommand, ProtectionLeg, ProtectionLegCommand,
    ProviderOrderIdentityKind, RegularOrderCommand, ReplaceOrderCommand, RuntimeExecutionCommand,
    StopLossProtection, TakeProfitProtection, TrailingDistance,
};
use vox_runtime::{
    HealthReadPort, InMemoryMetrics, MutationRecord, OpaqueRef, ReconciliationConfig,
    RuntimeConfig, RuntimeCoordinator, RuntimeError, RuntimeStore, SqliteRuntimeStore,
    StoredCredentialResolver,
};
use vox_tinvest::connection_provider::TInvestConnectionProvider;
use vox_tinvest::{StoredTInvestExecutionPort, StoredTInvestReadPort, StoredTInvestStreamPort};

use crate::composition::{
    ProductionClientFactory, ProductionConnectionService, ProductionSecretStore,
};
use crate::production_risk::ProductionRiskAdapter;

type ProductionCoordinator = RuntimeCoordinator<
    StoredTInvestReadPort<
        SqliteConnectionRepository,
        ProductionSecretStore,
        TInvestConnectionProvider,
    >,
    StoredTInvestExecutionPort<
        SqliteConnectionRepository,
        ProductionSecretStore,
        TInvestConnectionProvider,
    >,
    StoredTInvestStreamPort<
        SqliteConnectionRepository,
        ProductionSecretStore,
        TInvestConnectionProvider,
    >,
    StoredCredentialResolver<
        SqliteConnectionRepository,
        ProductionSecretStore,
        TInvestConnectionProvider,
    >,
    SqliteRuntimeStore,
    InMemoryMetrics,
>;

struct RuntimeEntry {
    coordinator: Arc<ProductionCoordinator>,
    store: SqliteRuntimeStore,
    risk: Arc<ProductionRiskAdapter>,
    public_scope: ExecutionScope,
}

pub struct ProductionRuntimeRegistry {
    repository: SqliteConnectionRepository,
    connections: Arc<ProductionConnectionService>,
    factory: Arc<ProductionClientFactory>,
    resolver: Arc<dyn AccountBindingResolver>,
    runtime_directory: PathBuf,
    environment: BrokerEnvironment,
    entries: Mutex<BTreeMap<String, Arc<RuntimeEntry>>>,
}

impl ProductionRuntimeRegistry {
    #[must_use]
    pub fn new(
        repository: SqliteConnectionRepository,
        connections: Arc<ProductionConnectionService>,
        factory: Arc<ProductionClientFactory>,
        resolver: Arc<dyn AccountBindingResolver>,
        runtime_directory: PathBuf,
        environment: BrokerEnvironment,
    ) -> Self {
        Self {
            repository,
            connections,
            factory,
            resolver,
            runtime_directory,
            environment,
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    async fn entry(&self, scope: &ExecutionScope) -> Result<Arc<RuntimeEntry>, ApiError> {
        validate_public_scope(scope, self.environment)?;
        let binding = self
            .resolver
            .resolve(&scope.account_id, &scope.broker_connection_id)
            .map_err(binding_error)?;
        let runtime_scope = runtime_scope_from_binding(
            scope,
            &binding,
            OpaqueRef::new("credential:resolved-from-stored-binding").map_err(model_error)?,
        )?;
        let key = runtime_scope.key();
        let mut entries = self.entries.lock().await;
        if let Some(entry) = entries.get(&key) {
            return Ok(Arc::clone(entry));
        }
        let store = SqliteRuntimeStore::open_async(self.runtime_path(&key))
            .await
            .map_err(store_error)?;
        let reads = Arc::new(StoredTInvestReadPort::new(Arc::clone(&self.factory)));
        let execution = Arc::new(StoredTInvestExecutionPort::new(Arc::clone(&self.factory)));
        let streams = Arc::new(StoredTInvestStreamPort::new(Arc::clone(&self.factory)));
        let credentials = Arc::new(StoredCredentialResolver::new(Arc::clone(&self.connections)));
        let metrics = Arc::new(InMemoryMetrics::default());
        let risk_store =
            vox_risk::SqliteRiskStore::open(self.runtime_directory.join("risk.sqlite3"))
                .map_err(risk_store_error)?;
        let risk = Arc::new(ProductionRiskAdapter::new(
            scope.account_id.clone(),
            scope.broker_connection_id.clone(),
            store.clone(),
            risk_store,
            Arc::clone(&self.factory),
        ));
        let coordinator = RuntimeCoordinator::new(
            runtime_scope,
            store.clone(),
            reads,
            execution,
            streams,
            credentials,
            risk.clone(),
            metrics,
            ReconciliationConfig::default(),
            RuntimeConfig {
                shutdown_timeout: Duration::from_secs(10),
            },
        );
        coordinator.start().await.map_err(runtime_error)?;
        let entry = Arc::new(RuntimeEntry {
            coordinator,
            store,
            risk,
            public_scope: scope.clone(),
        });
        entries.insert(key, Arc::clone(&entry));
        Ok(entry)
    }

    fn runtime_path(&self, scope_key: &str) -> PathBuf {
        let name = digest::digest(&digest::SHA256, scope_key.as_bytes())
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        self.runtime_directory.join(format!("{name}.sqlite3"))
    }

    pub async fn dispatch_automated(
        &self,
        scope: &ExecutionScope,
        command: RuntimeExecutionCommand,
        logical_request_id: &str,
        correlation_id: &str,
    ) -> Result<MutationReceiptDto, ApiError> {
        let entry = self.entry(scope).await?;
        self.connections
            .validate_runtime_execution_access(
                &vox_connections::ConnectionId::parse(scope.broker_connection_id.clone())
                    .map_err(model_error)?,
                entry.coordinator.broker_account_id(),
                ExecutionPurpose::ProductionAutomated,
                None,
            )
            .map_err(|_| permission_error())?;
        entry
            .coordinator
            .dispatch(command, logical_request_id, correlation_id)
            .await
            .map_err(runtime_error)?;
        self.receipt_from_store(&entry, logical_request_id)
    }

    fn receipt_from_store(
        &self,
        entry: &RuntimeEntry,
        logical_request_id: &str,
    ) -> Result<MutationReceiptDto, ApiError> {
        let record = entry
            .store
            .mutation(&entry.coordinator.scope_key(), logical_request_id)
            .map_err(store_error)?
            .ok_or_else(|| {
                ApiError::new(
                    ErrorCategory::NotFound,
                    "MUTATION_NOT_FOUND",
                    "mutation receipt not found",
                )
            })?;
        let risk_decision = entry
            .risk
            .decision_for_request(logical_request_id)
            .map_err(risk_admission_error)?
            .map(risk_decision_dto);
        mutation_receipt(&entry.store, &entry.public_scope, record, risk_decision)
    }

    pub async fn shutdown(&self) {
        let entries = self
            .entries
            .lock()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for entry in entries {
            let _ = entry.coordinator.shutdown().await;
        }
    }

    async fn invalidate_connection(&self, connection_id: &str) {
        let removed = {
            let mut entries = self.entries.lock().await;
            let keys = entries
                .iter()
                .filter(|(_, entry)| entry.public_scope.broker_connection_id == connection_id)
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>();
            keys.into_iter()
                .filter_map(|key| entries.remove(&key))
                .collect::<Vec<_>>()
        };
        for entry in removed {
            let _ = entry.coordinator.shutdown().await;
        }
    }
}

#[async_trait]
impl ConnectionLifecycleObserver for ProductionRuntimeRegistry {
    async fn connection_changed(&self, connection_id: &str) {
        self.invalidate_connection(connection_id).await;
    }
}

#[async_trait]
impl RuntimeQueries for ProductionRuntimeRegistry {
    async fn health(&self) -> Result<RuntimeHealthDto, ApiError> {
        let entries = self.entries.lock().await;
        if let Some(entry) = entries.values().next() {
            return Ok(RuntimeHealthDto::from(&entry.coordinator.health().await));
        }
        vox_api::application::RuntimeQueries::health(&vox_api::ProcessRuntime::starting(
            ProviderDto::TInvest,
            self.environment,
        ))
        .await
    }

    async fn scoped_health(&self, scope: &ExecutionScope) -> Result<RuntimeHealthDto, ApiError> {
        let entry = self.entry(scope).await?;
        Ok(RuntimeHealthDto::from(&entry.coordinator.health().await))
    }

    async fn scopes(&self) -> Result<Vec<ExecutionScope>, ApiError> {
        let connections = self
            .repository
            .list_connections()
            .map_err(repository_error)?;
        let mut scopes = Vec::new();
        for connection in connections.into_iter().filter(|connection| {
            connection.enabled
                && connection.health.state == vox_connections::ConnectionHealthState::Healthy
                && domain_environment(self.environment) == connection.environment
        }) {
            for binding in self
                .repository
                .bindings(&connection.id)
                .map_err(repository_error)?
                .into_iter()
                .filter(|binding| binding.enabled)
            {
                scopes.push(
                    ExecutionScope::new(
                        ProviderDto::TInvest,
                        self.environment,
                        connection.id.as_str(),
                        binding.vox_account_id.as_str(),
                        TradingMode::Live,
                    )
                    .map_err(|error| {
                        ApiError::new(
                            ErrorCategory::Internal,
                            "STORED_SCOPE_INVALID",
                            error.to_string(),
                        )
                    })?,
                );
            }
        }
        Ok(scopes)
    }

    async fn capabilities(
        &self,
        account_id: Option<&str>,
    ) -> Result<Option<CapabilitySet>, ApiError> {
        let mut set = CapabilitySet::without_backend_owners(
            ProviderDto::TInvest,
            self.environment,
            account_id.map(str::to_owned),
            AttachedBackends {
                runtime: true,
                accounts: true,
                execution: true,
                market_data: false,
                connections: true,
                risk: true,
            },
        );
        let scopes = self.scopes().await?;
        let scoped = account_id.and_then(|account| {
            scopes
                .iter()
                .find(|scope| scope.account_id == account)
                .cloned()
        });
        let Some(scope) = scoped else {
            remove_capability(
                &mut set,
                Capability::AccountReadSide,
                "no healthy explicit account binding exists",
                "#17",
            );
            remove_capability(
                &mut set,
                Capability::OrderExecution,
                "execution scope is not healthy and bound",
                "#17/#11",
            );
            remove_capability(
                &mut set,
                Capability::ProtectionExecution,
                "protection scope is not healthy and bound",
                "#17/#11",
            );
            return Ok(Some(set));
        };
        let binding = self
            .resolver
            .resolve(&scope.account_id, &scope.broker_connection_id)
            .map_err(binding_error)?;
        let connection_id = vox_connections::ConnectionId::parse(scope.broker_connection_id)
            .map_err(model_error)?;
        let authorization = self
            .repository
            .authorization(&connection_id, binding.broker_account_id())
            .map_err(repository_error)?;
        let entry = {
            let entries = self.entries.lock().await;
            entries
                .values()
                .find(|entry| entry.public_scope.account_id == scope.account_id)
                .cloned()
        };
        let runtime_ready = if let Some(entry) = entry {
            let health = entry.coordinator.health().await;
            health.state == vox_runtime::RuntimeState::Ready
                && health.new_exposure_allowed
                && health.execution_authorized
        } else {
            false
        };
        let authorized = authorization.is_some_and(|authorization| {
            authorization.mode != vox_connections::ExecutionAuthorizationMode::Disabled
        });
        if !runtime_ready || !authorized {
            remove_capability(
                &mut set,
                Capability::OrderExecution,
                if !authorized {
                    "Vox execution authorization is disabled"
                } else {
                    "runtime is not READY for new exposure"
                },
                "#17/#11",
            );
            remove_capability(
                &mut set,
                Capability::ProtectionExecution,
                if !authorized {
                    "Vox execution authorization is disabled"
                } else {
                    "runtime is not READY for new exposure"
                },
                "#17/#11",
            );
        }
        Ok(Some(set))
    }
}

#[async_trait]
impl RiskQueries for ProductionRuntimeRegistry {
    async fn risk_status(&self, scope: &ExecutionScope) -> Result<RiskStatusDto, ApiError> {
        let entry = self.entry(scope).await?;
        risk_status_dto(&entry)
    }

    async fn active_reservations(
        &self,
        scope: &ExecutionScope,
    ) -> Result<Vec<RiskReservationDto>, ApiError> {
        let entry = self.entry(scope).await?;
        entry
            .risk
            .active_reservations()
            .map_err(risk_admission_error)?
            .into_iter()
            .map(|reservation| {
                Ok(RiskReservationDto {
                    reservation_id: reservation.reservation_id,
                    scope: scope.clone(),
                    instrument_id: reservation.instrument_id,
                    logical_request_id: reservation.logical_request_id,
                    remaining_delta_lots: reservation.remaining_delta_lots,
                    state: reservation_state_dto(reservation.state),
                    updated_at_unix_ms: reservation.updated_at_unix_ms,
                })
            })
            .collect()
    }
}

#[async_trait]
impl RiskCommands for ProductionRuntimeRegistry {
    async fn change_state(
        &self,
        request: ChangeRiskStateRequest,
    ) -> Result<RiskStatusDto, ApiError> {
        if request.reason.trim().is_empty() {
            return Err(validation(
                "reason",
                "risk state change requires an audit reason",
            ));
        }
        let entry = self.entry(&request.scope).await?;
        entry
            .risk
            .replace_state(
                request.expected_policy_revision,
                risk_state(request.state),
                &request.reason,
            )
            .map_err(risk_admission_error)?;
        risk_status_dto(&entry)
    }
}

#[async_trait]
impl ExecutionCommands for ProductionRuntimeRegistry {
    async fn submit_order(
        &self,
        request: SubmitOrderRequest,
    ) -> Result<MutationReceiptDto, ApiError> {
        if request.protection.is_some() {
            return Err(validation(
                "protection",
                "submit protection through typed protection command after entry acknowledgement",
            ));
        }
        let entry = self.entry(&request.scope).await?;
        let command = RuntimeExecutionCommand::RegularOrder(RegularOrderCommand {
            account_id: entry.coordinator.broker_account_id().to_owned(),
            instrument_id: request.instrument_id,
            client_request_id: request.client_request_id.clone(),
            quantity_lots: request.quantity_lots,
            price: request
                .price
                .as_ref()
                .map(|price| price.to_fixed_point())
                .transpose()
                .map_err(|error| validation("price", &error.to_string()))?,
            price_convention: request.price_convention.into(),
            side: request.side.into(),
            order_type: request.order_type.into(),
            time_in_force: Some(request.time_in_force.into()),
            confirm_margin_trade: request.confirm_margin_trade,
        });
        entry
            .coordinator
            .dispatch_manual(
                command,
                request.client_request_id.clone(),
                uuid::Uuid::new_v4().to_string(),
            )
            .await
            .map_err(runtime_error)?;
        self.receipt_from_store(&entry, &request.client_request_id)
    }

    async fn cancel_order(
        &self,
        request: CancelOrderRequest,
    ) -> Result<MutationReceiptDto, ApiError> {
        let entry = self.entry(&request.scope).await?;
        let (order_id, kind) = cancel_target(request.target()?);
        let command = RuntimeExecutionCommand::CancelOrder(CancelOrderCommand {
            account_id: entry.coordinator.broker_account_id().to_owned(),
            order_id,
            order_id_kind: Some(kind),
        });
        entry
            .coordinator
            .dispatch_manual(
                command,
                request.client_request_id.clone(),
                uuid::Uuid::new_v4().to_string(),
            )
            .await
            .map_err(runtime_error)?;
        self.receipt_from_store(&entry, &request.client_request_id)
    }

    async fn receipt(
        &self,
        scope: &ExecutionScope,
        logical_request_id: &str,
    ) -> Result<MutationReceiptDto, ApiError> {
        let entry = self.entry(scope).await?;
        self.receipt_from_store(&entry, logical_request_id)
    }

    async fn replace_order(
        &self,
        request: ReplaceOrderRequest,
    ) -> Result<MutationReceiptDto, ApiError> {
        let entry = self.entry(&request.scope).await?;
        let (existing_order_id, kind) = cancel_target(request.target()?);
        let price = request
            .price
            .as_ref()
            .ok_or_else(|| validation("price", "replace requires exact price"))?
            .to_fixed_point()
            .map_err(|error| validation("price", &error.to_string()))?;
        let command = RuntimeExecutionCommand::ReplaceOrder(ReplaceOrderCommand {
            account_id: entry.coordinator.broker_account_id().to_owned(),
            existing_order_id,
            existing_order_id_kind: Some(kind),
            replacement_request_id: request.client_request_id.clone(),
            quantity_lots: request.quantity_lots,
            price,
            price_convention: request.price_convention.into(),
            confirm_margin_trade: request.confirm_margin_trade,
        });
        entry
            .coordinator
            .dispatch_manual(
                command,
                request.client_request_id.clone(),
                uuid::Uuid::new_v4().to_string(),
            )
            .await
            .map_err(runtime_error)?;
        self.receipt_from_store(&entry, &request.client_request_id)
    }

    async fn submit_stop_order(
        &self,
        request: SubmitStopOrderRequest,
    ) -> Result<MutationReceiptDto, ApiError> {
        let entry = self.entry(&request.scope).await?;
        let trigger = request
            .trigger_price
            .to_fixed_point()
            .map_err(|error| validation("trigger_price", &error.to_string()))?;
        let limit = request
            .limit_price
            .as_ref()
            .map(|price| price.to_fixed_point())
            .transpose()
            .map_err(|error| validation("limit_price", &error.to_string()))?;
        let reference_price = request
            .reference_price
            .to_fixed_point()
            .map_err(|error| validation("reference_price", &error.to_string()))?;
        let command = RuntimeExecutionCommand::PostStopOrder(ProtectionLegCommand {
            account_id: entry.coordinator.broker_account_id().to_owned(),
            instrument_id: request.instrument_id,
            client_request_id: request.client_request_id.clone(),
            quantity_lots: request.quantity_lots,
            position_side: request.position_side.into(),
            price_convention: request.price_convention.into(),
            reference_price,
            expire_at_unix_seconds: None,
            expire_at_nanos: None,
            confirm_margin_trade: request.confirm_margin_trade,
            leg: ProtectionLeg::StopLoss(StopLossProtection::Fixed {
                trigger_price: trigger,
                limit_price: limit,
            }),
            entry_reservation_id: None,
        });
        entry
            .coordinator
            .dispatch_manual(
                command,
                request.client_request_id.clone(),
                uuid::Uuid::new_v4().to_string(),
            )
            .await
            .map_err(runtime_error)?;
        self.receipt_from_store(&entry, &request.client_request_id)
    }

    async fn cancel_stop_order(
        &self,
        request: CancelOrderRequest,
    ) -> Result<MutationReceiptDto, ApiError> {
        let entry = self.entry(&request.scope).await?;
        let stop_id = match request.target()? {
            CancelTarget::BrokerOrder { broker_order_id } => broker_order_id,
            CancelTarget::LogicalRequest { .. } => {
                return Err(validation(
                    "logical_request_id",
                    "cancel stop requires broker stop identity",
                ));
            }
        };
        let command = RuntimeExecutionCommand::CancelStopOrder(CancelStopOrderCommand {
            account_id: entry.coordinator.broker_account_id().to_owned(),
            broker_stop_order_id: stop_id,
        });
        entry
            .coordinator
            .dispatch_manual(
                command,
                request.client_request_id.clone(),
                uuid::Uuid::new_v4().to_string(),
            )
            .await
            .map_err(runtime_error)?;
        self.receipt_from_store(&entry, &request.client_request_id)
    }

    async fn submit_protection(
        &self,
        request: SubmitProtectionRequest,
    ) -> Result<MutationReceiptDto, ApiError> {
        let entry = self.entry(&request.scope).await?;
        let leg = exact_protection_leg(&request)?;
        let reference_price = request
            .reference_price
            .to_fixed_point()
            .map_err(|error| validation("reference_price", &error.to_string()))?;
        let command = RuntimeExecutionCommand::ProtectionLeg(ProtectionLegCommand {
            account_id: entry.coordinator.broker_account_id().to_owned(),
            instrument_id: request.instrument_id,
            client_request_id: request.client_request_id.clone(),
            quantity_lots: request.quantity_lots,
            position_side: request.position_side.into(),
            price_convention: request.price_convention.into(),
            reference_price,
            expire_at_unix_seconds: None,
            expire_at_nanos: None,
            confirm_margin_trade: request.confirm_margin_trade,
            leg,
            entry_reservation_id: request.entry_reservation_id.clone(),
        });
        entry
            .coordinator
            .dispatch_manual(
                command,
                request.client_request_id.clone(),
                uuid::Uuid::new_v4().to_string(),
            )
            .await
            .map_err(runtime_error)?;
        self.receipt_from_store(&entry, &request.client_request_id)
    }
}

fn exact_protection_leg(request: &SubmitProtectionRequest) -> Result<ProtectionLeg, ApiError> {
    let plan = &request.plan;
    let has_stop = plan.stop_loss_trigger_price.is_some() || plan.stop_loss_trailing.is_some();
    let has_take_profit =
        plan.take_profit_trigger_price.is_some() || plan.take_profit_limit_price.is_some();
    if has_stop == has_take_profit {
        return Err(validation(
            "plan",
            "exactly one protection leg must be submitted per durable mutation",
        ));
    }
    if has_stop {
        return match (&plan.stop_loss_trigger_price, &plan.stop_loss_trailing) {
            (Some(trigger), None) => Ok(ProtectionLeg::StopLoss(StopLossProtection::Fixed {
                trigger_price: trigger.to_fixed_point().map_err(|error| {
                    validation("plan.stop_loss_trigger_price", &error.to_string())
                })?,
                limit_price: None,
            })),
            (None, Some(trailing)) => Ok(ProtectionLeg::StopLoss(StopLossProtection::Trailing {
                distance: trailing_distance(trailing, "plan.stop_loss_trailing")?,
                activation_price: None,
                protective_spread: None,
                instant_execution: None,
            })),
            _ => Err(validation(
                "plan",
                "fixed and trailing stop-loss forms are mutually exclusive",
            )),
        };
    }
    Ok(ProtectionLeg::TakeProfit(TakeProfitProtection {
        trigger_price: plan
            .take_profit_trigger_price
            .as_ref()
            .map(|price| price.to_fixed_point())
            .transpose()
            .map_err(|error| validation("plan.take_profit_trigger_price", &error.to_string()))?,
        limit_price: plan
            .take_profit_limit_price
            .as_ref()
            .map(|price| price.to_fixed_point())
            .transpose()
            .map_err(|error| validation("plan.take_profit_limit_price", &error.to_string()))?,
        trailing: None,
    }))
}

fn trailing_distance(
    value: &vox_api::contract::execution::TrailingDistanceDto,
    field: &'static str,
) -> Result<TrailingDistance, ApiError> {
    Ok(TrailingDistance {
        value: value
            .value
            .to_fixed_point()
            .map_err(|error| validation(field, &error.to_string()))?,
        mode: value.mode.into(),
    })
}

fn cancel_target(target: CancelTarget) -> (String, ProviderOrderIdentityKind) {
    match target {
        CancelTarget::BrokerOrder { broker_order_id } => {
            (broker_order_id, ProviderOrderIdentityKind::BrokerOrder)
        }
        CancelTarget::LogicalRequest { logical_request_id } => {
            (logical_request_id, ProviderOrderIdentityKind::ClientRequest)
        }
    }
}

fn mutation_receipt(
    store: &SqliteRuntimeStore,
    scope: &ExecutionScope,
    record: MutationRecord,
    risk_decision: Option<RiskDecisionDto>,
) -> Result<MutationReceiptDto, ApiError> {
    let links = store
        .all_identity_links(&record.scope_key)
        .map_err(store_error)?
        .into_iter()
        .find(|links| links.logical_request_id == record.logical_request_id);
    let state = JournalStateDto::from(record.state);
    Ok(MutationReceiptDto {
        logical_request_id: record.logical_request_id,
        scope: scope.clone(),
        kind: MutationKindDto::from(record.kind),
        state,
        decision: MutationReceiptDto::decision_for(state),
        risk_decision,
        correlation_id: record.correlation_id,
        broker_order_id: links.as_ref().and_then(|links| {
            links
                .replacement_broker_order_id
                .clone()
                .or_else(|| links.broker_order_id.clone())
        }),
        broker_stop_order_id: links.and_then(|links| links.broker_stop_order_id),
        reconciliation_disposition: record
            .reconciliation_disposition
            .map(|value| format!("{value:?}")),
        runtime_epoch: record.runtime_epoch,
        created_at_unix_ms: record.created_at_unix_ms,
        updated_at_unix_ms: record.updated_at_unix_ms,
    })
}

fn validate_public_scope(
    scope: &ExecutionScope,
    expected_environment: BrokerEnvironment,
) -> Result<(), ApiError> {
    if scope.provider != ProviderDto::TInvest
        || scope.environment != expected_environment
        || scope.trading_mode != TradingMode::Live
    {
        return Err(ApiError::new(
            ErrorCategory::Validation,
            "EXECUTION_SCOPE_MISMATCH",
            "scope does not match configured provider/environment/live mode",
        ));
    }
    Ok(())
}

fn risk_status_dto(entry: &RuntimeEntry) -> Result<RiskStatusDto, ApiError> {
    let policy = entry.risk.policy().map_err(risk_admission_error)?;
    let reservations = entry
        .risk
        .active_reservations()
        .map_err(risk_admission_error)?;
    let reserved_lots = reservations.iter().try_fold(0_i128, |total, reservation| {
        let remaining = i128::from(reservation.remaining_delta_lots)
            .checked_abs()
            .ok_or_else(|| {
                ApiError::new(
                    ErrorCategory::Internal,
                    "RISK_ARITHMETIC_OVERFLOW",
                    "risk reservation utilization overflow",
                )
            })?;
        total.checked_add(remaining).ok_or_else(|| {
            ApiError::new(
                ErrorCategory::Internal,
                "RISK_ARITHMETIC_OVERFLOW",
                "risk reservation utilization overflow",
            )
        })
    })?;
    let reserved_notional = reservations.iter().try_fold(0_i128, |total, reservation| {
        let remaining = reservation
            .reserved_notional_nanos
            .checked_abs()
            .ok_or_else(|| {
                ApiError::new(
                    ErrorCategory::Internal,
                    "RISK_ARITHMETIC_OVERFLOW",
                    "risk reservation notional overflow",
                )
            })?;
        total.checked_add(remaining).ok_or_else(|| {
            ApiError::new(
                ErrorCategory::Internal,
                "RISK_ARITHMETIC_OVERFLOW",
                "risk reservation notional overflow",
            )
        })
    })?;
    let (code, message) = match policy.state {
        vox_risk::RiskState::Normal => (RiskReasonCodeDto::Approved, "normal risk state"),
        vox_risk::RiskState::Warning => (RiskReasonCodeDto::Approved, "warning risk state"),
        vox_risk::RiskState::ReduceOnly => {
            (RiskReasonCodeDto::ReduceOnly, "new exposure is disabled")
        }
        vox_risk::RiskState::Halted => (RiskReasonCodeDto::Halted, "risk state is halted"),
        vox_risk::RiskState::KillSwitch => (
            RiskReasonCodeDto::KillSwitchActive,
            "risk kill switch is active",
        ),
    };
    Ok(RiskStatusDto {
        scope: entry.public_scope.clone(),
        state: risk_state_dto(policy.state),
        policy_revision: policy.revision,
        limits: vec![
            RiskLimitUsageDto {
                name: "active_reserved_lots".into(),
                used: reserved_lots.to_string(),
                limit: policy.max_position_abs_lots.map(|value| value.to_string()),
                unit: "LOTS".into(),
            },
            RiskLimitUsageDto {
                name: "active_reserved_notional".into(),
                used: reserved_notional.to_string(),
                limit: policy
                    .max_gross_exposure_nanos
                    .map(|value| value.to_string()),
                unit: "NANOS".into(),
            },
        ],
        reasons: vec![RiskReasonDto {
            code,
            message: message.into(),
        }],
        updated_at_unix_ms: now_unix_ms_api()?,
    })
}

fn risk_state(value: RiskStateDto) -> vox_risk::RiskState {
    match value {
        RiskStateDto::Normal => vox_risk::RiskState::Normal,
        RiskStateDto::Warning => vox_risk::RiskState::Warning,
        RiskStateDto::ReduceOnly => vox_risk::RiskState::ReduceOnly,
        RiskStateDto::Halted => vox_risk::RiskState::Halted,
        RiskStateDto::KillSwitch => vox_risk::RiskState::KillSwitch,
    }
}

fn risk_decision_dto(value: vox_risk::RiskDecision) -> RiskDecisionDto {
    RiskDecisionDto {
        decision_id: value.decision_id,
        policy_revision: value.policy_revision,
        action: match value.action {
            vox_risk::RiskActionKind::DirectionalOrder => RiskActionKindDto::DirectionalOrder,
            vox_risk::RiskActionKind::ReplaceDirectionalOrder => {
                RiskActionKindDto::ReplaceDirectionalOrder
            }
            vox_risk::RiskActionKind::CancelOrder => RiskActionKindDto::CancelOrder,
            vox_risk::RiskActionKind::ProtectionMaintenance => {
                RiskActionKindDto::ProtectionMaintenance
            }
            vox_risk::RiskActionKind::CancelProtection => RiskActionKindDto::CancelProtection,
        },
        requested_delta_lots: value.requested_delta_lots,
        approved_delta_lots: value.approved_delta_lots,
        outcome: match value.outcome {
            vox_risk::RiskOutcome::Approve => RiskOutcomeDto::Approve,
            vox_risk::RiskOutcome::Resize => RiskOutcomeDto::Resize,
            vox_risk::RiskOutcome::Reject => RiskOutcomeDto::Reject,
            vox_risk::RiskOutcome::ReduceOnly => RiskOutcomeDto::ReduceOnly,
            vox_risk::RiskOutcome::Halt => RiskOutcomeDto::Halt,
        },
        reasons: value
            .reasons
            .into_iter()
            .map(|reason| RiskReasonDto {
                code: risk_reason_code_dto(reason.code),
                message: reason.message,
            })
            .collect(),
        reservation_id: value.reservation_id,
        validity: RiskValidityDto {
            runtime_epoch: value.validity.runtime_epoch,
            reconciliation_revision: value.validity.reconciliation_revision,
            position_revision: value.validity.position_revision,
            order_revision: value.validity.order_revision,
            market_data_as_of_unix_ms: value.validity.market_data_as_of_unix_ms,
            instrument_constraints_revision: value.validity.instrument_constraints_revision,
            policy_revision: value.validity.policy_revision,
            execution_authorization_revision: value.validity.execution_authorization_revision,
        },
    }
}

fn risk_reason_code_dto(value: vox_risk::RiskReasonCode) -> RiskReasonCodeDto {
    match value {
        vox_risk::RiskReasonCode::Approved => RiskReasonCodeDto::Approved,
        vox_risk::RiskReasonCode::ResizedToProviderLimit => {
            RiskReasonCodeDto::ResizedToProviderLimit
        }
        vox_risk::RiskReasonCode::ResizedToPolicyLimit => RiskReasonCodeDto::ResizedToPolicyLimit,
        vox_risk::RiskReasonCode::InvalidQuantity => RiskReasonCodeDto::InvalidQuantity,
        vox_risk::RiskReasonCode::InstrumentUnavailable => RiskReasonCodeDto::InstrumentUnavailable,
        vox_risk::RiskReasonCode::InstrumentNotTradable => RiskReasonCodeDto::InstrumentNotTradable,
        vox_risk::RiskReasonCode::PriceUnavailable => RiskReasonCodeDto::PriceUnavailable,
        vox_risk::RiskReasonCode::PositionLotMismatch => RiskReasonCodeDto::PositionLotMismatch,
        vox_risk::RiskReasonCode::CriticalInputMissing => RiskReasonCodeDto::CriticalInputMissing,
        vox_risk::RiskReasonCode::RuntimeNotReady => RiskReasonCodeDto::RuntimeNotReady,
        vox_risk::RiskReasonCode::ExecutionUnauthorized => RiskReasonCodeDto::ExecutionUnauthorized,
        vox_risk::RiskReasonCode::AuthorizationRevisionChanged => {
            RiskReasonCodeDto::AuthorizationRevisionChanged
        }
        vox_risk::RiskReasonCode::PolicyRevisionChanged => RiskReasonCodeDto::PolicyRevisionChanged,
        vox_risk::RiskReasonCode::ReconciliationRevisionChanged => {
            RiskReasonCodeDto::ReconciliationRevisionChanged
        }
        vox_risk::RiskReasonCode::PositionRevisionChanged => {
            RiskReasonCodeDto::PositionRevisionChanged
        }
        vox_risk::RiskReasonCode::OrderRevisionChanged => RiskReasonCodeDto::OrderRevisionChanged,
        vox_risk::RiskReasonCode::InstrumentConstraintRevisionChanged => {
            RiskReasonCodeDto::InstrumentConstraintRevisionChanged
        }
        vox_risk::RiskReasonCode::MarketDataMissing => RiskReasonCodeDto::MarketDataMissing,
        vox_risk::RiskReasonCode::MarketDataStale => RiskReasonCodeDto::MarketDataStale,
        vox_risk::RiskReasonCode::UnknownMutationConflict => {
            RiskReasonCodeDto::UnknownMutationConflict
        }
        vox_risk::RiskReasonCode::ReduceOnly => RiskReasonCodeDto::ReduceOnly,
        vox_risk::RiskReasonCode::Halted => RiskReasonCodeDto::Halted,
        vox_risk::RiskReasonCode::MarginNotAllowed => RiskReasonCodeDto::MarginNotAllowed,
        vox_risk::RiskReasonCode::MarginConfirmationRequired => {
            RiskReasonCodeDto::MarginConfirmationRequired
        }
        vox_risk::RiskReasonCode::MarginUtilizationExceeded => {
            RiskReasonCodeDto::MarginUtilizationExceeded
        }
        vox_risk::RiskReasonCode::ProviderLimitUnavailable => {
            RiskReasonCodeDto::ProviderLimitUnavailable
        }
        vox_risk::RiskReasonCode::ProviderLimitExceeded => RiskReasonCodeDto::ProviderLimitExceeded,
        vox_risk::RiskReasonCode::MaxSingleOrderExceeded => {
            RiskReasonCodeDto::MaxSingleOrderExceeded
        }
        vox_risk::RiskReasonCode::MaxPositionExceeded => RiskReasonCodeDto::MaxPositionExceeded,
        vox_risk::RiskReasonCode::MaxGrossExposureExceeded => {
            RiskReasonCodeDto::MaxGrossExposureExceeded
        }
        vox_risk::RiskReasonCode::MaxNetExposureExceeded => {
            RiskReasonCodeDto::MaxNetExposureExceeded
        }
        vox_risk::RiskReasonCode::MaxInstrumentExposureExceeded => {
            RiskReasonCodeDto::MaxInstrumentExposureExceeded
        }
        vox_risk::RiskReasonCode::DailyLossExceeded => RiskReasonCodeDto::DailyLossExceeded,
        vox_risk::RiskReasonCode::ProtectionRequired => RiskReasonCodeDto::ProtectionRequired,
        vox_risk::RiskReasonCode::KillSwitchActive => RiskReasonCodeDto::KillSwitchActive,
        vox_risk::RiskReasonCode::PersistenceFailure => RiskReasonCodeDto::PersistenceFailure,
    }
}

fn risk_state_dto(value: vox_risk::RiskState) -> RiskStateDto {
    match value {
        vox_risk::RiskState::Normal => RiskStateDto::Normal,
        vox_risk::RiskState::Warning => RiskStateDto::Warning,
        vox_risk::RiskState::ReduceOnly => RiskStateDto::ReduceOnly,
        vox_risk::RiskState::Halted => RiskStateDto::Halted,
        vox_risk::RiskState::KillSwitch => RiskStateDto::KillSwitch,
    }
}

fn reservation_state_dto(value: vox_risk::ReservationState) -> ReservationStateDto {
    match value {
        vox_risk::ReservationState::Active => ReservationStateDto::Active,
        vox_risk::ReservationState::PartiallyConsumed => ReservationStateDto::PartiallyConsumed,
        vox_risk::ReservationState::Consumed => ReservationStateDto::Consumed,
        vox_risk::ReservationState::Released => ReservationStateDto::Released,
        vox_risk::ReservationState::UnknownHeld => ReservationStateDto::UnknownHeld,
        vox_risk::ReservationState::Orphaned => ReservationStateDto::Orphaned,
    }
}

fn now_unix_ms_api() -> Result<i64, ApiError> {
    i64::try_from(time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000).map_err(
        |_| {
            ApiError::new(
                ErrorCategory::Internal,
                "SYSTEM_CLOCK_INVALID",
                "system clock is out of range",
            )
        },
    )
}

fn domain_environment(value: BrokerEnvironment) -> vox_connections::BrokerEnvironment {
    match value {
        BrokerEnvironment::Sandbox => vox_connections::BrokerEnvironment::Sandbox,
        BrokerEnvironment::Production => vox_connections::BrokerEnvironment::Production,
    }
}

fn validation(field: &'static str, message: &str) -> ApiError {
    ApiError::validation(
        "execution request is invalid",
        vec![FieldError {
            field: field.to_owned(),
            message: message.to_owned(),
        }],
    )
}

fn permission_error() -> ApiError {
    ApiError::new(
        ErrorCategory::Permission,
        "EXECUTION_NOT_AUTHORIZED",
        "execution is not authorized for exact stored scope and purpose",
    )
}

fn binding_error(error: BindingError) -> ApiError {
    ApiError::new(
        ErrorCategory::Conflict,
        "ACCOUNT_BINDING_INVALID",
        error.to_string(),
    )
}

fn model_error(error: impl core::fmt::Display) -> ApiError {
    ApiError::new(
        ErrorCategory::Validation,
        "INVALID_SCOPE",
        error.to_string(),
    )
}

fn repository_error(_: vox_connections::RepositoryError) -> ApiError {
    ApiError::new(
        ErrorCategory::Transient,
        "PLATFORM_PERSISTENCE_UNAVAILABLE",
        "platform persistence unavailable",
    )
}

fn store_error(error: vox_runtime::StoreError) -> ApiError {
    ApiError::new(
        ErrorCategory::Internal,
        "RUNTIME_PERSISTENCE_FAILURE",
        error.to_string(),
    )
}

fn risk_store_error(error: vox_risk::RiskStoreError) -> ApiError {
    ApiError::new(
        ErrorCategory::Internal,
        "RISK_PERSISTENCE_FAILURE",
        error.to_string(),
    )
}

fn risk_admission_error(error: vox_runtime::RiskAdmissionError) -> ApiError {
    match error {
        vox_runtime::RiskAdmissionError::Denied { code, message } => {
            ApiError::new(ErrorCategory::Conflict, code, message)
        }
        vox_runtime::RiskAdmissionError::Stale(message) => ApiError::new(
            ErrorCategory::Stale,
            "RISK_POLICY_REVISION_CHANGED",
            message,
        ),
        vox_runtime::RiskAdmissionError::Unavailable(message) => {
            ApiError::new(ErrorCategory::Transient, "RISK_ENGINE_UNAVAILABLE", message)
        }
    }
}

fn runtime_error(error: RuntimeError) -> ApiError {
    let category = match &error {
        RuntimeError::ExecutionGateClosed | RuntimeError::Safety(_) => ErrorCategory::Conflict,
        RuntimeError::RiskAdmission(vox_runtime::RiskAdmissionError::Denied { .. }) => {
            ErrorCategory::Conflict
        }
        RuntimeError::RiskAdmission(vox_runtime::RiskAdmissionError::Stale(_)) => {
            ErrorCategory::Stale
        }
        RuntimeError::RiskAdmission(vox_runtime::RiskAdmissionError::Unavailable(_)) => {
            ErrorCategory::Transient
        }
        RuntimeError::Broker(_) => ErrorCategory::Transient,
        RuntimeError::CommandQueueFull => ErrorCategory::Transient,
        _ => ErrorCategory::Internal,
    };
    ApiError::new(category, "RUNTIME_EXECUTION_FAILED", error.to_string())
}

fn remove_capability(set: &mut CapabilitySet, capability: Capability, reason: &str, owner: &str) {
    set.supported.retain(|candidate| *candidate != capability);
    if !set
        .unavailable
        .iter()
        .any(|candidate| candidate.capability == capability)
    {
        set.unavailable.push(UnavailableCapability {
            capability,
            reason: reason.to_owned(),
            owner: owner.to_owned(),
        });
    }
}
