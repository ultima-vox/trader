//! Application ports.
//!
//! The transport depends on these traits, never on a broker client. A deployment attaches
//! whatever it actually has; anything not attached answers `CAPABILITY_UNAVAILABLE` with the
//! owning issue, which is the only honest answer when a contract does not exist yet.

use std::sync::Arc;

use async_trait::async_trait;

use crate::contract::account::{
    BrokerAccountDto, OperationsPageDto, OrderDto, PortfolioDto, PositionDto, ReconciliationDto,
    StopOrderDto,
};
use crate::contract::auth::CreateSessionRequest;
use crate::contract::capability::{AttachedBackends, CapabilitySet};
use crate::contract::connections::{
    BindBrokerAccountRequest, BrokerAccountBindingDto, BrokerConnectionMetadataDto,
    ChangeExecutionAuthorizationRequest, ConnectionDetailsDto, CreateBrokerConnectionRequest,
    CredentialRotationResultDto, DiscoveredBrokerAccountDto, ExecutionAuthorizationDto,
    RotateCredentialRequest,
};
use crate::contract::execution::{CancelOrderRequest, MutationReceiptDto, SubmitOrderRequest};
use crate::contract::market::{
    CandleIntervalDto, CandlesDto, InstrumentSummaryDto, OrderBookDto, QuoteDto, SessionDto,
    TradeTickDto,
};
use crate::contract::risk::{RiskReservationDto, RiskStatusDto};
use crate::contract::runtime::RuntimeHealthDto;
use crate::contract::scope::{BrokerEnvironment, ExecutionScope, ProviderDto};
use crate::error::ApiError;
use crate::events::{ApplicationEventBus, spawn_runtime_health_watch};

/// Runtime health and readiness, owned by #11.
#[async_trait]
pub trait RuntimeQueries: Send + Sync {
    async fn health(&self) -> Result<RuntimeHealthDto, ApiError>;
    /// Health for one explicit capital scope. Implementations must not substitute another scope.
    async fn scoped_health(&self, _scope: &ExecutionScope) -> Result<RuntimeHealthDto, ApiError> {
        Err(ApiError::capability_unavailable(
            "SCOPED_RUNTIME_HEALTH",
            "#18",
        ))
    }
    /// Scopes the operator may select. Empty until #17 bindings exist.
    async fn scopes(&self) -> Result<Vec<ExecutionScope>, ApiError>;

    async fn capabilities(
        &self,
        _account_id: Option<&str>,
    ) -> Result<Option<CapabilitySet>, ApiError> {
        Ok(None)
    }
}

/// The per-account read side, owned by #9 and #11.
#[async_trait]
pub trait AccountQueries: Send + Sync {
    async fn accounts(&self, scope: &ExecutionScope) -> Result<Vec<BrokerAccountDto>, ApiError>;
    async fn portfolio(&self, scope: &ExecutionScope) -> Result<PortfolioDto, ApiError>;
    async fn positions(&self, scope: &ExecutionScope) -> Result<Vec<PositionDto>, ApiError>;
    async fn orders(&self, scope: &ExecutionScope) -> Result<Vec<OrderDto>, ApiError>;
    async fn stop_orders(&self, scope: &ExecutionScope) -> Result<Vec<StopOrderDto>, ApiError>;
    async fn operations(
        &self,
        scope: &ExecutionScope,
        cursor: Option<&str>,
        limit: u16,
    ) -> Result<OperationsPageDto, ApiError>;
    async fn reconciliation(&self, scope: &ExecutionScope) -> Result<ReconciliationDto, ApiError>;
    async fn mutations(&self, scope: &ExecutionScope) -> Result<Vec<MutationReceiptDto>, ApiError>;
}

/// Capital-affecting commands, owned by #10 and gated by #17 authorization and #11 readiness.
#[async_trait]
pub trait ExecutionCommands: Send + Sync {
    async fn submit_order(
        &self,
        request: SubmitOrderRequest,
    ) -> Result<MutationReceiptDto, ApiError>;
    async fn cancel_order(
        &self,
        request: CancelOrderRequest,
    ) -> Result<MutationReceiptDto, ApiError>;
    async fn receipt(
        &self,
        scope: &ExecutionScope,
        logical_request_id: &str,
    ) -> Result<MutationReceiptDto, ApiError>;
    async fn replace_order(
        &self,
        request: crate::contract::execution::ReplaceOrderRequest,
    ) -> Result<MutationReceiptDto, ApiError>;
    async fn submit_stop_order(
        &self,
        request: crate::contract::execution::SubmitStopOrderRequest,
    ) -> Result<MutationReceiptDto, ApiError>;
    async fn cancel_stop_order(
        &self,
        request: crate::contract::execution::CancelOrderRequest,
    ) -> Result<MutationReceiptDto, ApiError>;
    async fn submit_protection(
        &self,
        request: crate::contract::execution::SubmitProtectionRequest,
    ) -> Result<MutationReceiptDto, ApiError>;
}

