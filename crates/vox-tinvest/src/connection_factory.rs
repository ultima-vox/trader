//! Builds existing T-Invest gRPC client only after #17 resolves exact stored connection scope.

use async_trait::async_trait;
use std::collections::BTreeSet;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::{Mutex, mpsc};
use vox_connections::{
    AccountTarget, BrokerCredentialClientFactory, BrokerEnvironment, BrokerProviderPort,
    ConnectionId, ConnectionRepository, ConnectionService, ExecutionPurpose, ProviderError,
    ProviderErrorKind, ProviderId, SecretBytes, SecretStore, ServiceError,
};
use vox_domain::RuntimeExecutionCommand;
use vox_runtime::{
    BrokerAccount, BrokerIdentityLinks, BrokerPortError, BrokerReadPort, BrokerResultClass,
    ExecutionPort, ExecutionResult, ExecutionStreamPort, MutationRecord, OperationsPage, OrderFact,
    PortfolioFact, PositionsFact, RuntimeExecutionPurpose, RuntimeScope, StopFact, StreamKind,
    StreamSignal,
};

use crate::account::AccountReadClient;
use crate::execution_dispatch::ExecutionRoute;
use crate::runtime_execution::{
    RuntimeDispatchAcknowledgement, RuntimeExecutionAdapterError, TInvestRuntimeExecutionAdapter,
    authoritative_rejection,
};
use crate::{GrpcConfigError, GrpcCredential, SecretToken, SecretTokenError, TInvestGrpcClient};

pub struct TInvestReadSession {
    pub target: AccountTarget,
    pub client: AccountReadClient,
    pub runtime_reads: crate::runtime_read::TInvestRuntimeReadAdapter,
    pub risk_reads: crate::risk_read::TInvestRiskReadAdapter,
    pub runtime_streams: crate::runtime_stream::TInvestRuntimeStreamAdapter,
}

struct AuthorizedTInvestExecutionClient {
    target: AccountTarget,
    adapter: TInvestRuntimeExecutionAdapter,
    authorization_revision: u64,
}

pub struct TInvestExecutionSession<R, S, P> {
    target: AccountTarget,
    adapter: TInvestRuntimeExecutionAdapter,
    authorization_revision: u64,
    purpose: ExecutionPurpose,
    connections: Arc<ConnectionService<R, S, P>>,
}

impl<R, S, P> TInvestExecutionSession<R, S, P>
where
    R: ConnectionRepository,
    S: SecretStore,
    P: BrokerProviderPort,
{
    #[must_use]
    pub const fn target(&self) -> &AccountTarget {
        &self.target
    }

    #[must_use]
    pub const fn authorization_revision(&self) -> u64 {
        self.authorization_revision
    }

    pub async fn dispatch_once(
        &self,
        command: &RuntimeExecutionCommand,
    ) -> Result<RuntimeDispatchAcknowledgement, ConnectionFactoryError> {
        self.connections.validate_runtime_execution_access(
            &self.target.connection_id,
            &self.target.provider_account_id,
            self.purpose,
            Some(self.authorization_revision),
        )?;
        self.adapter
            .dispatch_once(command)
            .await
            .map_err(Into::into)
    }
}

pub struct StoredTInvestClientFactory<R, S, P> {
    connections: Arc<ConnectionService<R, S, P>>,
}

pub struct StoredTInvestReadPort<R, S, P> {
    factory: Arc<StoredTInvestClientFactory<R, S, P>>,
}

impl<R, S, P> StoredTInvestReadPort<R, S, P> {
    #[must_use]
    pub const fn new(factory: Arc<StoredTInvestClientFactory<R, S, P>>) -> Self {
        Self { factory }
    }
}

pub struct StoredTInvestExecutionPort<R, S, P> {
    factory: Arc<StoredTInvestClientFactory<R, S, P>>,
}

pub struct StoredTInvestStreamPort<R, S, P> {
    factory: Arc<StoredTInvestClientFactory<R, S, P>>,
    active: Mutex<Option<crate::runtime_stream::TInvestRuntimeStreamAdapter>>,
}

impl<R, S, P> StoredTInvestStreamPort<R, S, P> {
    #[must_use]
    pub const fn new(factory: Arc<StoredTInvestClientFactory<R, S, P>>) -> Self {
        Self {
            factory,
            active: Mutex::const_new(None),
        }
    }
}

impl<R, S, P> StoredTInvestExecutionPort<R, S, P> {
    #[must_use]
    pub const fn new(factory: Arc<StoredTInvestClientFactory<R, S, P>>) -> Self {
        Self { factory }
    }
}

