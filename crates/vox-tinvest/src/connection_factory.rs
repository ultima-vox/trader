//! Builds existing T-Invest gRPC client only after #17 resolves exact stored connection scope.

use std::sync::Arc;
use thiserror::Error;
use vox_connections::{
    AccountTarget, BrokerCredentialClientFactory, BrokerEnvironment, BrokerProviderPort,
    ConnectionId, ConnectionRepository, ConnectionService, ExecutionPurpose, ProviderError,
    ProviderErrorKind, ProviderId, SecretBytes, SecretStore, ServiceError,
};
use vox_domain::RuntimeExecutionCommand;

use crate::account::AccountReadClient;
use crate::execution_dispatch::ExecutionRoute;
use crate::runtime_execution::{
    RuntimeDispatchAcknowledgement, RuntimeExecutionAdapterError, TInvestRuntimeExecutionAdapter,
};
use crate::{GrpcConfigError, GrpcCredential, SecretToken, SecretTokenError, TInvestGrpcClient};

pub struct TInvestReadSession {
    pub target: AccountTarget,
    pub client: AccountReadClient,
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
    Ok(TInvestReadSession {
        target,
        client: AccountReadClient::new(client),
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
