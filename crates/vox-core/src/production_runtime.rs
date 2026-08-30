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
use vox_api::application::{ConnectionLifecycleObserver, ExecutionCommands, RuntimeQueries};
use vox_api::binding::{AccountBindingResolver, BindingError};
use vox_api::contract::capability::{
    AttachedBackends, Capability, CapabilitySet, UnavailableCapability,
};
use vox_api::contract::execution::{
    CancelOrderRequest, CancelTarget, JournalStateDto, MutationKindDto, MutationReceiptDto,
    ReplaceOrderRequest, SubmitOrderRequest, SubmitProtectionRequest, SubmitStopOrderRequest,
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
    RiskAdmission, RiskAdmissionError, RiskAdmissionPort, RuntimeConfig, RuntimeCoordinator,
    RuntimeError, RuntimeExecutionPurpose, RuntimeStore, SqliteRuntimeStore, StoredCredentialResolver,
};
use vox_tinvest::connection_provider::TInvestConnectionProvider;
use vox_tinvest::{StoredTInvestExecutionPort, StoredTInvestReadPort, StoredTInvestStreamPort};

use crate::composition::{
    ProductionClientFactory, ProductionConnectionService, ProductionSecretStore,
};

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

struct FailClosedRiskAdmission;

#[async_trait]
impl RiskAdmissionPort for FailClosedRiskAdmission {
    async fn admit(
        &self,
        _: &vox_runtime::RuntimeScope,
        _: RuntimeExecutionPurpose,
        _: &RuntimeExecutionCommand,
        _: &str,
    ) -> Result<RiskAdmission, RiskAdmissionError> {
        Err(RiskAdmissionError::Unavailable(
            "#21 production risk application adapter is not wired yet".into(),
        ))
    }
}

struct RuntimeEntry {
    coordinator: Arc<ProductionCoordinator>,
    store: SqliteRuntimeStore,
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
        let coordinator = RuntimeCoordinator::new(
            runtime_scope,
            store.clone(),
            reads,
            execution,
            streams,
            credentials,
            Arc::new(FailClosedRiskAdmission),
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
        mutation_receipt(&entry.store, &entry.public_scope, record)
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

fn runtime_error(error: RuntimeError) -> ApiError {
    let category = match &error {
        RuntimeError::ExecutionGateClosed | RuntimeError::Safety(_) => ErrorCategory::Conflict,
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