impl<R, S, P> StoredTInvestClientFactory<R, S, P>
where
    R: ConnectionRepository,
    S: SecretStore,
    P: BrokerProviderPort,
{
    #[must_use]
    pub const fn new(connections: Arc<ConnectionService<R, S, P>>) -> Self {
        Self { connections }
    }

    pub fn read_session(
        &self,
        connection_id: &ConnectionId,
        provider_account_id: &str,
    ) -> Result<TInvestReadSession, ConnectionFactoryError> {
        let resolved = self.connections.open_runtime_read_client(
            &TInvestClientBuilder,
            connection_id,
            provider_account_id,
        )?;
        Ok(resolved)
    }

    pub fn execution_session(
        &self,
        connection_id: &ConnectionId,
        provider_account_id: &str,
        purpose: ExecutionPurpose,
    ) -> Result<TInvestExecutionSession<R, S, P>, ConnectionFactoryError> {
        let grant = self.connections.validate_runtime_execution_access(
            connection_id,
            provider_account_id,
            purpose,
            None,
        )?;
        let authorized = self.connections.open_runtime_execution_client(
            &TInvestClientBuilder,
            connection_id,
            provider_account_id,
            purpose,
            grant.authorization_revision,
        )?;
        Ok(TInvestExecutionSession {
            target: authorized.target,
            adapter: authorized.adapter,
            authorization_revision: authorized.authorization_revision,
            purpose,
            connections: Arc::clone(&self.connections),
        })
    }

    #[must_use]
    pub fn into_inner(self) -> Arc<ConnectionService<R, S, P>> {
        self.connections
    }
}

#[async_trait]
impl<R, S, P> BrokerReadPort for StoredTInvestReadPort<R, S, P>
where
    R: ConnectionRepository + 'static,
    S: SecretStore + 'static,
    P: BrokerProviderPort + 'static,
{
    async fn accounts(&self, scope: &RuntimeScope) -> Result<Vec<BrokerAccount>, BrokerPortError> {
        let session = self.session(scope)?;
        session.runtime_reads.accounts(scope).await
    }

    async fn portfolio(&self, scope: &RuntimeScope) -> Result<PortfolioFact, BrokerPortError> {
        let session = self.session(scope)?;
        session.runtime_reads.portfolio(scope).await
    }

    async fn positions(&self, scope: &RuntimeScope) -> Result<PositionsFact, BrokerPortError> {
        let session = self.session(scope)?;
        session.runtime_reads.positions(scope).await
    }

    async fn active_orders(&self, scope: &RuntimeScope) -> Result<Vec<OrderFact>, BrokerPortError> {
        let session = self.session(scope)?;
        session.runtime_reads.active_orders(scope).await
    }

    async fn stop_orders(
        &self,
        scope: &RuntimeScope,
        include_terminal_since_unix_ms: i64,
    ) -> Result<Vec<StopFact>, BrokerPortError> {
        let session = self.session(scope)?;
        session
            .runtime_reads
            .stop_orders(scope, include_terminal_since_unix_ms)
            .await
    }

    async fn order_state(
        &self,
        scope: &RuntimeScope,
        broker_order_id: Option<&str>,
        logical_request_id: Option<&str>,
    ) -> Result<Option<OrderFact>, BrokerPortError> {
        let session = self.session(scope)?;
        session
            .runtime_reads
            .order_state(scope, broker_order_id, logical_request_id)
            .await
    }

    async fn operations_page(
        &self,
        scope: &RuntimeScope,
        cursor: Option<&str>,
        from_unix_ms: i64,
        limit: u16,
    ) -> Result<OperationsPage, BrokerPortError> {
        let session = self.session(scope)?;
        session
            .runtime_reads
            .operations_page(scope, cursor, from_unix_ms, limit)
            .await
    }
}

impl<R, S, P> StoredTInvestReadPort<R, S, P>
where
    R: ConnectionRepository,
    S: SecretStore,
    P: BrokerProviderPort,
{
    fn session(&self, scope: &RuntimeScope) -> Result<TInvestReadSession, BrokerPortError> {
        let connection_id = ConnectionId::parse(scope.connection_ref.as_str().to_owned())
            .map_err(|_| factory_port_error("invalid runtime connection identity"))?;
        self.factory
            .read_session(&connection_id, &scope.broker_account_id)
            .map_err(|_| factory_port_error("stored read session unavailable"))
    }
}