/// The Vox-side market-data projection over the accepted #8 adapter layer.
///
/// This is a projection, not a second broker client: an implementation reads what #8 already
/// acquired and republishes it provider-neutrally. Nothing here talks to a provider.
#[async_trait]
pub trait MarketDataQueries: Send + Sync {
    /// Catalogue entries for the instruments the operator may pick, searched by ticker or name.
    async fn search_instruments(
        &self,
        provider: ProviderDto,
        query: &str,
        limit: u16,
    ) -> Result<Vec<InstrumentSummaryDto>, ApiError>;
    async fn quote(
        &self,
        provider: ProviderDto,
        instrument_uid: &str,
    ) -> Result<QuoteDto, ApiError>;
    async fn order_book(
        &self,
        provider: ProviderDto,
        instrument_uid: &str,
        depth: u16,
    ) -> Result<OrderBookDto, ApiError>;
    async fn trades(
        &self,
        provider: ProviderDto,
        instrument_uid: &str,
        limit: u16,
    ) -> Result<Vec<TradeTickDto>, ApiError>;
    async fn candles(
        &self,
        provider: ProviderDto,
        instrument_uid: &str,
        interval: CandleIntervalDto,
        from_unix_ms: i64,
        to_unix_ms: i64,
    ) -> Result<CandlesDto, ApiError>;
    async fn session(
        &self,
        provider: ProviderDto,
        instrument_uid: &str,
    ) -> Result<SessionDto, ApiError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticatedActor {
    pub user_id: String,
}

/// Session material passed only from application auth port to HTTP transport.
pub struct EstablishedSession {
    pub user_id: String,
    pub effective_permissions: Vec<crate::contract::auth::PermissionDto>,
    pub session_token: String,
    pub csrf_token: String,
    pub expires_at_unix_ms: i64,
    pub cookie_secure: bool,
}

impl core::fmt::Debug for EstablishedSession {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("EstablishedSession")
            .field("user_id", &self.user_id)
            .field("effective_permissions", &self.effective_permissions)
            .field("session_token", &"[REDACTED]")
            .field("csrf_token", &"[REDACTED]")
            .field("expires_at_unix_ms", &self.expires_at_unix_ms)
            .field("cookie_secure", &self.cookie_secure)
            .finish()
    }
}

#[async_trait]
pub trait SessionAuthentication: Send + Sync {
    async fn establish_session(
        &self,
        request: CreateSessionRequest,
    ) -> Result<EstablishedSession, ApiError>;
}

/// Risk state and policy queries, owned by #21.
///
/// The transport reads risk state through this port; the risk engine itself lives behind
/// the application boundary. Handlers never derive risk verdicts locally.
#[async_trait]
pub trait RiskQueries: Send + Sync {
    /// Current risk status for one execution scope.
    async fn risk_status(&self, scope: &ExecutionScope) -> Result<RiskStatusDto, ApiError>;
    /// Active reservations for one execution scope.
    async fn active_reservations(
        &self,
        scope: &ExecutionScope,
    ) -> Result<Vec<RiskReservationDto>, ApiError>;
}

