//! Attach accepted #11 runtime ports to the public application boundary.
//!
//! Mapping is exhaustive against `vox-runtime` types. Canonical `account_id` never becomes
//! a `RuntimeScope.broker_account_id` except through an [`AccountBindingResolver`].
//! Connection identity converts only through
//! [`connection_ref_from_broker_connection_id`].

use std::sync::Arc;

use async_trait::async_trait;
use vox_runtime::{
    BrokerPortError, BrokerReadPort, BrokerResultClass, HealthReadPort, OpaqueRef, ReasonCode,
    ReconciliationCheckpoint, RuntimeHealth, RuntimeScope, RuntimeState, RuntimeStore, StoreError,
    StreamHealth, StreamKind, StreamState,
};

use crate::application::{AccountQueries, RuntimeQueries};
use crate::binding::{
    AccountBinding, AccountBindingResolver, BindingError, connection_ref_from_broker_connection_id,
};
use crate::contract::account::{
    BrokerAccountDto, OperationsPageDto, OrderDto, PortfolioDto, PositionDto, ReconciliationDto,
    StopOrderDto,
};
use crate::contract::runtime::RuntimeHealthDto;
use crate::contract::scope::{BrokerEnvironment, ExecutionScope, ProviderDto};
use crate::error::{ApiError, ErrorCategory};

type CheckpointLoader =
    Arc<dyn Fn(&str) -> Result<Option<ReconciliationCheckpoint>, StoreError> + Send + Sync>;

/// Process-local runtime health from the accepted #11 `RuntimeHealth` type.
///
/// Used when the API process has no broker connection yet: the contract is attached, the
/// snapshot is `STARTING`, and no account data is invented. Health does not require an
/// account binding.
pub struct ProcessRuntime {
    health: RuntimeHealth,
}

impl ProcessRuntime {
    #[must_use]
    pub fn starting(provider: ProviderDto, environment: BrokerEnvironment) -> Self {
        Self {
            health: RuntimeHealth {
                state: RuntimeState::Starting,
                reason_code: ReasonCode::Startup,
                reason: "runtime created; no broker reconciliation yet".into(),
                provider: provider.into(),
                environment: environment.into(),
                account_display: "unbound".into(),
                runtime_epoch: 0,
                connected: false,
                last_successful_reconciliation_at_unix_ms: None,
                reconciliation_age_ms: None,
                unresolved_unknown_count: 0,
                open_order_count: 0,
                active_stop_count: 0,
                stream_states: disconnected_streams(),
                persistence_healthy: true,
                execution_authorized: false,
                new_exposure_allowed: false,
            },
        }
    }
}

#[async_trait]
impl HealthReadPort for ProcessRuntime {
    async fn health(&self) -> RuntimeHealth {
        self.health.clone()
    }
}

#[async_trait]
impl RuntimeQueries for ProcessRuntime {
    async fn health(&self) -> Result<RuntimeHealthDto, ApiError> {
        Ok(RuntimeHealthDto::from(&HealthReadPort::health(self).await))
    }

    async fn scopes(&self) -> Result<Vec<crate::contract::scope::ExecutionScope>, ApiError> {
        Ok(Vec::new())
    }
}

/// Runtime health adapter over any accepted `HealthReadPort`, including `RuntimeCoordinator`.
pub struct RuntimeHealthAdapter<H> {
    inner: Arc<H>,
}