#[async_trait]
impl<R, S, P> ExecutionPort for StoredTInvestExecutionPort<R, S, P>
where
    R: ConnectionRepository + 'static,
    S: SecretStore + 'static,
    P: BrokerProviderPort + 'static,
{
    async fn dispatch_once(
        &self,
        scope: &RuntimeScope,
        purpose: RuntimeExecutionPurpose,
        command: &RuntimeExecutionCommand,
        mutation: &MutationRecord,
    ) -> Result<ExecutionResult, BrokerPortError> {
        let connection_id = ConnectionId::parse(scope.connection_ref.as_str().to_owned())
            .map_err(|_| factory_port_error("invalid runtime connection identity"))?;
        let session = self
            .factory
            .execution_session(
                &connection_id,
                &scope.broker_account_id,
                execution_purpose(purpose),
            )
            .map_err(|_| authorization_port_error())?;
        match session.dispatch_once(command).await {
            Ok(acknowledgement) => Ok(ExecutionResult::Acknowledged {
                broker_evidence_ref: format!(
                    "request_id={}; broker_identity_present={}",
                    acknowledgement.transport_request_id,
                    acknowledgement.broker_order_id.is_some()
                        || acknowledgement.replacement_broker_order_id.is_some()
                        || acknowledgement.broker_stop_order_id.is_some()
                ),
                links: BrokerIdentityLinks {
                    logical_request_id: mutation.logical_request_id.clone(),
                    broker_order_id: acknowledgement.broker_order_id,
                    replacement_broker_order_id: acknowledgement.replacement_broker_order_id,
                    broker_stop_order_id: acknowledgement.broker_stop_order_id,
                    provider_operation_ids: acknowledgement
                        .provider_operation_id
                        .into_iter()
                        .collect(),
                    broker_fill_ids: BTreeSet::new(),
                },
            }),
            Err(ConnectionFactoryError::RuntimeExecution(
                RuntimeExecutionAdapterError::Validation(error),
            )) => Ok(ExecutionResult::Rejected {
                broker_evidence_ref: format!("pre-dispatch:{error}"),
            }),
            Err(ConnectionFactoryError::RuntimeExecution(
                RuntimeExecutionAdapterError::Authorization(error),
            )) => Ok(ExecutionResult::Rejected {
                broker_evidence_ref: format!("pre-dispatch:{error}"),
            }),
            Err(ConnectionFactoryError::RuntimeExecution(
                RuntimeExecutionAdapterError::Transport(error),
            )) if authoritative_rejection(&error) => Ok(ExecutionResult::Rejected {
                broker_evidence_ref: format!("provider-rejection:{}", error.metadata.request_id),
            }),
            Err(_) => Err(factory_port_error(
                "capital-affecting dispatch outcome is not authoritative",
            )),
        }
    }
}

#[async_trait]
impl<R, S, P> ExecutionStreamPort for StoredTInvestStreamPort<R, S, P>
where
    R: ConnectionRepository + 'static,
    S: SecretStore + 'static,
    P: BrokerProviderPort + 'static,
{
    async fn connect(
        &self,
        scope: &RuntimeScope,
        runtime_epoch: u64,
        output: mpsc::Sender<StreamSignal>,
    ) -> Result<BTreeSet<StreamKind>, BrokerPortError> {
        self.disconnect().await?;
        let connection_id = ConnectionId::parse(scope.connection_ref.as_str().to_owned())
            .map_err(|_| factory_port_error("invalid runtime connection identity"))?;
        let session = self
            .factory
            .read_session(&connection_id, &scope.broker_account_id)
            .map_err(|_| factory_port_error("stored stream session unavailable"))?;
        let streams = session.runtime_streams;
        let acknowledged = streams.connect(scope, runtime_epoch, output).await?;
        *self.active.lock().await = Some(streams);
        Ok(acknowledged)
    }

    async fn disconnect(&self) -> Result<(), BrokerPortError> {
        if let Some(streams) = self.active.lock().await.take() {
            streams.disconnect().await?;
        }
        Ok(())
    }
}

fn execution_purpose(value: RuntimeExecutionPurpose) -> ExecutionPurpose {
    match value {
        RuntimeExecutionPurpose::SandboxMutation => ExecutionPurpose::SandboxMutation,
        RuntimeExecutionPurpose::ProductionManual => ExecutionPurpose::ProductionManual,
        RuntimeExecutionPurpose::ProductionAutomated => ExecutionPurpose::ProductionAutomated,
    }
}

fn factory_port_error(message: &str) -> BrokerPortError {
    BrokerPortError {
        service: "StoredTInvestClientFactory",
        method: "Resolve",
        class: BrokerResultClass::Credential,
        message: message.to_owned(),
        retry_after: None,
    }
}

fn authorization_port_error() -> BrokerPortError {
    BrokerPortError {
        service: "StoredTInvestClientFactory",
        method: "AuthorizeExecution",
        class: BrokerResultClass::Permission,
        message: "execution authorization denied for exact stored scope and purpose".to_owned(),
        retry_after: None,
    }
}

struct TInvestClientBuilder;

impl BrokerCredentialClientFactory for TInvestClientBuilder {
    type ReadClient = TInvestReadSession;
    type ExecutionClient = AuthorizedTInvestExecutionClient;

    fn build_read_client(
        &self,
        target: AccountTarget,
        credential: SecretBytes,
    ) -> Result<Self::ReadClient, ProviderError> {
        build_session(target, credential).map_err(factory_provider_error)
    }

