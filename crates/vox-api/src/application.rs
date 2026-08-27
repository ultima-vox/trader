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
use crate::contract::capability::CapabilitySet;
use crate::contract::execution::{CancelOrderRequest, MutationReceiptDto, SubmitOrderRequest};
use crate::contract::runtime::RuntimeHealthDto;
use crate::contract::scope::{BrokerEnvironment, ExecutionScope, ProviderDto};
use crate::error::ApiError;

/// Runtime health and readiness, owned by #11.
#[async_trait]
pub trait RuntimeQueries: Send + Sync {
    async fn health(&self) -> Result<RuntimeHealthDto, ApiError>;
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
}

/// Capital-affecting commands, owned by #10 and gated by #17 authorization and #11 readiness.
#[async_trait]
pub trait ExecutionCommands: Send + Sync {
    async fn submit_order(&self, request: SubmitOrderRequest) -> Result<MutationReceiptDto, ApiError>;
    async fn cancel_order(&self, request: CancelOrderRequest) -> Result<MutationReceiptDto, ApiError>;
    async fn receipt(
        &self,
        scope: &ExecutionScope,
        logical_request_id: &str,
    ) -> Result<MutationReceiptDto, ApiError>;
}

/// What the process serves. A `None` port is a capability this deployment does not have.
#[derive(Clone)]
pub struct AppState {
    pub provider: ProviderDto,
    pub environment: BrokerEnvironment,
    pub runtime: Option<Arc<dyn RuntimeQueries>>,
    pub accounts: Option<Arc<dyn AccountQueries>>,
    pub execution: Option<Arc<dyn ExecutionCommands>>,
}

impl AppState {
    /// A process with no broker runtime attached: it can describe itself and nothing else.
    #[must_use]
    pub fn detached(provider: ProviderDto, environment: BrokerEnvironment) -> Self {
        Self { provider, environment, runtime: None, accounts: None, execution: None }
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
    pub fn capabilities(&self, account_id: Option<String>) -> CapabilitySet {
        CapabilitySet::without_backend_owners(
            self.provider,
            self.environment,
            account_id,
            self.accounts.is_some(),
        )
    }

    pub(crate) fn runtime_port(&self) -> Result<&Arc<dyn RuntimeQueries>, ApiError> {
        self.runtime
            .as_ref()
            .ok_or_else(|| ApiError::capability_unavailable("RUNTIME_HEALTH", "#11"))
    }

    pub(crate) fn accounts_port(&self) -> Result<&Arc<dyn AccountQueries>, ApiError> {
        self.accounts
            .as_ref()
            .ok_or_else(|| ApiError::capability_unavailable("ACCOUNT_READ_SIDE", "#11"))
    }

    pub(crate) fn execution_port(&self) -> Result<&Arc<dyn ExecutionCommands>, ApiError> {
        self.execution
            .as_ref()
            .ok_or_else(|| ApiError::capability_unavailable("ORDER_EXECUTION", "#10"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCategory;

    #[test]
    fn a_detached_process_refuses_instead_of_pretending() {
        let state = AppState::detached(ProviderDto::TInvest, BrokerEnvironment::Sandbox);
        let error = state.accounts_port().err().expect("a detached process has no account read side");
        assert_eq!(error.category, ErrorCategory::CapabilityUnavailable);
        assert_eq!(error.code, "CAPABILITY_UNAVAILABLE");
        assert!(!error.retryable, "a missing contract is not a transient failure");
    }
}
