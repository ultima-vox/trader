use async_trait::async_trait;
use vox_connections::{
    BrokerEnvironment, BrokerProviderPort, ConnectionId, ConnectionRepository, ConnectionService,
    CredentialRef, ExecutionAuthorizationMode, ProviderId, SecretStore, VoxAccountId,
};

use crate::model::{Provider, RuntimeEnvironment, RuntimeScope};
use crate::ports::{
    BrokerPortError, BrokerResultClass, CredentialResolution, CredentialResolverPort,
};

pub struct StoredCredentialResolver<R, S, P> {
    connections: ConnectionService<R, S, P>,
}

impl<R, S, P> StoredCredentialResolver<R, S, P> {
    #[must_use]
    pub const fn new(connections: ConnectionService<R, S, P>) -> Self {
        Self { connections }
    }

    #[must_use]
    pub fn into_inner(self) -> ConnectionService<R, S, P> {
        self.connections
    }
}

#[async_trait]
impl<R, S, P> CredentialResolverPort for StoredCredentialResolver<R, S, P>
where
    R: ConnectionRepository + Send + Sync,
    S: SecretStore + Send + Sync,
    P: BrokerProviderPort + Send + Sync,
{
    async fn resolve(&self, scope: &RuntimeScope) -> Result<CredentialResolution, BrokerPortError> {
        let connection_id = ConnectionId::parse(scope.connection_ref.as_str())
            .map_err(|_| credential_error("invalid connection reference"))?;
        let expected_credential_ref = CredentialRef::parse(scope.credential_ref.as_str())
            .map_err(|_| credential_error("invalid credential reference"))?;
        let vox_account_id = VoxAccountId::parse(scope.vox_account_id.clone())
            .map_err(|_| credential_error("invalid Vox account identity"))?;
        let resolved = self
            .connections
            .resolve_bound_connection(&connection_id, &scope.broker_account_id, &vox_account_id)
            .map_err(|_| credential_error("connection credential unavailable"))?;
        if resolved.credential_ref != expected_credential_ref
            || resolved.target.provider != provider(scope.provider)
            || resolved.target.environment != environment(scope.environment)
        {
            return Err(credential_error(
                "runtime scope does not match stored connection",
            ));
        }
        Ok(CredentialResolution {
            execution_authorized: resolved.execution_authorization
                == ExecutionAuthorizationMode::AutomatedAllowed,
        })
    }
}

fn provider(provider: Provider) -> ProviderId {
    match provider {
        Provider::TInvest => ProviderId::tinvest(),
    }
}

fn environment(environment: RuntimeEnvironment) -> BrokerEnvironment {
    match environment {
        RuntimeEnvironment::Sandbox => BrokerEnvironment::Sandbox,
        RuntimeEnvironment::Production => BrokerEnvironment::Production,
    }
}

fn credential_error(message: &str) -> BrokerPortError {
    BrokerPortError {
        service: "ConnectionService",
        method: "ResolveCredential",
        class: BrokerResultClass::Credential,
        message: message.to_owned(),
        retry_after: None,
    }
}