    fn build_execution_client(
        &self,
        target: AccountTarget,
        credential: SecretBytes,
        authorization_revision: u64,
    ) -> Result<Self::ExecutionClient, ProviderError> {
        let target_copy = target.clone();
        let client = build_client(&target, credential).map_err(factory_provider_error)?;
        let route = match target.environment {
            BrokerEnvironment::Production => ExecutionRoute::Production,
            BrokerEnvironment::Sandbox => ExecutionRoute::Sandbox,
        };
        Ok(AuthorizedTInvestExecutionClient {
            target: target_copy,
            adapter: TInvestRuntimeExecutionAdapter::new(client, route, true),
            authorization_revision,
        })
    }
}

fn factory_provider_error(error: ConnectionFactoryError) -> ProviderError {
    let kind = match error {
        ConnectionFactoryError::WrongProvider => ProviderErrorKind::UnsupportedProvider,
        ConnectionFactoryError::InvalidCredentialEncoding | ConnectionFactoryError::Secret(_) => {
            ProviderErrorKind::InvalidCredential
        }
        ConnectionFactoryError::Grpc(_) | ConnectionFactoryError::Connection(_) => {
            ProviderErrorKind::ProviderUnavailable
        }
        ConnectionFactoryError::RuntimeExecution(_) => ProviderErrorKind::ProviderUnavailable,
    };
    ProviderError::new(kind, "stored T-Invest client construction failed")
}

fn build_session(
    target: AccountTarget,
    credential: SecretBytes,
) -> Result<TInvestReadSession, ConnectionFactoryError> {
    let client = build_client(&target, credential)?;
    let environment = target.environment;
    Ok(TInvestReadSession {
        target,
        client: AccountReadClient::new(client.clone()),
        runtime_reads: crate::runtime_read::TInvestRuntimeReadAdapter::new(
            client.clone(),
            environment,
        ),
        risk_reads: crate::risk_read::TInvestRiskReadAdapter::new(client.clone(), environment),
        runtime_streams: crate::runtime_stream::TInvestRuntimeStreamAdapter::new(client),
    })
}

fn build_client(
    target: &AccountTarget,
    credential: SecretBytes,
) -> Result<TInvestGrpcClient, ConnectionFactoryError> {
    if target.provider != ProviderId::tinvest() {
        return Err(ConnectionFactoryError::WrongProvider);
    }
    let text = std::str::from_utf8(credential.expose_secret())
        .map_err(|_| ConnectionFactoryError::InvalidCredentialEncoding)?;
    let token = SecretToken::new(text)?;
    let client = match target.environment {
        BrokerEnvironment::Production => {
            TInvestGrpcClient::production(GrpcCredential::Production(token))?
        }
        BrokerEnvironment::Sandbox => TInvestGrpcClient::sandbox(GrpcCredential::Sandbox(token))?,
    };
    Ok(client)
}

#[derive(Debug, Error)]
pub enum ConnectionFactoryError {
    #[error("stored connection resolution failed closed")]
    Connection(#[from] ServiceError),
    #[error("stored connection belongs to another provider")]
    WrongProvider,
    #[error("stored credential is not valid UTF-8 bearer material")]
    InvalidCredentialEncoding,
    #[error(transparent)]
    Secret(#[from] SecretTokenError),
    #[error(transparent)]
    Grpc(#[from] GrpcConfigError),
    #[error(transparent)]
    RuntimeExecution(#[from] RuntimeExecutionAdapterError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use vox_connections::{ConnectionId, VoxAccountId};

    #[tokio::test]
    async fn stored_sandbox_secret_builds_existing_client_without_secret_output()
    -> Result<(), Box<dyn std::error::Error>> {
        let target = AccountTarget {
            connection_id: ConnectionId::new(),
            provider: ProviderId::tinvest(),
            environment: BrokerEnvironment::Sandbox,
            provider_account_id: "broker-account".to_owned(),
            vox_account_id: VoxAccountId::new(),
        };
        let session = build_session(target.clone(), SecretBytes::new(b"token-value".to_vec())?)?;
        assert_eq!(session.target, target);
        assert_eq!(session.target.environment, BrokerEnvironment::Sandbox);
        Ok(())
    }

    #[test]
    fn non_utf8_secret_fails_before_client_construction() -> Result<(), Box<dyn std::error::Error>>
    {
        let target = AccountTarget {
            connection_id: ConnectionId::new(),
            provider: ProviderId::tinvest(),
            environment: BrokerEnvironment::Production,
            provider_account_id: "broker-account".to_owned(),
            vox_account_id: VoxAccountId::new(),
        };
        assert!(matches!(
            build_session(target, SecretBytes::new(vec![0xff])?),
            Err(ConnectionFactoryError::InvalidCredentialEncoding)
        ));
        Ok(())
    }
}
