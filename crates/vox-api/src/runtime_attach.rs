//! Attach accepted #11 runtime ports to the public application boundary.
//!
//! Mapping is exhaustive against `vox-runtime` types. Nothing here invents account or
//! execution data: a process without a broker read port still reports account reads as
//! unavailable, and this adapter only answers when a real `BrokerReadPort` is composed.

use std::sync::Arc;

use async_trait::async_trait;
use vox_runtime::{
    BrokerPortError, BrokerReadPort, BrokerResultClass, HealthReadPort, OpaqueRef, ReasonCode,
    ReconciliationCheckpoint, RuntimeHealth, RuntimeScope, RuntimeState, RuntimeStore, StoreError,
    StreamHealth, StreamKind, StreamState,
};

use crate::application::{AccountQueries, RuntimeQueries};
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
/// snapshot is `STARTING`, and no account data is invented.
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
}

/// Account read-side adapter over accepted #11 `BrokerReadPort` and optional runtime store.
pub struct AccountReadAdapter<R> {
    bound: RuntimeScope,
    reads: Arc<R>,
    load_checkpoint: Option<CheckpointLoader>,
}

impl<R> AccountReadAdapter<R> {
    #[must_use]
    pub fn new(bound: RuntimeScope, reads: Arc<R>) -> Self {
        Self {
            bound,
            reads,
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

    fn runtime_scope(&self, requested: &ExecutionScope) -> Result<&RuntimeScope, ApiError> {
        if requested.provider != ProviderDto::from(self.bound.provider)
            || requested.environment != BrokerEnvironment::from(self.bound.environment)
            || requested.account_id != self.bound.broker_account_id
            || requested.broker_connection_id != self.bound.connection_ref.as_str()
        {
            return Err(ApiError::new(
                ErrorCategory::Stale,
                "OWNERSHIP_FAILURE",
                "the requested scope does not match the attached runtime",
            ));
        }
        Ok(&self.bound)
    }
}

#[async_trait]
impl<R> AccountQueries for AccountReadAdapter<R>
where
    R: BrokerReadPort + 'static,
{
    async fn accounts(&self, scope: &ExecutionScope) -> Result<Vec<BrokerAccountDto>, ApiError> {
        let runtime_scope = self.runtime_scope(scope)?;
        let accounts = self
            .reads
            .accounts(runtime_scope)
            .await
            .map_err(map_broker_error)?;
        Ok(accounts.iter().map(BrokerAccountDto::from).collect())
    }

    async fn portfolio(&self, scope: &ExecutionScope) -> Result<PortfolioDto, ApiError> {
        let runtime_scope = self.runtime_scope(scope)?;
        let fact = self
            .reads
            .portfolio(runtime_scope)
            .await
            .map_err(map_broker_error)?;
        PortfolioDto::from_fact(&fact)
    }

    async fn positions(&self, scope: &ExecutionScope) -> Result<Vec<PositionDto>, ApiError> {
        let runtime_scope = self.runtime_scope(scope)?;
        let facts = self
            .reads
            .positions(runtime_scope)
            .await
            .map_err(map_broker_error)?;
        Ok(facts.iter().map(PositionDto::from).collect())
    }

    async fn orders(&self, scope: &ExecutionScope) -> Result<Vec<OrderDto>, ApiError> {
        let runtime_scope = self.runtime_scope(scope)?;
        let facts = self
            .reads
            .active_orders(runtime_scope)
            .await
            .map_err(map_broker_error)?;
        Ok(facts.iter().map(OrderDto::from).collect())
    }

    async fn stop_orders(&self, scope: &ExecutionScope) -> Result<Vec<StopOrderDto>, ApiError> {
        let runtime_scope = self.runtime_scope(scope)?;
        let facts = self
            .reads
            .stop_orders(runtime_scope, 0)
            .await
            .map_err(map_broker_error)?;
        Ok(facts.iter().map(StopOrderDto::from).collect())
    }

    async fn operations(
        &self,
        scope: &ExecutionScope,
        cursor: Option<&str>,
        limit: u16,
    ) -> Result<OperationsPageDto, ApiError> {
        let runtime_scope = self.runtime_scope(scope)?;
        let page = self
            .reads
            .operations_page(runtime_scope, cursor, 0, limit)
            .await
            .map_err(map_broker_error)?;
        Ok(OperationsPageDto::from(&page))
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
}

/// Builds a `RuntimeScope` from a public execution scope. Credential material stays off
/// this boundary: the caller supplies an already-validated opaque credential reference.
pub fn runtime_scope_from_public(
    scope: &ExecutionScope,
    credential_ref: OpaqueRef,
) -> Result<RuntimeScope, ApiError> {
    let connection_ref = OpaqueRef::new(scope.broker_connection_id.clone()).map_err(|_| {
        ApiError::validation(
            "broker_connection_id is not a valid opaque connection identity",
            vec![crate::error::FieldError {
                field: "broker_connection_id".to_owned(),
                message: "must be a non-secret opaque connection identity".to_owned(),
            }],
        )
    })?;
    RuntimeScope::new(
        scope.provider.into(),
        scope.environment.into(),
        scope.account_id.clone(),
        connection_ref,
        credential_ref,
    )
    .map_err(|error| {
        ApiError::new(
            ErrorCategory::Validation,
            "INVALID_SCOPE",
            error.to_string(),
        )
    })
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
    use crate::contract::scope::TradingMode;
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use vox_runtime::{
        BrokerAccount, OperationsPage, OrderFact, PortfolioFact, PositionFact, Provider,
        RuntimeEnvironment, StopFact,
    };

    struct FakeReads {
        accounts: Mutex<Vec<BrokerAccount>>,
        portfolio: Mutex<PortfolioFact>,
        positions: Mutex<Vec<PositionFact>>,
    }

    #[async_trait]
    impl BrokerReadPort for FakeReads {
        async fn accounts(&self, _: &RuntimeScope) -> Result<Vec<BrokerAccount>, BrokerPortError> {
            Ok(self
                .accounts
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone())
        }

        async fn portfolio(&self, _: &RuntimeScope) -> Result<PortfolioFact, BrokerPortError> {
            Ok(self
                .portfolio
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone())
        }

        async fn positions(&self, _: &RuntimeScope) -> Result<Vec<PositionFact>, BrokerPortError> {
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

    fn bound_scope() -> Result<RuntimeScope, vox_runtime::ModelError> {
        RuntimeScope::new(
            Provider::TInvest,
            RuntimeEnvironment::Sandbox,
            "account:primary",
            OpaqueRef::new("connection:primary")?,
            OpaqueRef::new("credential:primary")?,
        )
    }

    fn public_scope() -> Result<ExecutionScope, crate::contract::scope::ScopeError> {
        ExecutionScope::new(
            ProviderDto::TInvest,
            BrokerEnvironment::Sandbox,
            "connection:primary",
            "account:primary",
            TradingMode::Live,
        )
    }

    #[tokio::test]
    async fn process_runtime_serves_accepted_starting_health() -> Result<(), ApiError> {
        let runtime = ProcessRuntime::starting(ProviderDto::TInvest, BrokerEnvironment::Sandbox);
        let health = RuntimeQueries::health(&runtime).await?;
        assert_eq!(
            health.state,
            crate::contract::runtime::RuntimeStateDto::Starting
        );
        assert_eq!(
            health.reason_code,
            crate::contract::runtime::ReasonCodeDto::Startup
        );
        assert!(!health.execution_authorized);
        assert!(!health.new_exposure_allowed);
        assert_eq!(health.stream_states.len(), 5);
        Ok(())
    }

    #[tokio::test]
    async fn account_adapter_maps_broker_facts_and_rejects_a_foreign_scope()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut currencies = BTreeMap::new();
        currencies.insert("rub".to_owned(), "100.500000000".to_owned());
        let reads = Arc::new(FakeReads {
            accounts: Mutex::new(vec![BrokerAccount {
                account_id: "account:primary".into(),
                open: true,
                accessible: true,
            }]),
            portfolio: Mutex::new(PortfolioFact {
                account_id: "account:primary".into(),
                currencies,
                broker_observed_at_unix_ms: Some(1),
            }),
            positions: Mutex::new(vec![PositionFact {
                account_id: "account:primary".into(),
                instrument_uid: "uid-1".into(),
                quantity_units: 3,
                broker_observed_at_unix_ms: Some(1),
            }]),
        });
        let adapter = AccountReadAdapter::new(bound_scope()?, reads);
        let scope = public_scope()?;
        let accounts = adapter.accounts(&scope).await?;
        assert_eq!(accounts[0].account_id, "account:primary");
        assert_eq!(accounts[0].broker_account_id, "account:primary");
        let portfolio = adapter.portfolio(&scope).await?;
        assert_eq!(portfolio.balances[0].amount.as_str(), "100.500000000");
        let positions = adapter.positions(&scope).await?;
        assert_eq!(positions[0].quantity_units, 3);

        let foreign = ExecutionScope::new(
            ProviderDto::TInvest,
            BrokerEnvironment::Sandbox,
            "connection:other",
            "account:primary",
            TradingMode::Live,
        )?;
        let error = match adapter.accounts(&foreign).await {
            Err(error) => error,
            Ok(_) => panic!("foreign connection must not retarget"),
        };
        assert_eq!(error.code, "OWNERSHIP_FAILURE");
        Ok(())
    }
}