/// Risk state mutations, owned by #21.
///
/// Operators change risk state (for example, halting trading during anomalous conditions)
/// through this port. The risk engine validates and persists the transition; the transport
/// merely relays the request.
#[async_trait]
pub trait RiskCommands: Send + Sync {
    /// Change the risk state for one execution scope.
    async fn change_state(
        &self,
        request: crate::contract::risk::ChangeRiskStateRequest,
    ) -> Result<RiskStatusDto, ApiError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionRequestContext {
    pub actor: AuthenticatedActor,
    pub correlation_id: String,
    pub now_unix_ms: i64,
}

#[async_trait]
pub trait ConnectionAdministration: Send + Sync {
    async fn list_connections(
        &self,
        context: &ConnectionRequestContext,
    ) -> Result<Vec<BrokerConnectionMetadataDto>, ApiError>;
    async fn create_connection(
        &self,
        context: &ConnectionRequestContext,
        request: CreateBrokerConnectionRequest,
    ) -> Result<BrokerConnectionMetadataDto, ApiError>;
    async fn connection_details(
        &self,
        context: &ConnectionRequestContext,
        connection_id: &str,
    ) -> Result<ConnectionDetailsDto, ApiError>;
    async fn revalidate_connection(
        &self,
        context: &ConnectionRequestContext,
        connection_id: &str,
    ) -> Result<BrokerConnectionMetadataDto, ApiError>;
    async fn rotate_credential(
        &self,
        context: &ConnectionRequestContext,
        connection_id: &str,
        request: RotateCredentialRequest,
    ) -> Result<CredentialRotationResultDto, ApiError>;
    async fn disable_connection(
        &self,
        context: &ConnectionRequestContext,
        connection_id: &str,
    ) -> Result<BrokerConnectionMetadataDto, ApiError>;
    async fn delete_connection(
        &self,
        context: &ConnectionRequestContext,
        connection_id: &str,
    ) -> Result<(), ApiError>;
    async fn accounts(
        &self,
        context: &ConnectionRequestContext,
        connection_id: &str,
    ) -> Result<Vec<DiscoveredBrokerAccountDto>, ApiError>;
    async fn bindings(
        &self,
        context: &ConnectionRequestContext,
        connection_id: &str,
    ) -> Result<Vec<BrokerAccountBindingDto>, ApiError>;
    async fn bind_account(
        &self,
        context: &ConnectionRequestContext,
        connection_id: &str,
        request: BindBrokerAccountRequest,
    ) -> Result<BrokerAccountBindingDto, ApiError>;
    async fn unbind_account(
        &self,
        context: &ConnectionRequestContext,
        binding_id: &str,
    ) -> Result<(), ApiError>;
    async fn change_execution_authorization(
        &self,
        context: &ConnectionRequestContext,
        connection_id: &str,
        request: ChangeExecutionAuthorizationRequest,
    ) -> Result<ExecutionAuthorizationDto, ApiError>;
}

/// Production process hook for invalidating runtime clients after credential lifecycle changes.
#[async_trait]
pub trait ConnectionLifecycleObserver: Send + Sync {
    async fn connection_changed(&self, connection_id: &str);
}

/// What the process serves. A `None` port is a capability this deployment does not have.
#[derive(Clone)]
pub struct AppState {
    pub provider: ProviderDto,
    pub environment: BrokerEnvironment,
    pub runtime: Option<Arc<dyn RuntimeQueries>>,
    pub accounts: Option<Arc<dyn AccountQueries>>,
    pub execution: Option<Arc<dyn ExecutionCommands>>,
    pub market_data: Option<Arc<dyn MarketDataQueries>>,
    pub connections: Option<Arc<dyn ConnectionAdministration>>,
    pub authentication: Option<Arc<dyn SessionAuthentication>>,
    pub risk_queries: Option<Arc<dyn RiskQueries>>,
    pub risk_commands: Option<Arc<dyn RiskCommands>>,
    /// Application-side live bus. Not a broker stream.
    pub events: ApplicationEventBus,
}

impl AppState {
    /// Production composition with mandatory #17/#11/#10 ports attached atomically.
    #[must_use]
    pub fn production(
        provider: ProviderDto,
        environment: BrokerEnvironment,
        runtime: Arc<dyn RuntimeQueries>,
        accounts: Arc<dyn AccountQueries>,
        execution: Arc<dyn ExecutionCommands>,
        connections: Arc<dyn ConnectionAdministration>,
        authentication: Arc<dyn SessionAuthentication>,
    ) -> Self {
        Self {
            provider,
            environment,
            runtime: Some(runtime),
            accounts: Some(accounts),
            execution: Some(execution),
            market_data: None,
            connections: Some(connections),
            authentication: Some(authentication),
            risk_queries: None,
            risk_commands: None,
            events: ApplicationEventBus::new(),
        }
    }

    /// A process with no broker runtime attached: it can describe itself and nothing else.
    #[must_use]
    pub fn detached(provider: ProviderDto, environment: BrokerEnvironment) -> Self {
        Self {
            provider,
            environment,
            runtime: None,
            accounts: None,
            execution: None,
            market_data: None,
            connections: None,
            authentication: None,
            risk_queries: None,
            risk_commands: None,
            events: ApplicationEventBus::new(),
        }
    }

    #[must_use]
    pub fn with_events(mut self, events: ApplicationEventBus) -> Self {
        self.events = events;
        self
    }

    /// Starts the runtime-health watcher once for this process. Safe to call without a runtime.
    pub fn spawn_runtime_watch(&self) {
        let Some(runtime) = self.runtime.clone() else {
            return;
        };
        spawn_runtime_health_watch(runtime, self.events.clone());
    }

    #[must_use]
    pub fn with_runtime(mut self, runtime: Arc<dyn RuntimeQueries>) -> Self {
        self.runtime = Some(runtime);
        self
    }

    #[must_use]
    pub fn with_accounts(mut self, accounts: Arc<dyn AccountQueries>) -> Self {
        self.accounts = Some(accounts);
        self
    }

    #[must_use]
    pub fn with_execution(mut self, execution: Arc<dyn ExecutionCommands>) -> Self {
        self.execution = Some(execution);
        self
    }

