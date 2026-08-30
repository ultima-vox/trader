//! #17-backed runtime credential gate. Every execution check re-reads durable authorization.

use async_trait::async_trait;
use std::sync::Arc;
use vox_connections::{
    BrokerEnvironment, BrokerProviderPort, ConnectionId, ConnectionRepository, ConnectionService,
    CredentialRef, ExecutionPurpose, ProviderId, SecretStore,
};

use crate::model::{Provider, RuntimeEnvironment, RuntimeScope};
use crate::ports::{
    BrokerPortError, BrokerResultClass, CredentialResolution, CredentialResolverPort,
    RuntimeExecutionPurpose,
};

pub struct StoredCredentialResolver<R, S, P> {
    connections: Arc<ConnectionService<R, S, P>>,
}

impl<R, S, P> StoredCredentialResolver<R, S, P> {
    #[must_use]
    pub const fn new(connections: Arc<ConnectionService<R, S, P>>) -> Self {
        Self { connections }
    }
}

#[async_trait]
impl<R, S, P> CredentialResolverPort for StoredCredentialResolver<R, S, P>
where
    R: ConnectionRepository,
    S: SecretStore,
    P: BrokerProviderPort,
{
    async fn resolve(&self, scope: &RuntimeScope) -> Result<CredentialResolution, BrokerPortError> {
        let (connection_id, expected_credential_ref) = parse_scope(scope)?;
        let resolved = self
            .connections
            .validate_runtime_read_access(&connection_id, &scope.broker_account_id)
            .map_err(|_| credential_error("connection read credential unavailable"))?;
        validate_scope(
            scope,
            &expected_credential_ref,
            &resolved.target,
            &resolved.credential_ref,
        )?;
        Ok(CredentialResolution {
            execution_authorized: self
                .authorize_execution(scope, default_execution_purpose(scope.environment))
                .await
                .is_ok(),
        })
    }

    async fn authorize_execution(
        &self,
        scope: &RuntimeScope,
        purpose: RuntimeExecutionPurpose,
    ) -> Result<(), BrokerPortError> {
        let (connection_id, expected_credential_ref) = parse_scope(scope)?;
        let resolved = self
            .connections
            .validate_runtime_execution_access(
                &connection_id,
                &scope.broker_account_id,
                execution_purpose(purpose),
                None,
            )
            .map_err(|_| authorization_error())?;
        validate_scope(
            scope,
            &expected_credential_ref,
            &resolved.target,
            &resolved.credential_ref,
        )?;
        Ok(())
    }
}

fn parse_scope(scope: &RuntimeScope) -> Result<(ConnectionId, CredentialRef), BrokerPortError> {
    let connection_id = ConnectionId::parse(scope.connection_ref.as_str())
        .map_err(|_| credential_error("invalid connection reference"))?;
    let credential_ref = CredentialRef::parse(scope.credential_ref.as_str())
        .map_err(|_| credential_error("invalid credential reference"))?;
    Ok((connection_id, credential_ref))
}

fn validate_scope(
    scope: &RuntimeScope,
    expected_credential_ref: &CredentialRef,
    target: &vox_connections::AccountTarget,
    actual_credential_ref: &CredentialRef,
) -> Result<(), BrokerPortError> {
    if actual_credential_ref != expected_credential_ref
        || target.provider != provider(scope.provider)
        || target.environment != environment(scope.environment)
        || target.provider_account_id != scope.broker_account_id
    {
        return Err(credential_error(
            "runtime scope does not match stored connection binding",
        ));
    }
    Ok(())
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

fn default_execution_purpose(environment: RuntimeEnvironment) -> RuntimeExecutionPurpose {
    match environment {
        RuntimeEnvironment::Sandbox => RuntimeExecutionPurpose::SandboxMutation,
        RuntimeEnvironment::Production => RuntimeExecutionPurpose::ProductionAutomated,
    }
}

fn execution_purpose(value: RuntimeExecutionPurpose) -> ExecutionPurpose {
    match value {
        RuntimeExecutionPurpose::SandboxMutation => ExecutionPurpose::SandboxMutation,
        RuntimeExecutionPurpose::ProductionManual => ExecutionPurpose::ProductionManual,
        RuntimeExecutionPurpose::ProductionAutomated => ExecutionPurpose::ProductionAutomated,
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

fn authorization_error() -> BrokerPortError {
    BrokerPortError {
        service: "ConnectionService",
        method: "AuthorizeExecution",
        class: BrokerResultClass::Permission,
        message: "execution authorization denied for exact connection/account/purpose".to_owned(),
        retry_after: None,
    }
}