impl<H> RuntimeHealthAdapter<H> {
    #[must_use]
    pub const fn new(inner: Arc<H>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl<H> RuntimeQueries for RuntimeHealthAdapter<H>
where
    H: HealthReadPort + 'static,
{
    async fn health(&self) -> Result<RuntimeHealthDto, ApiError> {
        Ok(RuntimeHealthDto::from(&self.inner.health().await))
    }

    async fn scopes(&self) -> Result<Vec<crate::contract::scope::ExecutionScope>, ApiError> {
        Ok(Vec::new())
    }
}

/// Account read-side adapter: public scope → binding → runtime scope → broker facts.
pub struct AccountReadAdapter<R> {
    resolver: Arc<dyn AccountBindingResolver>,
    reads: Arc<R>,
    opaque_credential_ref: OpaqueRef,
    load_checkpoint: Option<CheckpointLoader>,
}

impl<R> AccountReadAdapter<R> {
    #[must_use]
    pub fn new(
        resolver: Arc<dyn AccountBindingResolver>,
        reads: Arc<R>,
        opaque_credential_ref: OpaqueRef,
    ) -> Self {
        Self {
            resolver,
            reads,
            opaque_credential_ref,
            load_checkpoint: None,
        }
    }

    #[must_use]
    pub fn with_store<S>(self, store: S) -> Self
    where
        S: RuntimeStore,
    {
        Self {
            load_checkpoint: Some(Arc::new(move |key: &str| store.load_checkpoint(key))),
            ..self
        }
    }

    fn runtime_scope(&self, requested: &ExecutionScope) -> Result<RuntimeScope, ApiError> {
        let binding = self
            .resolver
            .resolve(&requested.account_id, &requested.broker_connection_id)
            .map_err(map_binding_error)?;
        runtime_scope_from_binding(requested, &binding, self.opaque_credential_ref.clone())
    }
}

/// Builds a `RuntimeScope` from a public scope and an **already resolved** binding.
///
/// Broker account identity comes only from `binding.broker_account_id`. Connection
/// identity converts only through `connection_ref_from_broker_connection_id`.
/// Credential material is an opaque composition-time ref, never taken from the request.
pub fn runtime_scope_from_binding(
    scope: &ExecutionScope,
    binding: &AccountBinding,
    opaque_credential_ref: OpaqueRef,
) -> Result<RuntimeScope, ApiError> {
    if scope.account_id != binding.account_id()
        || scope.broker_connection_id != binding.broker_connection_id()
    {
        return Err(ApiError::new(
            ErrorCategory::Internal,
            "BINDING_SCOPE_MISMATCH",
            "resolved binding does not match the requested public scope",
        ));
    }
    let connection_ref = connection_ref_from_broker_connection_id(&scope.broker_connection_id)
        .map_err(|error| {
            ApiError::validation(
                error.to_string(),
                vec![crate::error::FieldError {
                    field: "broker_connection_id".to_owned(),
                    message: error.to_string(),
                }],
            )
        })?;
    RuntimeScope::new(
        scope.provider.into(),
        scope.environment.into(),
        binding.broker_account_id().to_owned(),
        connection_ref,
        binding
            .credential_ref()
            .cloned()
            .unwrap_or(opaque_credential_ref),
    )
    .map_err(|error| {
        ApiError::new(
            ErrorCategory::Validation,
            "INVALID_SCOPE",
            error.to_string(),
        )
    })
}

#[async_trait]
impl<R> AccountQueries for AccountReadAdapter<R>
where
    R: BrokerReadPort + 'static,
{
    async fn accounts(&self, scope: &ExecutionScope) -> Result<Vec<BrokerAccountDto>, ApiError> {
        let binding = self
            .resolver
            .resolve(&scope.account_id, &scope.broker_connection_id)
            .map_err(map_binding_error)?;
        let runtime_scope =
            runtime_scope_from_binding(scope, &binding, self.opaque_credential_ref.clone())?;
        let accounts = self
            .reads
            .accounts(&runtime_scope)
            .await
            .map_err(map_broker_error)?;
        let matching = accounts
            .iter()
            .find(|fact| fact.account_id == binding.broker_account_id())
            .ok_or_else(|| {
                ApiError::new(
                    ErrorCategory::Conflict,
                    "BROKER_ACCOUNT_ACCESS_CHANGED",
                    "bound broker account is absent from authoritative account discovery",
                )
            })?;
        Ok(vec![BrokerAccountDto::from_bound_fact(&binding, matching)?])
    }

    async fn portfolio(&self, scope: &ExecutionScope) -> Result<PortfolioDto, ApiError> {
        let binding = self
            .resolver
            .resolve(&scope.account_id, &scope.broker_connection_id)
            .map_err(map_binding_error)?;
        let runtime_scope =
            runtime_scope_from_binding(scope, &binding, self.opaque_credential_ref.clone())?;
        let fact = self
            .reads
            .portfolio(&runtime_scope)
            .await
            .map_err(map_broker_error)?;
        PortfolioDto::from_bound_fact(&binding, &fact)
    }

    async fn positions(&self, scope: &ExecutionScope) -> Result<Vec<PositionDto>, ApiError> {
        let (binding, runtime_scope) = self.bound_runtime(scope)?;
        let facts = self
            .reads
            .positions(&runtime_scope)
            .await
            .map_err(map_broker_error)?;
        facts
            .instruments
            .iter()
            .map(|fact| PositionDto::from_bound_fact(&binding, fact))
            .collect()
    }

    async fn orders(&self, scope: &ExecutionScope) -> Result<Vec<OrderDto>, ApiError> {
        let (binding, runtime_scope) = self.bound_runtime(scope)?;
        let facts = self
            .reads
            .active_orders(&runtime_scope)
            .await
            .map_err(map_broker_error)?;
        facts
            .iter()
            .map(|fact| OrderDto::from_bound_fact(&binding, fact))
            .collect()
    }

    async fn stop_orders(&self, scope: &ExecutionScope) -> Result<Vec<StopOrderDto>, ApiError> {
        let (binding, runtime_scope) = self.bound_runtime(scope)?;
        let facts = self
            .reads
            .stop_orders(&runtime_scope, 0)
            .await
            .map_err(map_broker_error)?;
        facts
            .iter()
            .map(|fact| StopOrderDto::from_bound_fact(&binding, fact))
            .collect()
    }

    async fn operations(
        &self,
        scope: &ExecutionScope,
        cursor: Option<&str>,
        limit: u16,
    ) -> Result<OperationsPageDto, ApiError> {
        let (binding, runtime_scope) = self.bound_runtime(scope)?;
        let page = self
            .reads
            .operations_page(&runtime_scope, cursor, 0, limit)
            .await
            .map_err(map_broker_error)?;
        OperationsPageDto::from_bound_page(&binding, &page)
    }

    async fn reconciliation(&self, scope: &ExecutionScope) -> Result<ReconciliationDto, ApiError> {
        let runtime_scope = self.runtime_scope(scope)?;
        let Some(load_checkpoint) = self.load_checkpoint.as_ref() else {
            return Err(ApiError::new(
                ErrorCategory::NotFound,
                "RECONCILIATION_UNAVAILABLE",
                "no runtime store is attached to serve a reconciliation checkpoint",
            ));
        };
        match load_checkpoint(&runtime_scope.key()) {
            Ok(Some(checkpoint)) => Ok(ReconciliationDto::from(&checkpoint)),
            Ok(None) => Err(ApiError::new(
                ErrorCategory::NotFound,
                "RECONCILIATION_NOT_FOUND",
                "no reconciliation checkpoint exists for this scope yet",
            )),
            Err(error) => Err(map_store_error(error)),
        }
    }

    async fn mutations(
        &self,
        scope: &ExecutionScope,
    ) -> Result<Vec<crate::contract::execution::MutationReceiptDto>, ApiError> {
        let _runtime_scope = self.runtime_scope(scope)?;
        Ok(Vec::new())
    }
}

impl<R> AccountReadAdapter<R> {
    fn bound_runtime(
        &self,
        scope: &ExecutionScope,
    ) -> Result<(AccountBinding, RuntimeScope), ApiError> {
        let binding = self
            .resolver
            .resolve(&scope.account_id, &scope.broker_connection_id)
            .map_err(map_binding_error)?;
        let runtime_scope =
            runtime_scope_from_binding(scope, &binding, self.opaque_credential_ref.clone())?;
        Ok((binding, runtime_scope))
    }
}

fn map_binding_error(error: BindingError) -> ApiError {
    match error {
        BindingError::UnknownAccount => ApiError::new(
            ErrorCategory::NotFound,
            "ACCOUNT_BINDING_NOT_FOUND",
            error.to_string(),
        ),
        BindingError::BoundToOtherConnection { .. } => ApiError::new(
            ErrorCategory::Stale,
            "ACCOUNT_BOUND_TO_OTHER_CONNECTION",
            error.to_string(),
        ),
        BindingError::EmptyAccount
        | BindingError::EmptyConnection
        | BindingError::EmptyBrokerAccount
        | BindingError::DuplicateAccount(_) => ApiError::new(
            ErrorCategory::Validation,
            "INVALID_ACCOUNT_BINDING",
            error.to_string(),
        ),
        BindingError::Unavailable => ApiError::new(
            ErrorCategory::Transient,
            "ACCOUNT_BINDING_UNAVAILABLE",
            error.to_string(),
        ),
    }
}

fn map_broker_error(error: BrokerPortError) -> ApiError {
    let category = match error.class {
        BrokerResultClass::Success => ErrorCategory::Internal,
        BrokerResultClass::RateLimited | BrokerResultClass::Transient => ErrorCategory::Transient,
        BrokerResultClass::Credential | BrokerResultClass::Permission => ErrorCategory::Permission,
        BrokerResultClass::Permanent => ErrorCategory::Internal,
    };
    ApiError::new(category, "BROKER_READ_FAILED", error.to_string())
}

fn map_store_error(error: StoreError) -> ApiError {
    let (category, code) = match error {
        StoreError::OwnershipUnavailable => (ErrorCategory::Conflict, "OWNERSHIP_FAILURE"),
        StoreError::StaleEpoch => (ErrorCategory::Stale, "STALE_EPOCH"),
        StoreError::DuplicateMutation => (ErrorCategory::Conflict, "DUPLICATE_MUTATION"),
        StoreError::InvalidMutationTransition => {
            (ErrorCategory::Conflict, "INVALID_MUTATION_TRANSITION")
        }
        StoreError::Corrupt(_) | StoreError::UnsupportedSchema(_) => {
            (ErrorCategory::Internal, "RUNTIME_STORE_CORRUPT")
        }
        StoreError::Persistence(_) | StoreError::BlockingTask(_) => {
            (ErrorCategory::Transient, "RUNTIME_STORE_FAILURE")
        }
    };
    ApiError::new(category, code, error.to_string())
}

fn disconnected_streams() -> Vec<StreamHealth> {
    [
        StreamKind::OrderState,
        StreamKind::Trades,
        StreamKind::Positions,
        StreamKind::Portfolio,
        StreamKind::Operations,
    ]
    .into_iter()
    .map(|stream| StreamHealth {
        stream,
        required_for_ready: matches!(
            stream,
            StreamKind::OrderState
                | StreamKind::Positions
                | StreamKind::Portfolio
                | StreamKind::Operations
        ),
        state: StreamState::Disconnected,
        queue_depth: 0,
        last_event_at_unix_ms: None,
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::AppState;
    use crate::binding::{AccountBinding, StaticAccountBindingResolver};
    use crate::contract::scope::TradingMode;
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use vox_runtime::{
        BrokerAccount, OperationsPage, OrderFact, PortfolioFact, PositionsFact, Provider,
        RuntimeEnvironment, StopFact,
    };

    struct FakeReads {
        last_scope: Mutex<Option<RuntimeScope>>,
        accounts: Mutex<Vec<BrokerAccount>>,
        portfolio: Mutex<PortfolioFact>,
        positions: Mutex<PositionsFact>,
    }

    impl FakeReads {
        fn remember(&self, scope: &RuntimeScope) {
            *self
                .last_scope
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(scope.clone());
        }
    }

    #[async_trait]
    impl BrokerReadPort for FakeReads {
        async fn accounts(
            &self,
            scope: &RuntimeScope,
        ) -> Result<Vec<BrokerAccount>, BrokerPortError> {
            self.remember(scope);
            Ok(self
                .accounts
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone())
        }

        async fn portfolio(&self, scope: &RuntimeScope) -> Result<PortfolioFact, BrokerPortError> {
            self.remember(scope);
            Ok(self
                .portfolio
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone())
        }

        async fn positions(&self, scope: &RuntimeScope) -> Result<PositionsFact, BrokerPortError> {
            self.remember(scope);
            Ok(self
                .positions
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone())
        }

        async fn active_orders(&self, _: &RuntimeScope) -> Result<Vec<OrderFact>, BrokerPortError> {
            Ok(Vec::new())
        }

        async fn stop_orders(
            &self,
            _: &RuntimeScope,
            _: i64,
        ) -> Result<Vec<StopFact>, BrokerPortError> {
            Ok(Vec::new())
        }

        async fn order_state(
            &self,
            _: &RuntimeScope,
            _: Option<&str>,
            _: Option<&str>,
        ) -> Result<Option<OrderFact>, BrokerPortError> {
            Ok(None)
        }

        async fn operations_page(
            &self,
            _: &RuntimeScope,
            _: Option<&str>,
            _: i64,
            _: u16,
        ) -> Result<OperationsPage, BrokerPortError> {
            Ok(OperationsPage {
                items: Vec::new(),
                next_cursor: None,
            })
        }
    }

    fn credential() -> Result<OpaqueRef, vox_runtime::ModelError> {
        OpaqueRef::new("credential:fixture")
    }

    fn public_scope(
        connection: &str,
        account: &str,
    ) -> Result<ExecutionScope, crate::contract::scope::ScopeError> {
        ExecutionScope::new(
            ProviderDto::TInvest,
            BrokerEnvironment::Sandbox,
            connection,
            account,
            TradingMode::Live,
        )
    }

    fn adapter_with(
        resolver: StaticAccountBindingResolver,
        broker_account_id: &str,
    ) -> Result<AccountReadAdapter<FakeReads>, Box<dyn std::error::Error>> {
        let reads = Arc::new(FakeReads {
            last_scope: Mutex::new(None),
            accounts: Mutex::new(vec![BrokerAccount {
                account_id: broker_account_id.into(),
                open: true,
                accessible: true,
            }]),
            portfolio: Mutex::new(PortfolioFact {
                account_id: broker_account_id.into(),
                total_portfolio_valuation: None,
                total_currency_valuation: None,
                broker_daily_yield: None,
                cash_balances: BTreeMap::new(),
                broker_observed_at_unix_ms: Some(1),
            }),
            positions: Mutex::new(PositionsFact::default()),
        });
        Ok(AccountReadAdapter::new(
            Arc::new(resolver),
            reads,
            credential()?,
        ))
    }

    #[tokio::test]
    async fn process_runtime_serves_accepted_starting_health() -> Result<(), ApiError> {
        let runtime = ProcessRuntime::starting(ProviderDto::TInvest, BrokerEnvironment::Sandbox);
        let health = RuntimeQueries::health(&runtime).await?;
        assert_eq!(
            health.state,
            crate::contract::runtime::RuntimeStateDto::Starting
        );
        assert!(!health.execution_authorized);
        assert!(!health.new_exposure_allowed);
        Ok(())
    }

    #[test]
    fn runtime_health_works_without_account_binding() {
        let state = AppState::detached(ProviderDto::TInvest, BrokerEnvironment::Sandbox)
            .with_runtime(Arc::new(ProcessRuntime::starting(
                ProviderDto::TInvest,
                BrokerEnvironment::Sandbox,
            )));
        assert!(state.runtime_port().is_ok());
        let error = match state.accounts_port() {
            Err(error) => error,
            Ok(_) => panic!("account reads must stay unavailable without a binding"),
        };
        assert_eq!(error.code, "CAPABILITY_UNAVAILABLE");
        assert_eq!(
            error.details.as_ref().and_then(|value| value.get("owner")),
            Some(&serde_json::json!("#17"))
        );
    }

    #[tokio::test]
    async fn adapter_uses_binding_broker_id_not_canonical_account_id()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut resolver = StaticAccountBindingResolver::new();
        resolver.bind(AccountBinding::new(
            "vox-acct-1",
            "connection:primary",
            "broker-99",
        )?)?;
        let adapter = adapter_with(resolver, "broker-99")?;
        let scope = public_scope("connection:primary", "vox-acct-1")?;
        let accounts = adapter.accounts(&scope).await?;
        assert_eq!(accounts[0].account_id, "vox-acct-1");
        assert_eq!(accounts[0].broker_account_id, "broker-99");
        assert_ne!(accounts[0].account_id, accounts[0].broker_account_id);

        let seen = adapter
            .reads
            .last_scope
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let seen = match seen {
            Some(scope) => scope,
            None => panic!("broker port must see a runtime scope"),
        };
        assert_eq!(seen.broker_account_id, "broker-99");
        assert_ne!(seen.broker_account_id, "vox-acct-1");
        assert_eq!(seen.connection_ref.as_str(), "connection:primary");
        Ok(())
    }

    #[tokio::test]
    async fn unknown_canonical_account_does_not_reach_the_broker()
    -> Result<(), Box<dyn std::error::Error>> {
        let adapter = adapter_with(StaticAccountBindingResolver::new(), "broker-99")?;
        let scope = public_scope("connection:primary", "2000000001")?;
        let error = match adapter.accounts(&scope).await {
            Err(error) => error,
            Ok(_) => panic!("unknown canonical id must fail closed"),
        };
        assert_eq!(error.code, "ACCOUNT_BINDING_NOT_FOUND");
        assert!(
            adapter
                .reads
                .last_scope
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .is_none()
        );
        Ok(())
    }

    #[tokio::test]
    async fn account_bound_to_another_connection_fails_closed()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut resolver = StaticAccountBindingResolver::new();
        resolver.bind(AccountBinding::new(
            "vox-acct-1",
            "connection:one",
            "broker-1",
        )?)?;
        let adapter = adapter_with(resolver, "broker-1")?;
        let scope = public_scope("connection:two", "vox-acct-1")?;
        let error = match adapter.accounts(&scope).await {
            Err(error) => error,
            Ok(_) => panic!("wrong connection must fail closed"),
        };
        assert_eq!(error.code, "ACCOUNT_BOUND_TO_OTHER_CONNECTION");
        Ok(())
    }

    #[test]
    fn runtime_scope_from_binding_never_copies_canonical_id_into_broker_id()
    -> Result<(), Box<dyn std::error::Error>> {
        let binding = AccountBinding::new("vox-acct-1", "connection:primary", "broker-99")?;
        let scope = public_scope("connection:primary", "vox-acct-1")?;
        let runtime = runtime_scope_from_binding(&scope, &binding, credential()?)?;
        assert_eq!(runtime.broker_account_id, "broker-99");
        assert_ne!(runtime.broker_account_id, scope.account_id);
        assert_eq!(runtime.connection_ref.as_str(), scope.broker_connection_id);
        assert_eq!(runtime.provider, Provider::TInvest);
        assert_eq!(runtime.environment, RuntimeEnvironment::Sandbox);
        Ok(())
    }
}