    /// What this deployment can actually do.
    #[must_use]
    pub fn with_market_data(mut self, market_data: Arc<dyn MarketDataQueries>) -> Self {
        self.market_data = Some(market_data);
        self
    }

    #[must_use]
    pub fn with_connections(mut self, connections: Arc<dyn ConnectionAdministration>) -> Self {
        self.connections = Some(connections);
        self
    }

    #[must_use]
    pub fn with_risk_queries(mut self, risk_queries: Arc<dyn RiskQueries>) -> Self {
        self.risk_queries = Some(risk_queries);
        self
    }

    #[must_use]
    pub fn with_risk_commands(mut self, risk_commands: Arc<dyn RiskCommands>) -> Self {
        self.risk_commands = Some(risk_commands);
        self
    }

    #[must_use]
    pub fn capabilities(&self, account_id: Option<String>) -> CapabilitySet {
        CapabilitySet::without_backend_owners(
            self.provider,
            self.environment,
            account_id,
            AttachedBackends {
                runtime: self.runtime.is_some(),
                accounts: self.accounts.is_some(),
                execution: self.execution.is_some(),
                market_data: self.market_data.is_some(),
                connections: self.connections.is_some(),
                risk: self.risk_queries.is_some(),
            },
        )
    }

    pub(crate) fn market_data_port(&self) -> Result<&Arc<dyn MarketDataQueries>, ApiError> {
        self.market_data.as_ref().ok_or_else(|| {
            ApiError::capability_unavailable("MARKET_DATA", "#38 projection over #8")
        })
    }

    pub(crate) fn runtime_port(&self) -> Result<&Arc<dyn RuntimeQueries>, ApiError> {
        self.runtime
            .as_ref()
            .ok_or_else(|| ApiError::capability_unavailable("RUNTIME_HEALTH", "#11"))
    }

    pub(crate) fn accounts_port(&self) -> Result<&Arc<dyn AccountQueries>, ApiError> {
        self.accounts
            .as_ref()
            .ok_or_else(|| ApiError::capability_unavailable("ACCOUNT_READ_SIDE", "#17"))
    }

    pub(crate) fn execution_port(&self) -> Result<&Arc<dyn ExecutionCommands>, ApiError> {
        self.execution
            .as_ref()
            .ok_or_else(|| ApiError::capability_unavailable("ORDER_EXECUTION", "#10"))
    }

    pub(crate) fn connections_port(&self) -> Result<&Arc<dyn ConnectionAdministration>, ApiError> {
        self.connections
            .as_ref()
            .ok_or_else(|| ApiError::capability_unavailable("BROKER_CONNECTIONS", "#17"))
    }

    pub(crate) fn authentication_port(&self) -> Result<&Arc<dyn SessionAuthentication>, ApiError> {
        self.authentication.as_ref().ok_or_else(|| {
            ApiError::capability_unavailable("SESSION_AUTHENTICATION", "#47 follow-up")
        })
    }

    pub(crate) fn risk_queries_port(
        &self,
    ) -> Result<&Arc<dyn RiskQueries>, ApiError> {
        self.risk_queries
            .as_ref()
            .ok_or_else(|| ApiError::capability_unavailable("RISK_QUERIES", "#21"))
    }

    pub(crate) fn risk_commands_port(
        &self,
    ) -> Result<&Arc<dyn RiskCommands>, ApiError> {
        self.risk_commands
            .as_ref()
            .ok_or_else(|| ApiError::capability_unavailable("RISK_COMMANDS", "#21"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCategory;

    #[test]
    fn a_detached_process_refuses_instead_of_pretending() {
        let state = AppState::detached(ProviderDto::TInvest, BrokerEnvironment::Sandbox);
        let error = state
            .accounts_port()
            .err()
            .expect("a detached process has no account read side");
        assert_eq!(error.category, ErrorCategory::CapabilityUnavailable);
        assert_eq!(error.code, "CAPABILITY_UNAVAILABLE");
        assert_eq!(
            error.details.as_ref().and_then(|value| value.get("owner")),
            Some(&serde_json::json!("#17"))
        );
        assert!(
            !error.retryable,
            "a missing contract is not a transient failure"
        );
    }

    #[test]
    fn a_market_read_without_a_projection_names_its_owner() {
        let state = AppState::detached(ProviderDto::TInvest, BrokerEnvironment::Sandbox);
        let error = state
            .market_data_port()
            .err()
            .expect("a detached process has no market-data projection");
        assert_eq!(error.category, ErrorCategory::CapabilityUnavailable);
        assert!(
            error
                .details
                .as_ref()
                .and_then(|d| d.get("owner"))
                .is_some(),
            "an unavailable capability must name who owns it"
        );
    }
}
